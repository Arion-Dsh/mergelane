//! Poll explicitly selected issues and emit changed states as JSON lines.

use std::{io::Write as _, time::Duration};

use serde_json::{Value, json};

use super::{Gitea, IssueRef};

impl Gitea {
    pub async fn watch(&self, issues: &[String], interval: Duration) -> anyhow::Result<()> {
        let issues: Vec<_> = issues
            .iter()
            .map(|issue| self.issue_ref(issue))
            .collect::<Result<_, _>>()?;
        let mut previous = vec![None; issues.len()];
        let mut timer = tokio::time::interval(interval);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            for (issue, previous) in issues.iter().zip(&mut previous) {
                let state = match self.watch_state(issue).await {
                    Ok(state) => state,
                    Err(error) => {
                        json!({"issue": self.issue_link(issue), "error": error.to_string()})
                    }
                };
                if previous.as_ref() != Some(&state) {
                    let mut stdout = std::io::stdout().lock();
                    writeln!(stdout, "{state}")?;
                    stdout.flush()?;
                    *previous = Some(state);
                }
            }
        }
    }

    async fn watch_state(&self, issue: &IssueRef) -> anyhow::Result<Value> {
        let response = self.read_issue(issue).await?;
        Ok(json!({
            "issue": response.html_url,
            "state": response.state,
            "assignees": response.assignees,
            "dependencies": self.dependency_reports(&response.body).await?
        }))
    }
}
