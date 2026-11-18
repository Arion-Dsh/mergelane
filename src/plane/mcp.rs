//! Plane MCP tools and the latest preview owned by one client session.

use anyhow::{Context as _, ensure};
use serde_json::{Value, json};

use super::{
    Plane,
    plan::{Draft, Plan},
    write::Checkpoint,
};
use crate::{
    gitea::Gitea,
    mcp::{ApplyArgs, ContextArgs, object, tool},
};

pub(crate) struct Session {
    write: bool,
    pending: Option<Plan>,
}

impl Session {
    pub(crate) fn new(write: bool) -> Self {
        Self {
            write,
            pending: None,
        }
    }

    pub(crate) fn tools(&self) -> Vec<Value> {
        tools(self.write)
    }

    pub(crate) async fn call(
        &mut self,
        plane: &Plane,
        gitea: Option<&Gitea>,
        name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        match name {
            "plane_search_issues" => plane.search(serde_json::from_value(args)?).await,
            "plane_issue_context" => {
                let args: ContextArgs = serde_json::from_value(args)?;
                plane.context(&args.issue, gitea).await
            }
            "plane_preview_plan" => {
                let plan = Plan::prepare(plane, serde_json::from_value::<Draft>(args)?).await?;
                let preview = serde_json::to_value(&plan)?;
                self.pending = Some(plan);
                Ok(preview)
            }
            "plane_apply_plan" if self.write => {
                let args: ApplyArgs = serde_json::from_value(args)?;
                ensure!(
                    args.confirmed,
                    "the user must confirm the preview before publishing"
                );
                let plan = self
                    .pending
                    .as_ref()
                    .context("preview a Plane plan in this MCP session first")?;
                ensure!(
                    args.plan_id == plan.id,
                    "plan ID does not match the latest Plane preview"
                );
                plane.apply(plan).await
            }
            "plane_checkpoint" if self.write => {
                plane
                    .checkpoint(serde_json::from_value::<Checkpoint>(args)?)
                    .await
            }
            _ => anyhow::bail!("unknown or disabled Plane tool"),
        }
    }
}

fn tools(write: bool) -> Vec<Value> {
    let text = json!({"type":"string"});
    let id = json!({"type":"string","format":"uuid"});
    let lines = json!({"type":"array","items":text,"maxItems":12});
    let mut tools = vec![
        tool(
            "plane_search_issues",
            "Find team work items by title, identifier, project name or teammate display name. Omit filters for a workspace overview. Mine restricts to the current account. Uses the configured workspace unless overridden. Read result URLs with plane_issue_context for progress and blockers. Ask which task to use when titles are ambiguous.",
            object(
                json!({"query":text,"mine":{"type":"boolean","default":false},"assignee":text,"workspace":text,
                "project":text,"limit":{"type":"integer","minimum":1,"maximum":50,"default":20}}),
                &[],
            ),
            true,
        ),
        tool(
            "plane_issue_context",
            "Read a Plane work item URL, project members, comments and blocking relationships. If Gitea is configured, check linked Git file versions. Recheck before starting or submitting work.",
            object(json!({"issue":text}), &["issue"]),
            true,
        ),
        tool(
            "plane_preview_plan",
            "Preview a task split for the user to confirm. Uses project member UUIDs from plane_issue_context. Tasks default to the parent's project. After uses earlier zero-based task indices.",
            object(
                json!({"parent":text,"tasks":{"type":"array","minItems":1,"maxItems":20,
                "items":object(json!({"title":text,"project":id,"description":text,"assignee":id,
                    "acceptance":lines,"after":{"type":"array","items":{"type":"integer","minimum":0}}}),
                    &["title","assignee","acceptance"])}}),
                &["parent", "tasks"],
            ),
            true,
        ),
    ];
    if write {
        tools.extend([
            tool("plane_apply_plan", "Publish the latest preview after the user confirms it. Creates assigned work items, native blocking relations, and a summary comment on the parent.",
                object(json!({"plan_id":text,"confirmed":{"type":"boolean"}}), &["plan_id","confirmed"]), false),
            tool("plane_checkpoint", "After the user confirms the text, share progress, verification and next actions in a Plane work item comment.",
                object(json!({"issue":text,"summary":text,"next":lines,"checks":lines,"commit_url":text,"confirmed":{"type":"boolean"}}),
                    &["issue","summary","next","confirmed"]), false)
        ]);
    }
    tools
}
