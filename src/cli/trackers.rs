use std::path::PathBuf;

use clap::Args;
use reqwest::Url;

use super::WriteOptions;
use crate::{
    credentials::read_token,
    github::GitHub,
    gitlab::GitLab,
    jira::{Jira, Options as JiraOptions},
    tracker::{Backends, Session},
};

#[derive(Args)]
pub(super) struct TrackerConnection {
    #[arg(long, env = "GITHUB_URL", requires = "github_token_file")]
    github_url: Option<Url>,
    #[arg(long, env = "GITHUB_TOKEN_FILE", requires = "github_url")]
    github_token_file: Option<PathBuf>,
    #[arg(long, env = "GITLAB_URL", requires = "gitlab_token_file")]
    gitlab_url: Option<Url>,
    #[arg(long, env = "GITLAB_TOKEN_FILE", requires = "gitlab_url")]
    gitlab_token_file: Option<PathBuf>,
    #[arg(long, env = "JIRA_URL", requires_all = ["jira_token_file", "jira_email"])]
    jira_url: Option<Url>,
    #[arg(long, env = "JIRA_TOKEN_FILE", requires = "jira_url")]
    jira_token_file: Option<PathBuf>,
    #[arg(long, env = "JIRA_EMAIL", requires = "jira_url")]
    jira_email: Option<String>,
    /// Atlassian Cloud ID for scoped tokens, using the official API gateway.
    #[arg(long, env = "JIRA_CLOUD_ID", requires = "jira_url")]
    jira_cloud_id: Option<String>,
    #[arg(long, env = "JIRA_ISSUE_TYPE", requires = "jira_url")]
    jira_issue_type: Option<String>,
    #[arg(long, env = "JIRA_LINK_TYPE", requires = "jira_url")]
    jira_link_type: Option<String>,
}

impl TrackerConnection {
    pub(super) async fn open(self, writes: &WriteOptions) -> anyhow::Result<Backends> {
        let github = match (self.github_url, self.github_token_file) {
            (Some(url), Some(file)) => Some(Session::new(
                GitHub::new(url, read_token(&file).await?)?,
                writes.github_write,
            )),
            (None, None) => None,
            _ => anyhow::bail!("configure both GITHUB_URL and GITHUB_TOKEN_FILE"),
        };
        let gitlab = match (self.gitlab_url, self.gitlab_token_file) {
            (Some(url), Some(file)) => Some(Session::new(
                GitLab::new(url, read_token(&file).await?)?,
                writes.gitlab_write,
            )),
            (None, None) => None,
            _ => anyhow::bail!("configure both GITLAB_URL and GITLAB_TOKEN_FILE"),
        };
        let jira = match (self.jira_url, self.jira_token_file, self.jira_email) {
            (Some(url), Some(file), Some(email)) => Some(Session::new(
                Jira::new(
                    url,
                    read_token(&file).await?,
                    JiraOptions {
                        email,
                        issue_type: self.jira_issue_type.unwrap_or_else(|| "Task".to_owned()),
                        link_type: self.jira_link_type.unwrap_or_else(|| "Blocks".to_owned()),
                        cloud_id: self.jira_cloud_id,
                    },
                )?,
                writes.jira_write,
            )),
            (None, None, None) => None,
            _ => anyhow::bail!("configure JIRA_URL, JIRA_TOKEN_FILE and JIRA_EMAIL together"),
        };
        Ok(Backends {
            github,
            gitlab,
            jira,
        })
    }
}
