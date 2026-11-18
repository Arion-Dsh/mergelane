use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Plane, WorkItemRef, references::html};
use crate::validation::simple_text;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub parent: String,
    pub tasks: Vec<Task>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub title: String,
    pub project: Option<Uuid>,
    #[serde(default)]
    pub description: String,
    /// A project member UUID, obtained from the context tool.
    pub assignee: Uuid,
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub after: Vec<usize>,
}

#[derive(Serialize)]
pub struct Plan {
    pub id: String,
    pub parent: String,
    pub tasks: Vec<Task>,
    #[serde(skip)]
    pub reference: WorkItemRef,
}

impl Plan {
    pub async fn prepare(plane: &Plane, draft: Draft) -> anyhow::Result<Self> {
        let reference = plane.reference(&draft.parent).await?;
        let item = plane.item(&reference).await?;
        let mut tasks = draft.tasks;
        ensure!((1..=20).contains(&tasks.len()), "provide 1 to 20 tasks");
        for (index, task) in tasks.iter_mut().enumerate() {
            ensure!(
                simple_text(&task.title, 200),
                "task {} needs a single-line title",
                index + 1
            );
            ensure!(
                task.description.len() <= 8_000,
                "task description exceeds 8000 bytes"
            );
            ensure!(
                (1..=12).contains(&task.acceptance.len())
                    && task.acceptance.iter().all(|line| simple_text(line, 500)),
                "task {} needs 1 to 12 single-line acceptance criteria",
                index + 1
            );
            ensure!(
                task.after.iter().all(|&previous| previous < index),
                "after must contain earlier zero-based indices"
            );
            task.after.sort_unstable();
            task.after.dedup();
            let project = *task.project.get_or_insert(reference.project);
            let members = plane
                .members(&reference.workspace, project)
                .await
                .with_context(|| format!("read members for task {}", index + 1))?;
            ensure!(
                members.iter().any(|member| member.id == task.assignee),
                "task {} assignee is not a member of its Plane project",
                index + 1
            );
        }
        let parent = plane.item_link(&reference, &item).await?;
        let digest = Sha256::digest(serde_json::to_vec(&(&parent, &tasks))?);
        Ok(Self {
            id: format!("{digest:x}"),
            parent,
            tasks,
            reference,
        })
    }
}

impl Task {
    pub fn description_html(&self, parent: &str) -> String {
        let mut body = format!(
            "<p>Source work item: {}</p><p>{}</p><h3>Acceptance criteria</h3><ul>",
            html(parent),
            html(&self.description)
        );
        for line in &self.acceptance {
            body.push_str(&format!("<li>{}</li>", html(line)));
        }
        body.push_str("</ul>");
        body
    }
}
