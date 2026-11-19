//! GitHub.com and GitHub Enterprise Server REST issue adapter.

use anyhow::{Context as _, ensure};
use reqwest::{Url, header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tracker::{
    Api, Comment, Http, Issue, Member, Search, Task, append, instance_path, project_path,
};

pub(crate) struct GitHub {
    http: Http,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct RemoteIssue {
    id: u64,
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    #[serde(default)]
    assignees: Vec<User>,
    pull_request: Option<Value>,
}

impl GitHub {
    pub fn new(base: Url, token: String) -> anyhow::Result<Self> {
        let mut http = Http::token("GitHub", base, token, header::AUTHORIZATION, true)?;
        http.api = if http.web.as_str() == "https://github.com/" {
            Url::parse("https://api.github.com/").expect("static API URL")
        } else {
            append(http.web.clone(), &["api", "v3"])
        };
        Ok(Self { http })
    }

    fn reference(&self, input: &str) -> anyhow::Result<(String, u64)> {
        let (project, number) = if input.contains("://") {
            let path = instance_path(&self.http.web, input)?;
            let (project, number) = path
                .rsplit_once("/issues/")
                .context("expected a GitHub issue URL")?;
            (project.to_owned(), number.to_owned())
        } else {
            let (project, number) = input
                .rsplit_once('#')
                .context("expected owner/repo#number or an issue URL")?;
            (project.to_owned(), number.to_owned())
        };
        let number = number
            .parse::<u64>()
            .context("invalid GitHub issue number")?;
        ensure!(number > 0, "issue number must be positive");
        Ok((self.project(&project)?, number))
    }

    fn path<'a>(&self, project: &'a str) -> anyhow::Result<Vec<&'a str>> {
        let (owner, repo) = project.split_once('/').context("expected owner/repo")?;
        Ok(vec!["repos", owner, repo, "issues"])
    }

    fn decode(&self, project: &str, value: Value) -> anyhow::Result<Issue> {
        let remote: RemoteIssue = serde_json::from_value(value)?;
        ensure!(
            remote.pull_request.is_none() && remote.id > 0 && remote.number > 0,
            "expected a GitHub issue"
        );
        let (owner, repo) = project.split_once('/').context("expected owner/repo")?;
        Ok(Issue {
            id: remote.id.to_string(),
            key: format!("{project}#{}", remote.number),
            project: project.to_owned(),
            url: append(
                self.http.web.clone(),
                &[owner, repo, "issues", &remote.number.to_string()],
            )
            .to_string(),
            title: remote.title,
            description: remote.body.unwrap_or_default(),
            state: remote.state,
            assignees: remote
                .assignees
                .into_iter()
                .map(|user| Member {
                    id: user.login.clone(),
                    name: user.login,
                })
                .collect(),
        })
    }
}

impl Api for GitHub {
    fn instance(&self) -> &Url {
        &self.http.web
    }

    async fn check_connection(&self) -> anyhow::Result<()> {
        let user: User = serde_json::from_value(self.http.get(&["user"], &[]).await?)?;
        ensure!(
            !user.login.is_empty(),
            "GitHub returned an empty account login"
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "github"
    }
    fn project(&self, value: &str) -> anyhow::Result<String> {
        Ok(project_path(value, false)?.to_ascii_lowercase())
    }

    async fn search(&self, input: Search) -> anyhow::Result<Value> {
        ensure!(
            input.cursor.is_none(),
            "GitHub uses page rather than cursor"
        );
        ensure!(
            !(input.mine && input.assignee.is_some()),
            "choose mine or assignee"
        );
        let mut query = "is:issue".to_owned();
        if input.open_only {
            query.push_str(" is:open");
        }
        if let Some(project) = input.project {
            query.push_str(&format!(" repo:{}", self.project(&project)?));
        }
        let assignee = if input.mine {
            let user: User = serde_json::from_value(self.http.get(&["user"], &[]).await?)?;
            Some(user.login)
        } else {
            input.assignee
        };
        if let Some(name) = assignee {
            ensure!(
                name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
                "invalid GitHub assignee login"
            );
            query.push_str(&format!(" assignee:{name}"));
        }
        if let Some(text) = input.query {
            query.push(' ');
            query.push_str(&text);
        }
        ensure!(
            input.page <= 34,
            "GitHub search exposes at most 1000 results; narrow the query"
        );
        let value = self
            .http
            .get(
                &["search", "issues"],
                &[
                    ("q", query),
                    ("page", input.page.to_string()),
                    ("per_page", "30".to_owned()),
                ],
            )
            .await?;
        let batch = value["items"]
            .as_array()
            .context("GitHub search returned no items array")?;
        let mut items = Vec::new();
        for item in batch {
            let url = item["html_url"]
                .as_str()
                .context("GitHub result has no URL")?;
            let (project, _) = self.reference(url)?;
            items.push(self.decode(&project, item.clone())?);
        }
        let total = value["total_count"]
            .as_u64()
            .context("GitHub search returned no count")?;
        let next = (u64::from(input.page) * 30 < total.min(1000)).then_some(input.page + 1);
        Ok(
            json!({"items":items,"next_page":next,"truncated":total > 1000 || value["incomplete_results"] == true}),
        )
    }

    async fn issue(&self, reference: &str) -> anyhow::Result<Issue> {
        let (project, number) = self.reference(reference)?;
        let number_text = number.to_string();
        let mut path = self.path(&project)?;
        path.push(&number_text);
        let issue = self.decode(&project, self.http.get(&path, &[]).await?)?;
        ensure!(
            issue.key == format!("{project}#{number}"),
            "GitHub returned a different issue"
        );
        Ok(issue)
    }

    async fn members(&self, project: &str) -> anyhow::Result<Vec<Member>> {
        let project = self.project(project)?;
        let (owner, repo) = project.split_once('/').context("expected owner/repo")?;
        self.http
            .list(&["repos", owner, repo, "assignees"], &[])
            .await?
            .into_iter()
            .map(|value| {
                let user: User = serde_json::from_value(value)?;
                Ok(Member {
                    id: user.login.clone(),
                    name: user.login,
                })
            })
            .collect()
    }

    async fn comments(&self, issue: &Issue) -> anyhow::Result<Vec<Comment>> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.extend([number.as_str(), "comments"]);
        self.http
            .list(&path, &[])
            .await?
            .into_iter()
            .map(|value| {
                Ok(Comment {
                    body: value["body"]
                        .as_str()
                        .context("comment has no body")?
                        .to_owned(),
                    url: value["html_url"].as_str().map(str::to_owned),
                })
            })
            .collect()
    }

