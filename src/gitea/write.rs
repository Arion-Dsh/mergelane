use anyhow::{Context as _, ensure};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::plan::{Plan, Task};
use super::{CHECKPOINT_MARKER, Comment, Gitea, Issue, IssueRef, User, append, repository};
use crate::{issue, validation::simple_text};

impl Gitea {
    pub(super) async fn create_issue(
        &self,
        project: &str,
        input: &issue::Create,
    ) -> anyhow::Result<Value> {
        let (owner, repo) = repository(project)?;
        let mut body = json!({"title":input.title});
        if let Some(description) = &input.description {
            body["body"] = json!(description);
        }
        let mut issue: Issue = self
            .post(
                self.repo_api(owner, repo, &["issues"]),
                &body,
                "create unassigned issue",
            )
            .await
            .with_context(|| {
                format!(
                    "could not confirm creation in {project}; check the repository before retrying"
                )
            })?;
        ensure!(
            issue.number > 0 && issue.pull_request.is_none(),
            "expected a created Gitea issue"
        );
        let reference = IssueRef {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            number: issue.number,
        };
        issue.html_url = self.issue_link(&reference);
        ensure!(
            issue.assignees.as_deref().unwrap_or_default().is_empty(),
            "Gitea created {} with an assignee; check the repository's assignment rules",
            issue.html_url
        );
        Ok(json!({"url":issue.html_url,"issue":issue}))
    }

    pub(super) async fn assign_issue(&self, input: issue::Assign) -> anyhow::Result<Value> {
        let reference = self.issue_ref(&input.issue)?;
        let issue = self.read_issue(&reference).await?;
        let matches = |user: &User| user.login.eq_ignore_ascii_case(&input.assignee);
        let assignees = issue.assignees.as_deref().unwrap_or_default();
        if assignees.iter().any(matches) {
            return Ok(json!({"url":issue.html_url,"issue":issue}));
        }
        ensure!(
            assignees.is_empty(),
            "{} already has an assignee",
            issue.html_url
        );
        ensure!(
            self.assignees(&reference.owner, &reference.repo)
                .await?
                .iter()
                .any(matches),
            "the specified user is not assignable in {}/{}",
            reference.owner,
            reference.repo
        );
        let body = json!({"assignees":[input.assignee]});
        let request = self
            .http
            .request(Method::PATCH, self.issue_api(&reference))
            .json(&body);
        let mut updated: Issue = self.request(request, "assign issue").await?;
        ensure!(
            updated.id == issue.id
                && updated.number == reference.number
                && updated.pull_request.is_none()
                && updated
                    .assignees
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(matches),
            "Gitea did not confirm the requested assignee on {}",
            issue.html_url
        );
        updated.html_url = issue.html_url;
        Ok(json!({"url":updated.html_url,"issue":updated}))
    }

    pub(super) async fn edit_issue(&self, mut input: issue::Edit) -> anyhow::Result<Value> {
        let reference = self.issue_ref(&input.issue)?;
        let issue = self.read_issue(&reference).await?;
        input.preserve_plan_marker(&issue.body);
        let body = input.fields("title", "body", |text| json!(text));
        let request = self.http.patch(self.issue_api(&reference)).json(&body);
        let mut updated: Issue = self.request(request, "edit issue").await.with_context(|| {
            format!(
                "could not confirm edit of {}; check the issue",
                issue.html_url
            )
        })?;
        ensure!(
            updated.id == issue.id
                && updated.number == reference.number
                && updated.pull_request.is_none()
                && input
                    .title
                    .as_ref()
                    .is_none_or(|title| title == &updated.title)
                && input
                    .description
                    .as_ref()
                    .is_none_or(|description| description == &updated.body),
            "Gitea did not confirm the requested edit on {}",
            issue.html_url
        );
        updated.html_url = issue.html_url;
        Ok(json!({"url":updated.html_url,"issue":updated}))
    }

    pub async fn apply(&self, plan: &Plan) -> anyhow::Result<Value> {
        let parent = self.issue_ref(&plan.parent)?;
        self.read_issue(&parent).await?;
        let mut created = Vec::with_capacity(plan.tasks.len());
        for (index, task) in plan.tasks.iter().enumerate() {
            let issue = self.publish_task(plan, task, index).await.with_context(|| {
                format!("task {} ({}) could not be published; {} earlier issues were resolved; reapply the same plan after checking Gitea",
                    index + 1, task.title, created.len())
            })?;
            created.push(issue);
        }
        for (task, issue) in plan.tasks.iter().zip(&created) {
            self.link_blockers(issue, &task.after, &created).await
                .with_context(|| format!("work items are created, but linking dependencies on {} failed; reapply the same plan after checking Gitea", self.issue_link(issue)))?;
        }
        let links: Vec<_> = created.iter().map(|issue| self.issue_link(issue)).collect();
        self.publish_summary(&parent, plan, &links).await
            .context("work items are created, but publishing the summary failed; reapply the same plan after checking Gitea")?;
        Ok(json!({"parent": plan.parent, "issues": links}))
    }

