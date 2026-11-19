use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Member, Plane, WorkItem, WorkItemRef, references::valid_slug};
use crate::validation::simple_text;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    query: Option<String>,
    workspace: Option<String>,
    project: Option<String>,
    assignee: Option<String>,
    #[serde(default)]
    mine: bool,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(skip)]
    open_only: bool,
}

fn default_limit() -> usize {
    20
}

#[derive(Deserialize)]
pub(super) struct Project {
    pub id: Uuid,
    name: String,
    pub identifier: String,
    pub default_assignee: Option<Uuid>,
}

#[derive(Deserialize)]
struct SearchHit {
    id: Uuid,
    project_id: Uuid,
}

#[derive(Deserialize, Serialize)]
pub(super) struct WorkflowState {
    pub id: Uuid,
    pub name: String,
    pub group: String,
}

impl Plane {
    pub(super) async fn resolve_project(
        &self,
        workspace: &str,
        selector: &str,
    ) -> anyhow::Result<Project> {
        let projects = self
            .list(
                self.workspace_api(workspace, &["projects"]),
                "list workspace projects",
            )
            .await?;
        select_projects(projects, Some(selector))?
            .pop()
            .context("no matching Plane project")
    }

    pub(crate) async fn pull_open(&self, project: Option<String>) -> anyhow::Result<Value> {
        self.search(Search {
            query: None,
            workspace: None,
            assignee: None,
            mine: project.is_none(),
            project,
            limit: 50,
            open_only: true,
        })
        .await
    }

    pub async fn search(&self, input: Search) -> anyhow::Result<Value> {
        let workspace = input.workspace.as_ref().or(self.default_workspace.as_ref())
            .context("set --plane-workspace once in the MCP configuration, or provide the workspace slug for this search")?;
        ensure!(valid_slug(workspace), "invalid Plane workspace slug");
        ensure!(
            (1..=50).contains(&input.limit),
            "limit must be between 1 and 50"
        );
        for text in [&input.query, &input.project, &input.assignee]
            .into_iter()
            .flatten()
        {
            ensure!(
                simple_text(text, 250),
                "search fields must contain 1 to 250 single-line bytes"
            );
        }
        ensure!(
            !(input.mine && input.assignee.is_some()),
            "choose mine or assignee"
        );
        let projects: Vec<Project> = self
            .list(
                self.workspace_api(workspace, &["projects"]),
                "list workspace projects",
            )
            .await?;
        let projects = select_projects(projects, input.project.as_deref())?;
        let me = if input.mine {
            Some(self.current_user().await?)
        } else {
            None
        };
        let mut items = Vec::new();
        let mut errors = Vec::new();
        let mut truncated = false;
        for project in projects {
            match self.search_project(workspace, &project, &input, me).await {
                Ok((mut matches, limited)) => {
                    truncated |= limited;
                    items.append(&mut matches);
                }
                Err(error) => {
                    errors.push(json!({"project":project.name,"error":error.to_string()}))
                }
            }
        }
        truncated |= items.len() > input.limit;
        items.truncate(input.limit);
        Ok(
            json!({"items":items,"truncated":truncated,"project_errors":errors,
            "notice":"These are work items visible to the authenticated account. Read their URLs with plane_issue_context for checkpoints and blockers. Ask which task to use when titles are ambiguous. Narrow the project or query when results are truncated."}),
        )
    }