    async fn blockers(&self, issue: &Issue) -> anyhow::Result<Vec<String>> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.extend([number.as_str(), "dependencies", "blocked_by"]);
        self.http
            .list(&path, &[])
            .await?
            .into_iter()
            .map(|value| {
                let (project, number) = self.reference(
                    value["html_url"]
                        .as_str()
                        .context("dependency has no URL")?,
                )?;
                Ok(format!("{project}#{number}"))
            })
            .collect()
    }

    async fn find_task(&self, project: &str, marker: &str) -> anyhow::Result<Option<Issue>> {
        let values = self
            .http
            .list(
                &self.path(project)?,
                &[
                    ("state", "all".to_owned()),
                    ("sort", "created".to_owned()),
                    ("direction", "desc".to_owned()),
                ],
            )
            .await?;
        let mut found = None;
        for value in values.into_iter().filter(|value| {
            value["pull_request"].is_null()
                && value["body"]
                    .as_str()
                    .is_some_and(|body| body.lines().next() == Some(marker))
        }) {
            ensure!(
                found.is_none(),
                "duplicate GitHub tasks match this plan; resolve them remotely"
            );
            found = Some(self.decode(project, value)?);
        }
        Ok(found)
    }

    async fn create(&self, task: &Task, body: &str, _marker: &str) -> anyhow::Result<Issue> {
        self.decode(
            task.project()?,
            self.http
                .post(
                    &self.path(task.project()?)?,
                    &json!({"title":task.title,"body":body,"assignees":[task.assignee]}),
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
            body["body"] = json!(description);
        }
        self.decode(project, self.http.post(&self.path(project)?, &body).await?)
    }

    async fn assign(&self, issue: &Issue, assignee: &str) -> anyhow::Result<Issue> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.extend([number.as_str(), "assignees"]);
        let value = self
            .http
            .post(&path, &json!({"assignees":[assignee]}))
            .await?;
        self.decode(&issue.project, value)
    }

    async fn edit(&self, issue: &Issue, input: &crate::issue::Edit) -> anyhow::Result<Issue> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.push(&number);
        let body = input.fields("title", "body", |text| json!(text));
        self.decode(&issue.project, self.http.patch(&path, &body).await?)
    }

    async fn block(&self, issue: &Issue, blocker: &Issue) -> anyhow::Result<()> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.extend([number.as_str(), "dependencies", "blocked_by"]);
        self.http
            .post(&path, &json!({"issue_id":blocker.id.parse::<u64>()?}))
            .await?;
        Ok(())
    }

    async fn comment(&self, issue: &Issue, body: &str) -> anyhow::Result<()> {
        let (_, number) = self.reference(&issue.key)?;
        let number = number.to_string();
        let mut path = self.path(&issue.project)?;
        path.extend([number.as_str(), "comments"]);
        self.http.post(&path, &json!({"body":body})).await?;
        Ok(())
    }
}