    async fn publish_task(
        &self,
        plan: &Plan,
        task: &Task,
        index: usize,
    ) -> anyhow::Result<IssueRef> {
        let (owner, repo) = repository(
            task.repository
                .as_deref()
                .context("missing task repository")?,
        )?;
        let endpoint = self.repo_api(owner, repo, &["issues"]);
        let marker = format!("<!-- mergelane:task:{}:{} -->", plan.id, index + 1);
        let mut search = endpoint.clone();
        search
            .query_pairs_mut()
            .append_pair("state", "all")
            .append_pair("type", "issues")
            .append_pair("q", &plan.id);
        let issues = self
            .scan::<Issue>(search, "find previously created issue")
            .await?;
        let mut existing = issues
            .into_iter()
            .filter(|issue| issue.body.lines().next() == Some(&marker));
        let issue = match existing.next() {
            Some(issue) => {
                ensure!(
                    existing.next().is_none(),
                    "multiple issues match this plan step; resolve the duplicate in Gitea"
                );
                issue
            }
            None => {
                let eligible = self.assignees(owner, repo).await?;
                ensure!(
                    eligible
                        .iter()
                        .any(|user| user.login.eq_ignore_ascii_case(&task.assignee)),
                    "{} is not assignable in {owner}/{repo}",
                    task.assignee
                );
                let body = task_body(&marker, &plan.parent, task);
                self.post(
                    endpoint,
                    &json!({"title": task.title, "body": body, "assignees":[task.assignee]}),
                    "create and assign issue",
                )
                .await?
            }
        };
        ensure!(
            issue.number > 0 && issue.pull_request.is_none(),
            "Gitea did not return a created issue"
        );
        let reference = IssueRef {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            number: issue.number,
        };
        ensure!(
            issue
                .assignees
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|user| user.login.eq_ignore_ascii_case(&task.assignee)),
            "{} is missing the planned assignee {}; verify it in Gitea",
            self.issue_link(&reference),
            task.assignee
        );
        Ok(reference)
    }

    async fn assignees(&self, owner: &str, repo: &str) -> anyhow::Result<Vec<User>> {
        self.get(
            self.repo_api(owner, repo, &["assignees"]),
            "list assignable users",
        )
        .await
    }

    async fn link_blockers(
        &self,
        issue: &IssueRef,
        after: &[usize],
        created: &[IssueRef],
    ) -> anyhow::Result<()> {
        if after.is_empty() {
            return Ok(());
        }
        let endpoint = append(self.issue_api(issue), &["dependencies"]);
        let existing: Vec<_> = self
            .scan::<Issue>(endpoint.clone(), "read blocking issues")
            .await?
            .into_iter()
            .map(|issue| self.issue_ref(&issue.html_url))
            .collect::<Result<_, _>>()?;
        for previous in after {
            let blocker = &created[*previous];
            if !existing.contains(blocker) {
                self.post::<Value>(
                    endpoint.clone(),
                    &json!({"owner": blocker.owner, "repo": blocker.repo, "index": blocker.number}),
                    "link blocking issue",
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn publish_summary(
        &self,
        parent: &IssueRef,
        plan: &Plan,
        links: &[String],
    ) -> anyhow::Result<()> {
        let endpoint = append(self.issue_api(parent), &["comments"]);
        let marker = format!("<!-- mergelane:plan:{} -->", plan.id);
        let comments = self
            .get::<Vec<Comment>>(endpoint.clone(), "check plan summary")
            .await?;
        if comments
            .iter()
            .any(|comment| comment.body.lines().next() == Some(&marker))
        {
            return Ok(());
        }
        let mut body = format!("{marker}\n## Tasks\n");
        for (task, link) in plan.tasks.iter().zip(links) {
            body.push_str(&format!("- {} — {} — {link}\n", task.title, task.assignee));
        }
        self.post::<Value>(endpoint, &json!({"body":body}), "publish plan summary")
            .await?;
        Ok(())
    }

    pub async fn checkpoint(&self, input: Checkpoint) -> anyhow::Result<Value> {
        ensure!(
            input.confirmed,
            "the user must confirm before publishing a checkpoint"
        );
        ensure!(
            simple_text(&input.summary, 500),
            "checkpoint summary must be a single line"
        );
        for lines in [&input.next, &input.checks] {
            ensure!(
                lines.len() <= 12 && lines.iter().all(|line| simple_text(line, 500)),
                "provide at most 12 single-line items"
            );
        }
        if let Some(url) = &input.commit_url {
            super::references::validate_commit_link(&self.base, url)?;
        }
        let issue = self.issue_ref(&input.issue)?;
        self.read_issue(&issue).await?;
        let comment: Comment = self
            .post(
                append(self.issue_api(&issue), &["comments"]),
                &json!({"body": input.body()}),
                "publish checkpoint",
            )
            .await?;
        Ok(json!({"issue":self.issue_link(&issue),"checkpoint":comment.html_url}))
    }
}

fn task_body(marker: &str, parent: &str, task: &Task) -> String {
    let mut body = format!(
        "{marker}\nSource issue: {parent}\n\n{}\n\n## Acceptance criteria\n",
        task.description
    );
    bullets(&mut body, &task.acceptance);
    body
}

fn bullets(body: &mut String, lines: &[String]) {
    for line in lines {
        body.push_str(&format!("- {line}\n"));
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    issue: String,
    summary: String,
    next: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    commit_url: Option<String>,
    confirmed: bool,
}

impl Checkpoint {
    fn body(&self) -> String {
        let mut body = format!("{CHECKPOINT_MARKER}\n## Checkpoint\n{}\n", self.summary);
        if let Some(commit) = &self.commit_url {
            body.push_str(&format!("\nCommit: {commit}\n"));
        }
        if !self.checks.is_empty() {
            body.push_str("\n### Verification\n");
            bullets(&mut body, &self.checks);
        }
        if !self.next.is_empty() {
            body.push_str("\n### Next\n");
            bullets(&mut body, &self.next);
        }
        body
    }
}
