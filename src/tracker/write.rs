use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Api, Issue, Plan};
use crate::validation::simple_text;

pub(super) async fn apply(api: &impl Api, plan: &Plan) -> anyhow::Result<Value> {
    api.issue(&plan.source.key).await?;
    let mut created = Vec::new();
    for (index, task) in plan.tasks.iter().enumerate() {
        let project = task.project()?;
        let members = api.members(project).await?;
        ensure!(
            members.iter().any(|member| member.id == task.assignee),
            "assignee is no longer assignable in {project}"
        );
        let marker = format!("mergelane:task:{}:{index}", plan.id);
        let issue = match api.find_task(project, &marker).await? {
            Some(issue) => issue,
            None => {
                let acceptance = task
                    .acceptance
                    .iter()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let body = format!(
                    "{marker}\nSource: {}\n\n{}\n\nAcceptance criteria\n{acceptance}",
                    plan.parent, task.description
                );
                api.create(task, &body, &marker).await.with_context(|| format!(
                    "task {} creation failed after {} earlier tasks; creation outcome may be uncertain: inspect remote state and wait for search indexing before retrying the unchanged plan", index + 1, created.len()))?
            }
        };
        ensure!(
            issue.project == project && issue.description.lines().next() == Some(&marker),
            "remote task does not match its plan marker and project"
        );
        ensure!(
            issue
                .assignees
                .iter()
                .any(|member| member.id == task.assignee),
            "remote task is missing its planned assignee; inspect {}",
            issue.url
        );
        created.push(issue);
    }
    for (task, issue) in plan.tasks.iter().zip(&created) {
        if task.after.is_empty() {
            continue;
        }
        let blockers = api.blockers(issue).await.context("tasks exist; could not inspect dependencies; retry the unchanged plan after checking platform capabilities")?;
        for &previous in &task.after {
            let blocker = &created[previous];
            if !blockers.contains(&blocker.key) {
                api.block(issue, blocker).await.context("tasks exist; dependency linking failed; inspect platform permissions and retry the unchanged plan")?;
            }
        }
    }
    let summary = created
        .iter()
        .map(|issue| format!("- {}: {}", issue.title, issue.url))
        .collect::<Vec<_>>()
        .join("\n");
    publish(
        api,
        &plan.source,
        &format!("mergelane:plan:{}", plan.id),
        &summary,
    )
    .await
    .context("tasks exist; publishing the summary failed; retry the unchanged plan")?;
    Ok(
        json!({"parent":plan.parent,"issues":created.iter().map(|issue| &issue.url).collect::<Vec<_>>()}),
    )
}

async fn publish(api: &impl Api, issue: &Issue, marker: &str, body: &str) -> anyhow::Result<()> {
    if api
        .comments(issue)
        .await?
        .iter()
        .any(|comment| comment.body.lines().next() == Some(marker))
    {
        return Ok(());
    }
    api.comment(issue, &format!("{marker}\n{body}")).await
}

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

pub(super) async fn checkpoint(api: &impl Api, checkpoint: Checkpoint) -> anyhow::Result<Value> {
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
    if let Some(url) = &checkpoint.commit_url {
        let url = reqwest::Url::parse(url)?;
        ensure!(
            url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "commit URL must be HTTPS without credentials, query or fragment"
        );
    }
    let issue = api.issue(&checkpoint.issue).await?;
    let digest = Sha256::digest(serde_json::to_vec(&checkpoint)?);
    let checks = checkpoint.checks.join("\n");
    let next = checkpoint.next.join("\n");
    let body = format!(
        "{}\nCommit: {}\nVerification:\n{checks}\nNext:\n{next}",
        checkpoint.summary,
        checkpoint.commit_url.as_deref().unwrap_or("not provided")
    );
    publish(
        api,
        &issue,
        &format!("mergelane:checkpoint:{digest:x}"),
        &body,
    )
    .await?;
    Ok(json!({"issue":issue.url,"checkpoint_id":format!("{digest:x}")}))
}
