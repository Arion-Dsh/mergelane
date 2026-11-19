use anyhow::ensure;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Gitea, Issue, repository};
use crate::validation::simple_text;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub query: Option<String>,
    #[serde(default)]
    pub mine: bool,
    pub owner: Option<String>,
    pub assignee: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
    #[serde(skip)]
    pub(crate) project: Option<String>,
    #[serde(skip)]
    pub(crate) open_only: bool,
}

fn first_page() -> u32 {
    1
}

impl Gitea {
    pub async fn search(&self, input: Search) -> anyhow::Result<Value> {
        ensure!(
            input.page > 0 && input.page < u32::MAX,
            "page must be positive"
        );
        if let Some(query) = &input.query {
            ensure!(
                simple_text(query, 250),
                "query must contain 1 to 250 single-line bytes"
            );
        }
        for name in [&input.owner, &input.assignee].into_iter().flatten() {
            ensure!(
                simple_text(name, 255),
                "owner and assignee must be single-line names"
            );
        }
        let mut url = match &input.project {
            Some(project) => {
                let (owner, repo) = repository(project)?;
                self.repo_api(owner, repo, &["issues"])
            }
            None => self.url(&["api", "v1", "repos", "issues", "search"]),
        };
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("type", "issues")
                .append_pair("state", if input.open_only { "open" } else { "all" })
                .append_pair("page", &input.page.to_string())
                .append_pair("limit", "30");
            if let Some(text) = &input.query {
                query.append_pair("q", text);
            }
            if let Some(owner) = &input.owner {
                query.append_pair("owner", owner);
            }
            if input.mine {
                query.append_pair("assigned", "true");
            }
        }
        let issues: Vec<Issue> = self.get(url, "search issues").await?;
        let next_page = (!issues.is_empty()).then_some(input.page + 1);
        let mut items = Vec::new();
        for issue in issues {
            if issue.pull_request.is_some() {
                continue;
            }
            if input.assignee.as_ref().is_some_and(|name| {
                !issue
                    .assignees
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|user| user.login.eq_ignore_ascii_case(name))
            }) {
                continue;
            }
            let reference = self.issue_ref(&issue.html_url)?;
            items.push(
                json!({"title":issue.title,"url":self.issue_link(&reference),"state":issue.state,
                "number":issue.number,"repository":format!("{}/{}",reference.owner,reference.repo),
                "assignees":issue.assignees}),
            );
        }
        Ok(json!({"items":items,"next_page":next_page,
            "notice":"Results reflect the current account's Gitea permissions. Follow next_page for more candidates. Ask the user which issue to use if several match."}))
    }
}
