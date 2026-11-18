use std::collections::BTreeMap;

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Api, Issue};
use crate::validation::simple_text;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Draft {
    parent: String,
    tasks: Vec<Task>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub title: String,
    pub project: Option<String>,
    #[serde(default)]
    pub description: String,
    pub assignee: String,
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub after: Vec<usize>,
}

#[derive(Serialize)]
pub(crate) struct Plan {
    pub id: String,
    pub parent: String,
    pub tasks: Vec<Task>,
    #[serde(skip)]
    pub source: Issue,
}

impl Plan {
    pub async fn prepare(api: &impl Api, draft: Draft) -> anyhow::Result<Self> {
        ensure!(
            (1..=20).contains(&draft.tasks.len()),
            "provide 1 to 20 tasks"
        );
        let mut tasks = draft.tasks;
        for (index, task) in tasks.iter_mut().enumerate() {
            task.validate(index)?;
        }
        let source = api.issue(&draft.parent).await?;
        let mut members = BTreeMap::new();
        for task in &mut tasks {
            let project = api.project(task.project.as_deref().unwrap_or(&source.project))?;
            if !members.contains_key(&project) {
                members.insert(project.clone(), api.members(&project).await?);
            }
            ensure!(
                members[&project]
                    .iter()
                    .any(|member| member.id == task.assignee),
                "assignee is not an assignable member of {project}"
            );
            task.project = Some(project);
        }
        let digest = Sha256::digest(serde_json::to_vec(&(api.name(), &source.url, &tasks))?);
        Ok(Self {
            id: format!("{digest:x}"),
            parent: source.url.clone(),
            tasks,
            source,
        })
    }
}

impl Task {
    fn validate(&mut self, index: usize) -> anyhow::Result<()> {
        ensure!(
            simple_text(&self.title, 200),
            "task {} requires a single-line title",
            index + 1
        );
        ensure!(
            simple_text(&self.assignee, 255),
            "task {} requires an assignee ID",
            index + 1
        );
        ensure!(
            self.description.len() <= 8000 && !self.description.contains("mergelane:"),
            "invalid task description"
        );
        ensure!(
            (1..=12).contains(&self.acceptance.len())
                && self.acceptance.iter().all(|line| simple_text(line, 500)),
            "provide 1 to 12 single-line acceptance criteria"
        );
        ensure!(
            self.after.iter().all(|&previous| previous < index),
            "after must contain earlier zero-based task indices"
        );
        self.after.sort_unstable();
        self.after.dedup();
        Ok(())
    }

    pub fn project(&self) -> anyhow::Result<&str> {
        self.project
            .as_deref()
            .context("preview must resolve the task project")
    }
}
