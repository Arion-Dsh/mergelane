use anyhow::{Context as _, ensure};
use reqwest::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    ApiError, Comment, Plane, SOURCE, WorkItem, WorkItemRef, plan::Plan, references::html,
};
use crate::issue;

impl Plane {
    pub(super) async fn create_issue(
        &self,
        project: &str,
        input: &issue::Create,
    ) -> anyhow::Result<Value> {
        let workspace = self.default_workspace.as_deref().context(
            "configure a Plane workspace with --plane-workspace before creating an issue",
        )?;
        let project = self.resolve_project(workspace, project).await?;
        ensure!(
            project.default_assignee.is_none(),
            "Plane project {} has a default assignee; remove the project default to create an unassigned issue",
            project.identifier
        );
        let mut body = json!({"name":input.title,"assignees":[]});
        if let Some(description) = &input.description {
            body["description_html"] = json!(format!("<p>{}</p>", html(description)));
        }
        let item: WorkItem = self
            .post(
                self.project_api(workspace, project.id, &["work-items"]),
                &body,
                "create unassigned work item",
            )
            .await
            .with_context(|| {
                format!(
                    "could not confirm creation in {}; check the project before retrying",
                    project.identifier
                )
            })?;
        ensure!(
            item.project == project.id && item.sequence_id > 0,
            "Plane returned a different work item"
        );
        let url = self.browse_link(workspace, &project.identifier, item.sequence_id);
        ensure!(
            item.assignees.is_empty(),
            "Plane created {url} with an assignee; check the project's assignment rules"
        );
        Ok(json!({"url":url,"issue":item}))
    }

    pub(super) async fn assign_issue(&self, input: issue::Assign) -> anyhow::Result<Value> {
        let assignee = Uuid::parse_str(&input.assignee).context("provide a Plane member UUID")?;
        let reference = self.reference(&input.issue).await?;
        let issue = self.item(&reference).await?;
        let url = self.item_link(&reference, &issue).await?;
        if issue.assignees.contains(&assignee) {
            return Ok(json!({"url":url,"issue":issue}));
        }
        ensure!(issue.assignees.is_empty(), "{url} already has an assignee");
        ensure!(
            self.members(&reference.workspace, reference.project)
                .await?
                .iter()
                .any(|member| member.id == assignee),
            "the specified user is not a member of the Plane project"
        );
        let request = self
            .http
            .patch(self.item_api(&reference, &[]))
            .json(&json!({"assignees":[assignee]}));
        let updated: WorkItem = self.request(request, "assign work item").await?;
        ensure!(
            updated.id == reference.item
                && updated.project == reference.project
                && updated.assignees.contains(&assignee),
            "Plane did not confirm the requested assignee on {url}"
        );
        Ok(json!({"url":url,"issue":updated}))
    }

    pub(super) async fn edit_issue(&self, input: issue::Edit) -> anyhow::Result<Value> {
        let reference = self.reference(&input.issue).await?;
        let issue = self.item(&reference).await?;
        let url = self.item_link(&reference, &issue).await?;
        let body = input.fields("name", "description_html", |text| {
            json!(format!("<p>{}</p>", html(text)))
        });
        let request = self.http.patch(self.item_api(&reference, &[])).json(&body);
        let updated: WorkItem = self
            .request(request, "edit work item")
            .await
            .with_context(|| format!("could not confirm edit of {url}; check the work item"))?;
        ensure!(
            updated.id == reference.item
                && updated.project == reference.project
                && updated.sequence_id == issue.sequence_id
                && input
                    .title
                    .as_ref()
                    .is_none_or(|title| title == &updated.name),
            "Plane did not confirm the requested edit on {url}"
        );
        Ok(json!({"url":url,"issue":updated}))
    }

