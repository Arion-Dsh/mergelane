use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Api, Issue, Plan};
use crate::{issue, validation::simple_text};

pub(super) async fn create_issue(
    api: &impl Api,
    project: &str,
    input: &issue::Create,
) -> anyhow::Result<Value> {
    let issue = api
        .create_unassigned(project, input)
        .await
        .with_context(|| {
            format!("could not confirm creation in {project}; check the project before retrying")
        })?;
    ensure!(
        issue.project == project,
        "the platform returned a created issue in a different project; inspect {}",
        issue.url
    );
    ensure!(
        issue.assignees.is_empty(),
        "the platform created {} with an assignee; check the project's assignment rules",
        issue.url
    );
    Ok(json!({"url":issue.url, "issue":issue}))
}

pub(super) async fn assign_issue(api: &impl Api, input: issue::Assign) -> anyhow::Result<Value> {
    let issue = api.issue(&input.issue).await?;
    let matches = |member: &super::Member| {
        if api.name() == "github" {
            member.id.eq_ignore_ascii_case(&input.assignee)
        } else {
            member.id == input.assignee
        }
    };
    if issue.assignees.iter().any(&matches) {
        return Ok(json!({"url":issue.url, "issue":issue}));
    }
    ensure!(
        issue.assignees.is_empty(),
        "{} already has an assignee",
        issue.url
    );
    let updated = api
        .assign(&issue, &input.assignee)
        .await
        .with_context(|| format!("could not confirm assignment of {}", issue.url))?;
    ensure!(
        updated.id == issue.id && updated.key == issue.key && updated.assignees.iter().any(matches),
        "the platform did not confirm the requested assignee on {}",
        issue.url
    );
    Ok(json!({"url":updated.url, "issue":updated}))
}

pub(super) async fn edit_issue(api: &impl Api, mut input: issue::Edit) -> anyhow::Result<Value> {
    let issue = api.issue(&input.issue).await?;
    input.preserve_plan_marker(&issue.description);
    let updated = api
        .edit(&issue, &input)
        .await
        .with_context(|| format!("could not confirm edit of {}; check the issue", issue.url))?;
    ensure!(
        updated.id == issue.id
            && updated.key == issue.key
            && updated.project == issue.project
            && input
                .title
                .as_ref()
                .is_none_or(|title| title == &updated.title)
            && input.description.as_ref().is_none_or(|description| {
                description.trim_end_matches('\n') == updated.description.trim_end_matches('\n')
            }),
        "the platform did not confirm the requested edit on {}",
        issue.url
    );
    Ok(json!({"url":updated.url, "issue":updated}))
}

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
