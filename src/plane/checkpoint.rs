use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Plane, WorkItemRef, references::html};
use crate::validation::simple_text;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    issue: String,
    summary: String,
    next: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    commit_url: Option<String>,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParentDraft {
    issue: String,
    progress: Progress,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishParent {
    issue: String,
    progress: Progress,
    preview_id: String,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    summary: String,
    next: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
    commit_url: Option<String>,
}

#[derive(Serialize)]
struct ParentPreview {
    preview_id: String,
    child_url: String,
    child_title: String,
    parent_url: String,
    parent_title: String,
    comment_html: String,
}

impl Plane {
    pub(super) async fn checkpoint(&self, checkpoint: Checkpoint) -> anyhow::Result<Value> {
        ensure!(
            checkpoint.confirmed,
            "the user must confirm before publishing a checkpoint"
        );
        let body = checkpoint_body(
            &checkpoint.summary,
            &checkpoint.next,
            &checkpoint.checks,
            checkpoint.commit_url.as_deref(),
        )?;
        let item = self.reference(&checkpoint.issue).await?;
        let work_item = self.item(&item).await?;
        // Keep the existing input and digest format for retries from earlier versions.
        let digest = Sha256::digest(serde_json::to_vec(&checkpoint)?);
        let external = format!("checkpoint:{}:{digest:x}", item.item);
        let comment = self.publish_comment(&item, &external, body).await?;
        Ok(json!({"issue":self.item_link(&item, &work_item).await?,"comment_id":comment.id}))
    }

    pub(super) async fn preview_parent_checkpoint(
        &self,
        draft: ParentDraft,
    ) -> anyhow::Result<Value> {
        let (_, preview) = self
            .parent_checkpoint(&draft.issue, &draft.progress)
            .await?;
        Ok(serde_json::to_value(preview)?)
    }

    pub(super) async fn publish_parent_checkpoint(
        &self,
        args: PublishParent,
    ) -> anyhow::Result<Value> {
        ensure!(
            args.confirmed,
            "the user must confirm before publishing a checkpoint"
        );
        let (parent, preview) = self.parent_checkpoint(&args.issue, &args.progress).await?;
        ensure!(
            args.preview_id == preview.preview_id,
            "the parent checkpoint no longer matches the preview; preview it again and ask the user to confirm"
        );
        let external = format!("parent-checkpoint:{}", preview.preview_id);
        let comment = self
            .publish_comment(&parent, &external, preview.comment_html)
            .await
            .with_context(|| {
                format!(
                    "could not confirm checkpoint on {}; check it before retrying",
                    preview.parent_url
                )
            })?;
        Ok(
            json!({"child_url":preview.child_url,"parent_url":preview.parent_url,"comment_id":comment.id}),
        )
    }

    async fn parent_checkpoint(
        &self,
        issue: &str,
        progress: &Progress,
    ) -> anyhow::Result<(WorkItemRef, ParentPreview)> {
        let body = checkpoint_body(
            &progress.summary,
            &progress.next,
            &progress.checks,
            progress.commit_url.as_deref(),
        )?;
        let child = self.reference(issue).await?;
        let (item, user) = tokio::try_join!(self.item(&child), self.current_user())?;
        ensure!(
            item.assignees.contains(&user),
            "the child work item must be assigned to the current Plane account"
        );
        let parent_id = item
            .parent
            .filter(|id| *id != child.item)
            .context("this work item has no native Plane parent")?;
        let parent = WorkItemRef {
            item: parent_id,
            ..child.clone()
        };
        let (parent_item, project) =
            tokio::try_join!(self.item(&parent), self.item_project(&child))
                .context("could not read the native Plane parent in the child's project")?;
        let child_url = self.browse_link(&child.workspace, &project.identifier, item.sequence_id);
        let parent_url = self.browse_link(
            &parent.workspace,
            &project.identifier,
            parent_item.sequence_id,
        );
        let comment_html = format!(
            "<p>Child work item: <a href=\"{}\">{}</a></p>{body}",
            html(&child_url),
            html(&item.name)
        );
        // Bind the reviewed text to both work items so a reparented child needs a new preview.
        let digest = Sha256::digest(serde_json::to_vec(&(
            &child,
            &parent,
            &parent_url,
            &parent_item.name,
            &comment_html,
        ))?);
        Ok((
            parent,
            ParentPreview {
                preview_id: format!("{digest:x}"),
                child_url,
                child_title: item.name,
                parent_url,
                parent_title: parent_item.name,
                comment_html,
            },
        ))
    }
}

fn checkpoint_body(
    summary: &str,
    next: &[String],
    checks: &[String],
    commit_url: Option<&str>,
) -> anyhow::Result<String> {
    ensure!(
        simple_text(summary, 500),
        "checkpoint summary must be a single line"
    );
    for lines in [next, checks] {
        ensure!(
            lines.len() <= 12 && lines.iter().all(|line| simple_text(line, 500)),
            "provide at most 12 single-line items"
        );
    }
    if let Some(commit_url) = commit_url {
        let url = reqwest::Url::parse(commit_url).context("invalid checkpoint commit URL")?;
        ensure!(
            url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "checkpoint commit URL must use HTTPS without credentials"
        );
    }
    let mut body = format!(
        "<h3>Checkpoint</h3><p>{}</p><h4>Verification</h4><ul>",
        html(summary)
    );
    if let Some(commit_url) = commit_url {
        body.push_str(&format!("<li>Commit: {}</li>", html(commit_url)));
    }
    for line in checks {
        body.push_str(&format!("<li>{}</li>", html(line)));
    }
    body.push_str("</ul><h4>Next</h4><ul>");
    for line in next {
        body.push_str(&format!("<li>{}</li>", html(line)));
    }
    body.push_str("</ul>");
    Ok(body)
}