    pub(super) async fn current_user(&self) -> anyhow::Result<Uuid> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("validated HTTP URL")
            .pop_if_empty()
            .extend(["api", "v1", "users", "me", ""]);
        #[derive(Deserialize)]
        struct Identity {
            id: Uuid,
        }
        let identity: Identity = self.get(url, "read current Plane account").await?;
        Ok(identity.id)
    }

    async fn search_project(
        &self,
        workspace: &str,
        project: &Project,
        input: &Search,
        me: Option<Uuid>,
    ) -> anyhow::Result<(Vec<Value>, bool)> {
        let (items, truncated) = self
            .project_candidates(workspace, project.id, input.query.as_deref())
            .await?;
        let members = self.members(workspace, project.id).await?;
        let assignee = if let Some(name) = &input.assignee {
            let Some(member) = resolve_member(&members, name)? else {
                return Ok((Vec::new(), truncated));
            };
            Some(member)
        } else {
            me
        };
        let states = self.states(workspace, project.id).await?;
        let mut matches = Vec::new();
        for item in items {
            if assignee.is_some_and(|id| !item.assignees.contains(&id)) {
                continue;
            }
            let state = states.iter().find(|state| Some(state.id) == item.state);
            if input.open_only
                && state
                    .is_some_and(|state| matches!(state.group.as_str(), "completed" | "cancelled"))
            {
                continue;
            }
            matches.push(json!({
                "title":item.name,"url":self.browse_link(workspace, &project.identifier, item.sequence_id),"project":project.name,
                "project_identifier":project.identifier,"state_id":item.state,
                "state":state.map(|state| &state.name),"state_group":state.map(|state| &state.group),
                "assignees":item.assignees.iter().map(|id| json!({
                    "id":id,"name":members.iter().find(|member| member.id == *id).map(|member| &member.display_name)
                })).collect::<Vec<_>>()
            }));
        }
        Ok((matches, truncated))
    }

    async fn project_candidates(
        &self,
        workspace: &str,
        project: Uuid,
        query: Option<&str>,
    ) -> anyhow::Result<(Vec<WorkItem>, bool)> {
        let Some(query) = query else {
            return Ok((
                self.list(
                    self.project_api(workspace, project, &["work-items"]),
                    "list project work items",
                )
                .await?,
                false,
            ));
        };
        let mut url = self.workspace_api(workspace, &["work-items", "search"]);
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("workspace_search", "false")
            .append_pair("project_id", &project.to_string())
            .append_pair("limit", "51");
        #[derive(Deserialize)]
        struct Response {
            issues: Vec<SearchHit>,
        }
        let response: Response = self.get(url, "search project work items").await?;
        let truncated = response.issues.len() >= 51;
        let mut items = Vec::new();
        for hit in response.issues.into_iter().take(50) {
            ensure!(
                hit.project_id == project,
                "Plane search returned a different project"
            );
            items.push(
                self.item(&WorkItemRef {
                    workspace: workspace.to_owned(),
                    project,
                    item: hit.id,
                })
                .await?,
            );
        }
        Ok((items, truncated))
    }

    pub(super) async fn states(
        &self,
        workspace: &str,
        project: Uuid,
    ) -> anyhow::Result<Vec<WorkflowState>> {
        self.list(
            self.project_api(workspace, project, &["states"]),
            "read project states",
        )
        .await
    }
}

fn select_projects(projects: Vec<Project>, selector: Option<&str>) -> anyhow::Result<Vec<Project>> {
    let projects: Vec<_> = projects
        .into_iter()
        .filter(|project| {
            selector.is_none_or(|name| {
                name.eq_ignore_ascii_case(&project.name)
                    || name.eq_ignore_ascii_case(&project.identifier)
                    || name.eq_ignore_ascii_case(&project.id.to_string())
            })
        })
        .collect();
    ensure!(
        !projects.is_empty(),
        "no visible Plane project matches the search"
    );
    ensure!(
        selector.is_none() || projects.len() == 1,
        "several Plane projects have this name; choose a project identifier"
    );
    ensure!(
        projects.len() <= 20,
        "more than 20 projects are visible; narrow the search to one project"
    );
    Ok(projects)
}

fn resolve_member(members: &[Member], name: &str) -> anyhow::Result<Option<Uuid>> {
    let mut matches = members.iter().filter(|member| {
        name.eq_ignore_ascii_case(&member.display_name)
            || name.eq_ignore_ascii_case(&member.id.to_string())
    });
    let Some(member) = matches.next() else {
        return Ok(None);
    };
    ensure!(
        matches.next().is_none(),
        "several members have this name; choose the member from plane_issue_context"
    );
    Ok(Some(member.id))
}
