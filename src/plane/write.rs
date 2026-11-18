use anyhow::{Context as _, ensure};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    ApiError, Comment, Plane, SOURCE, WorkItem, WorkItemRef, plan::Plan, references::html,
};
use crate::validation::simple_text;

impl Plane {
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

    async fn publish_comment(
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

    pub async fn checkpoint(&self, checkpoint: Checkpoint) -> anyhow::Result<Value> {
        ensure!(
            checkpoint.confirmed,
            "the user must confirm before publishing a checkpoint"
        );
        ensure!(
            simple_text(&checkpoint.summary, 500),
            "checkpoint summary must be a single line"
        );
        for lines in [&checkpoint.next, &checkpoint.checks] {
            ensure!(
                lines.len() <= 12 && lines.iter().all(|line| simple_text(line, 500)),
                "provide at most 12 single-line items"
            );
        }
        if let Some(commit_url) = &checkpoint.commit_url {
            let url = reqwest::Url::parse(commit_url).context("invalid checkpoint commit URL")?;
            ensure!(
                url.scheme() == "https"
                    && url.host().is_some()
                    && url.username().is_empty()
                    && url.password().is_none(),
                "checkpoint commit URL must use HTTPS without credentials"
            );
        }
        let item = self.reference(&checkpoint.issue).await?;
        let work_item = self.item(&item).await?;
        let digest = Sha256::digest(serde_json::to_vec(&checkpoint)?);
        let external = format!("checkpoint:{}:{digest:x}", item.item);
        let mut body = format!(
            "<h3>Checkpoint</h3><p>{}</p><h4>Verification</h4><ul>",
            html(&checkpoint.summary)
        );
        if let Some(commit_url) = &checkpoint.commit_url {
            body.push_str(&format!("<li>Commit: {}</li>", html(commit_url)));
        }
        for line in &checkpoint.checks {
            body.push_str(&format!("<li>{}</li>", html(line)));
        }
        body.push_str("</ul><h4>Next</h4><ul>");
        for line in &checkpoint.next {
            body.push_str(&format!("<li>{}</li>", html(line)));
        }
        body.push_str("</ul>");
        let comment = self.publish_comment(&item, &external, body).await?;
        Ok(json!({"issue":self.item_link(&item, &work_item).await?,"comment_id":comment.id}))
    }
}

fn status(error: &anyhow::Error) -> Option<StatusCode> {
    error.downcast_ref::<ApiError>().map(|error| error.status)
}

#[derive(Deserialize, Serialize)]
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
