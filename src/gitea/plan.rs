use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Gitea, IssueRef, repository};
use crate::validation::simple_text;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub parent: String,
    pub tasks: Vec<Task>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub title: String,
    pub repository: Option<String>,
    #[serde(default)]
    pub description: String,
    pub assignee: String,
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub after: Vec<usize>,
}

#[derive(Serialize)]
pub struct Plan {
    pub id: String,
    pub parent: String,
    pub tasks: Vec<Task>,
}

impl Plan {
    pub fn prepare(gitea: &Gitea, draft: Draft) -> anyhow::Result<Self> {
        let parent = gitea.issue_ref(&draft.parent)?;
        let parent_repository = format!("{}/{}", parent.owner, parent.repo);
        let mut tasks = draft.tasks;
        ensure!((1..=20).contains(&tasks.len()), "provide 1 to 20 tasks");
        for (index, task) in tasks.iter_mut().enumerate() {
            validate(task, index)?;
            let repo = task
                .repository
                .get_or_insert_with(|| parent_repository.clone());
            repository(repo).with_context(|| format!("task {} repository", index + 1))?;
            repo.make_ascii_lowercase();
            task.after.sort_unstable();
            task.after.dedup();
        }
        let parent = gitea.issue_link(&IssueRef {
            owner: parent.owner.to_ascii_lowercase(),
            repo: parent.repo.to_ascii_lowercase(),
            number: parent.number,
        });
        let digest = Sha256::digest(serde_json::to_vec(&(&parent, &tasks))?);
        Ok(Self {
            id: format!("{digest:x}"),
            parent,
            tasks,
        })
    }
}

fn validate(task: &Task, index: usize) -> anyhow::Result<()> {
    ensure!(
        simple_text(&task.title, 200),
        "task {} needs a single-line title",
        index + 1
    );
    ensure!(
        simple_text(&task.assignee, 64)
            && task
                .assignee
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "task {} needs a Gitea username",
        index + 1
    );
    ensure!(
        task.description.len() <= 8_000 && !task.description.contains("<!-- mergelane:"),
        "task {} has an invalid description",
        index + 1
    );
    ensure!(
        (1..=12).contains(&task.acceptance.len())
            && task.acceptance.iter().all(|line| simple_text(line, 500)),
        "task {} needs 1 to 12 single-line acceptance criteria",
        index + 1
    );
    ensure!(
        task.after.iter().all(|&previous| previous < index),
        "after must contain earlier zero-based task indices"
    );
    Ok(())
}
