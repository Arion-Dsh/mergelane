//! Gitea MCP tools and the latest preview owned by one client session.

use anyhow::{Context as _, ensure};
use serde_json::{Value, json};

use super::{
    Gitea,
    plan::{Draft, Plan},
    write::Checkpoint,
};
use crate::{
    issue,
    mcp::{ApplyArgs, ContextArgs, object, tool},
};

const CONTEXT: &str = "gitea_issue_context";
const PREVIEW: &str = "gitea_preview_plan";
const APPLY: &str = "gitea_apply_plan";
const CHECKPOINT: &str = "gitea_checkpoint";

pub(crate) struct Session {
    write: bool,
    default_project: Option<String>,
    pending: Option<Plan>,
}

impl Session {
    pub(crate) fn new(write: bool, default_project: Option<String>) -> Self {
        Self {
            write,
            default_project,
            pending: None,
        }
    }

    pub(crate) fn tools(&self) -> Vec<Value> {
        tools(self.write)
    }

    pub(crate) async fn call(
        &mut self,
        gitea: &Gitea,
        name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        match name {
            "gitea_search_issues" => gitea.search(serde_json::from_value(args)?).await,
            CONTEXT => {
                let args: ContextArgs = serde_json::from_value(args)?;
                gitea.context(&args.issue).await
            }
            PREVIEW => {
                let plan = Plan::prepare(gitea, serde_json::from_value::<Draft>(args)?)?;
                gitea.read_issue(&gitea.issue_ref(&plan.parent)?).await?;
                let preview = serde_json::to_value(&plan)?;
                self.pending = Some(plan);
                Ok(preview)
            }
            APPLY if self.write => {
                let args: ApplyArgs = serde_json::from_value(args)?;
                ensure!(
                    args.confirmed,
                    "the user must confirm the preview before publishing"
                );
                let plan = self
                    .pending
                    .as_ref()
                    .context("preview a plan in this MCP session first")?;
                ensure!(
                    plan.id == args.plan_id,
                    "plan ID does not match the latest preview"
                );
                gitea.apply(plan).await
            }
            CHECKPOINT if self.write => {
                let args: Checkpoint = serde_json::from_value(args)?;
                gitea.checkpoint(args).await
            }
            "gitea_create_issue" if self.write => {
                let input = issue::Create::parse(args)?;
                let project = input.project(self.default_project.as_deref())?;
                gitea.create_issue(project, &input).await
            }
            "gitea_assign_issue" if self.write => {
                gitea.assign_issue(issue::Assign::parse(args)?).await
            }
            "gitea_edit_issue" if self.write => gitea.edit_issue(issue::Edit::parse(args)?).await,
            _ => anyhow::bail!("unknown or disabled tool"),
        }
    }
}

fn tools(write: bool) -> Vec<Value> {
    let text = json!({"type":"string"});
    let lines = json!({"type":"array","items":text,"maxItems":12});
    let flag = json!({"type":"boolean"});
    let mut result = vec![
        tool(
            "gitea_search_issues",
            "Find accessible team issues by title, repository owner or assignee username. Mine filters to the current account. Results provide URLs for gitea_issue_context. Follow next_page, and ask which task to use when several titles match.",
            object(
                json!({"query":text,"mine":{"type":"boolean","default":false},
                "owner":text,"assignee":text,"page":{"type":"integer","minimum":1,"default":1}}),
                &[],
            ),
            true,
        ),
        tool(
            CONTEXT,
            "Read one explicit Gitea issue, its blockers, versioned files, published plans, and team checkpoints. Recheck before resuming work or submitting.",
            object(json!({"issue":text}), &["issue"]),
            true,
        ),
        tool(
            PREVIEW,
            "Preview a task split without changing Gitea. Display it and ask the user to confirm it. Use the ticket owner's Gitea username for each task. After uses zero-based indices of earlier tasks.",
            object(
                json!({"parent":text,"tasks":{"type":"array","minItems":1,"maxItems":20,
                "items":object(json!({"title":text,"repository":text,"description":text,"assignee":text,
                    "acceptance":lines,"after":{"type":"array","items":{"type":"integer","minimum":0}}}),
                    &["title","assignee","acceptance"])}}),
                &["parent", "tasks"],
            ),
            true,
        ),
    ];
    if write {
        result.extend(issue::tools(
            "gitea",
            "Repository as owner/repo.",
            "Gitea username.",
        ));
        result.extend([
            tool(APPLY, "Publish the latest preview in this MCP session only after the user explicitly confirms it. Creates Gitea issues, assigns owners, and records dependencies.",
                object(json!({"plan_id":text,"confirmed":flag}), &["plan_id","confirmed"]), false),
            tool(CHECKPOINT, "After the user approves the text, publish a shared checkpoint on the issue. Include progress, verification, and the next action; use a pushed commit link when available.",
                object(json!({"issue":text,"summary":text,"next":lines,"checks":lines,"commit_url":text,"confirmed":flag}), &["issue","summary","next","confirmed"]), false),
        ]);
    }
    result
}