    pub async fn apply(&self, plan: &Plan) -> anyhow::Result<Value> {
        self.item(&plan.reference).await?;
        // Check membership again before writing, including after a session has been idle.
        for task in &plan.tasks {
            let members = self
                .members(
                    &plan.reference.workspace,
                    task.project.context("missing project")?,
                )
                .await?;
            ensure!(
                members.iter().any(|member| member.id == task.assignee),
                "planned assignee is no longer a project member"
            );
        }
        let mut created = Vec::with_capacity(plan.tasks.len());
        let mut links = Vec::with_capacity(plan.tasks.len());
        for (index, task) in plan.tasks.iter().enumerate() {
            let project = task.project.context("missing project")?;
            let external = format!("{}:{}", plan.id, index);
            let existing = self
                .find_item(&plan.reference.workspace, project, &external)
                .await?;
            let item = match existing {
                Some(item) => item,
                None => {
                    let mut body = json!({
                        "name":task.title,"description_html":task.description_html(&plan.parent),
                        "assignees":[task.assignee],"external_source":SOURCE,"external_id":external
                    });
                    if project == plan.reference.project {
                        body["parent"] = json!(plan.reference.item);
                    }
                    let result = self
                        .post::<WorkItem>(
                            self.project_api(&plan.reference.workspace, project, &["work-items"]),
                            &body,
                            "create work item",
                        )
                        .await;
                    match result {
                        Ok(item) => item,
                        Err(error) if status(&error) == Some(StatusCode::CONFLICT) => {
                            self.find_item(&plan.reference.workspace, project, &external).await?
                                .context("Plane reported a duplicate that is not yet readable; retry the same plan")?
                        }
                        Err(error) => return Err(error.context(format!(
                            "task {} failed after {} earlier tasks; retry the same preview to resume", index + 1, created.len()
                        ))),
                    }
                }
            };
            ensure!(
                item.project == project
                    && item.external_id.as_deref() == Some(&external)
                    && item.external_source.as_deref() == Some(SOURCE),
                "Plane returned a different plan task"
            );
            ensure!(
                item.assignees.contains(&task.assignee),
                "created task {} is missing the planned assignee; check it in Plane",
                index + 1
            );
            let reference = WorkItemRef {
                workspace: plan.reference.workspace.clone(),
                project,
                item: item.id,
            };
            links.push(self.item_link(&reference, &item).await?);
            created.push(reference);
        }
        for (task, item) in plan.tasks.iter().zip(&created) {
            if task.after.is_empty() {
                continue;
            }
            let relations = self
                .relations(item)
                .await
                .context("tasks exist; retry the same preview to resume dependency linking")?;
            let existing = relations["blocked_by"]
                .as_array()
                .context("Plane relation response has no blocked_by list")?;
            let missing: Vec<_> = task
                .after
                .iter()
                .filter_map(|&index| {
                    let blocker = &created[index];
                    (!existing
                        .iter()
                        .any(|value| value["issue_id"] == blocker.item.to_string()))
                    .then_some(blocker.item)
                })
                .collect();
            if !missing.is_empty() {
                self.post::<Value>(
                    self.item_api(item, &["relations"]),
                    &json!({"relation_type":"blocked_by","issues":missing}),
                    "link blocking work items",
                )
                .await
                .context("tasks exist; retry the same preview to resume dependency linking")?;
            }
        }
        let mut summary = String::from("<h3>Tasks</h3><ul>");
        for (task, link) in plan.tasks.iter().zip(&links) {
            summary.push_str(&format!(
                "<li>{} — {} — {}</li>",
                html(&task.title),
                task.assignee,
                html(link)
            ));
        }
        summary.push_str("</ul>");
        self.publish_comment(&plan.reference, &format!("plan:{}", plan.id), summary)
            .await
            .context("tasks exist; retry the same preview to publish the summary")?;
        Ok(json!({"parent":plan.parent,"issues":links}))
    }

    async fn find_item(
        &self,
        workspace: &str,
        project: uuid::Uuid,
        external: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        let mut url = self.project_api(workspace, project, &["work-items"]);
        url.query_pairs_mut()
            .append_pair("external_source", SOURCE)
            .append_pair("external_id", external);
        let value: Value = match self
            .get(url.clone(), "find previously created work item")
            .await
        {
            Ok(value) => value,
            Err(error) if status(&error) == Some(StatusCode::NOT_FOUND) => return Ok(None),
            Err(error) => return Err(error),
        };
        if value.get("id").is_some() {
            let item: WorkItem = serde_json::from_value(value)?;
            ensure!(
                item.external_source.as_deref() == Some(SOURCE)
                    && item.external_id.as_deref() == Some(external),
                "Plane returned a work item with a different external ID"
            );
            return Ok(Some(item));
        }
        let items: Vec<WorkItem> = self.list(url, "find previously created work item").await?;
        let mut matches = items.into_iter().filter(|item| {
            item.external_source.as_deref() == Some(SOURCE)
                && item.external_id.as_deref() == Some(external)
        });
        let found = matches.next();
        ensure!(
            matches.next().is_none(),
            "duplicate work items for this plan step; resolve them in Plane"
        );
        Ok(found)
    }

    pub(super) async fn publish_comment(
        &self,
        item: &WorkItemRef,
        external: &str,
        body: String,
    ) -> anyhow::Result<Comment> {
        let comments = self.comments(item).await?;
        if let Some(comment) = comments.into_iter().find(|comment| {
            comment.external_source.as_deref() == Some(SOURCE)
                && comment.external_id.as_deref() == Some(external)
        }) {
            return Ok(comment);
        }
        let result = self.post(self.item_api(item, &["comments"]), &json!({
            "comment_html":body,"external_source":SOURCE,"external_id":external,"access":"INTERNAL"
        }), "publish work item comment").await;
        match result {
            Err(error) if status(&error) == Some(StatusCode::CONFLICT) => self
                .comments(item)
                .await?
                .into_iter()
                .find(|comment| {
                    comment.external_source.as_deref() == Some(SOURCE)
                        && comment.external_id.as_deref() == Some(external)
                })
                .context("duplicate comment is not yet readable; retry"),
            other => other,
        }
    }
}

fn status(error: &anyhow::Error) -> Option<StatusCode> {
    error.downcast_ref::<ApiError>().map(|error| error.status)
}
