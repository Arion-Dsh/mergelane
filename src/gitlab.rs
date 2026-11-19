//! GitLab.com and self-managed GitLab REST v4 issue adapter.

use anyhow::{Context as _, ensure};
use reqwest::{Url, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tracker::{
    Api, Comment, Http, Issue, Member, Search, Task, append, instance_path, project_path,
};

pub(crate) struct GitLab {
    http: Http,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    username: String,
}

#[derive(Deserialize)]
struct RemoteIssue {
    id: u64,
    iid: u64,
    title: String,
    description: Option<String>,
    state: String,
    web_url: String,
    #[serde(default)]
    assignees: Vec<User>,
}

impl GitLab {
    pub fn new(base: Url, token: String) -> anyhow::Result<Self> {
        let mut http = Http::token(
            "GitLab",
            base,
            token,
            header::HeaderName::from_static("private-token"),
            false,
        )?;
        http.api = append(http.web.clone(), &["api", "v4"]);
        Ok(Self { http })
    }

    fn reference(&self, input: &str) -> anyhow::Result<(String, u64)> {
        let (project, number) = if input.contains("://") {
            let path = instance_path(&self.http.web, input)?;
            let (project, number) = path
                .rsplit_once("/-/issues/")
                .or_else(|| path.rsplit_once("/-/work_items/"))
                .context("expected a GitLab issue URL")?;
            (project.to_owned(), number.to_owned())
        } else {
            let (project, number) = input
                .rsplit_once('#')
                .context("expected group/project#number or an issue URL")?;
            (project.to_owned(), number.to_owned())
        };
        let number = number
            .parse::<u64>()
            .context("invalid GitLab issue number")?;
        ensure!(number > 0, "issue number must be positive");
        Ok((self.project(&project)?, number))
    }

    fn decode(&self, value: Value) -> anyhow::Result<Issue> {
        let remote: RemoteIssue = serde_json::from_value(value)?;
        let (project, number) = self.reference(&remote.web_url)?;
        ensure!(
            number == remote.iid && remote.id > 0,
            "GitLab returned inconsistent issue identifiers"
        );
        let mut path: Vec<&str> = project.split('/').collect();
        let number_text = number.to_string();
        path.extend(["-", "issues", &number_text]);
        let url = append(self.http.web.clone(), &path).to_string();
        Ok(Issue {
            id: remote.id.to_string(),
            key: format!("{project}#{number}"),
            project,
            url,
            title: remote.title,
            description: remote.description.unwrap_or_default(),
            state: remote.state,
            assignees: remote
                .assignees
                .into_iter()
                .map(|user| Member {
                    id: user.id.to_string(),
                    name: user.username,
                })
                .collect(),
        })
    }
}

impl Api for GitLab {
    fn instance(&self) -> &Url {
        &self.http.web
    }

    async fn check_connection(&self) -> anyhow::Result<()> {
        let user: User = serde_json::from_value(self.http.get(&["user"], &[]).await?)?;
        ensure!(user.id > 0, "GitLab returned an invalid account ID");
        Ok(())
    }

    fn name(&self) -> &'static str {
        "gitlab"
    }
    fn project(&self, value: &str) -> anyhow::Result<String> {
        project_path(value, true)
    }

    async fn search(&self, input: Search) -> anyhow::Result<Value> {
        ensure!(
            input.cursor.is_none(),
            "GitLab uses page rather than cursor"
        );
        ensure!(
            !(input.mine && input.assignee.is_some()),
            "choose mine or assignee"
        );
        ensure!(
            input.project.is_some() || input.mine,
            "provide a GitLab project path, or use mine for global assigned issues"
        );
        let mut query = vec![
            (
                "state",
                if input.open_only { "opened" } else { "all" }.to_owned(),
            ),
            (
                "scope",
                if input.mine { "assigned_to_me" } else { "all" }.to_owned(),
            ),
            ("page", input.page.to_string()),
            ("per_page", "30".to_owned()),
        ];
        if let Some(text) = input.query {
            query.push(("search", text));
        }
        if let Some(assignee) = input.assignee {
            if assignee.parse::<u64>().is_ok_and(|id| id > 0) {
                query.push(("assignee_id", assignee));
            } else {
                query.push(("assignee_username[]", assignee));
            }
        }
        let path;
        let project;
        if let Some(input) = input.project {
            project = self.project(&input)?;
            path = vec!["projects", project.as_str(), "issues"];
        } else {
            path = vec!["issues"];
        }
        let value = self.http.get(&path, &query).await?;
        let batch = value
            .as_array()
            .context("GitLab search returned no array")?;
        let items = batch
            .iter()
            .cloned()
            .map(|value| self.decode(value))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(json!({"items":items,"next_page":(batch.len() == 30).then_some(input.page + 1)}))
    }

    async fn issue(&self, reference: &str) -> anyhow::Result<Issue> {
        let (project, number) = self.reference(reference)?;
        let issue = self.decode(
            self.http
                .get(&["projects", &project, "issues", &number.to_string()], &[])
                .await?,
        )?;
        ensure!(
            issue.project == project && issue.key == format!("{project}#{number}"),
            "GitLab returned a different issue"
        );
        Ok(issue)
    }

    async fn members(&self, project: &str) -> anyhow::Result<Vec<Member>> {
        let project = self.project(project)?;
        self.http
            .list(&["projects", &project, "members", "all"], &[])
            .await?
            .into_iter()
            .map(|value| {
                let user: User = serde_json::from_value(value)?;
                Ok(Member {
                    id: user.id.to_string(),
                    name: user.username,
                })
            })
            .collect()
    }

    async fn comments(&self, issue: &Issue) -> anyhow::Result<Vec<Comment>> {
        let (project, number) = self.reference(&issue.key)?;
        self.http
            .list(
                &["projects", &project, "issues", &number.to_string(), "notes"],
                &[],
            )
            .await?
            .into_iter()
            .filter(|value| value["system"] != true)
            .map(|value| {
                Ok(Comment {
                    body: value["body"]
                        .as_str()
                        .context("note has no body")?
                        .to_owned(),
                    url: None,
                })
            })
            .collect()
    }

    async fn blockers(&self, issue: &Issue) -> anyhow::Result<Vec<String>> {
        let (project, number) = self.reference(&issue.key)?;
        self.http
            .list(
                &["projects", &project, "issues", &number.to_string(), "links"],
                &[],
            )
            .await?
            .into_iter()
            .filter(|value| value["link_type"] == "is_blocked_by")
            .map(|value| {
                let (project, number) = self.reference(
                    value["web_url"]
                        .as_str()
                        .context("linked issue has no URL")?,
                )?;
                Ok(format!("{project}#{number}"))
            })
            .collect()
    }

    async fn find_task(&self, project: &str, marker: &str) -> anyhow::Result<Option<Issue>> {
        let values = self
            .http
            .list(
                &["projects", project, "issues"],
                &[("state", "all".to_owned()), ("scope", "all".to_owned())],
            )
            .await?;
        let mut found = None;
        for value in values.into_iter().filter(|value| {
            value["description"]
                .as_str()
                .is_some_and(|body| body.lines().next() == Some(marker))
        }) {
            ensure!(
                found.is_none(),
                "duplicate GitLab tasks match this plan; resolve them remotely"
            );
            found = Some(self.decode(value)?);
        }
        Ok(found)
    }

    async fn create(&self, task: &Task, body: &str, _marker: &str) -> anyhow::Result<Issue> {
        let id = member_id(&task.assignee)?;
        self.decode(
            self.http
                .post(
                    &["projects", task.project()?, "issues"],
                    &json!({"title":task.title,"description":body,"assignee_ids":[id]}),
                )
                .await?,
        )
    }

    async fn create_unassigned(
        &self,
        project: &str,
        input: &crate::issue::Create,
    ) -> anyhow::Result<Issue> {
        let mut body = json!({"title": input.title});
        if let Some(description) = &input.description {
            body["description"] = json!(description);
        }
        let value = self
            .http
            .post(&["projects", project, "issues"], &body)
            .await?;
        self.decode(value)
    }

    async fn assign(&self, issue: &Issue, assignee: &str) -> anyhow::Result<Issue> {
        let id = member_id(assignee)?;
        ensure!(
            id.to_string() == assignee,
            "use the numeric member ID from issue_context"
        );
        let (project, number) = self.reference(&issue.key)?;
        let value = self
            .http
            .put(
                &["projects", &project, "issues", &number.to_string()],
                &json!({"assignee_ids":[id]}),
            )
            .await?;
        self.decode(value)
    }

    async fn edit(&self, issue: &Issue, input: &crate::issue::Edit) -> anyhow::Result<Issue> {
        let (project, number) = self.reference(&issue.key)?;
        let body = input.fields("title", "description", |text| json!(text));
        let value = self
            .http
            .put(
                &["projects", &project, "issues", &number.to_string()],
                &body,
            )
            .await?;
        self.decode(value)
    }

    async fn block(&self, issue: &Issue, blocker: &Issue) -> anyhow::Result<()> {
        let (project, number) = self.reference(&issue.key)?;
        let (target, target_number) = self.reference(&blocker.key)?;
        self.http.post(&["projects", &project, "issues", &number.to_string(), "links"],
            &json!({"target_project_id":target,"target_issue_iid":target_number,"link_type":"is_blocked_by"})).await?;
        Ok(())
    }

    async fn comment(&self, issue: &Issue, body: &str) -> anyhow::Result<()> {
        let (project, number) = self.reference(&issue.key)?;
        self.http
            .post(
                &["projects", &project, "issues", &number.to_string(), "notes"],
                &json!({"body":body}),
            )
            .await?;
        Ok(())
    }
}

fn member_id(assignee: &str) -> anyhow::Result<u64> {
    let id = assignee
        .parse::<u64>()
        .context("GitLab assignee must be a numeric member ID")?;
    ensure!(id > 0, "GitLab assignee must be positive");
    Ok(id)
}
