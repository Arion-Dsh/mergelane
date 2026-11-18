//! Command-line arguments and construction of configured backend clients.

mod profile;
mod trackers;

pub(crate) use profile::parse;

use std::path::PathBuf;

use anyhow::ensure;
use clap::{Args, Parser, Subcommand};
use reqwest::Url;

use crate::{credentials::read_token, gitea::Gitea, plane};
use trackers::TrackerConnection;

#[derive(Parser)]
#[command(
    version,
    about = "Team collaboration tools for coding agents across Gitea, Plane, GitHub, GitLab and Jira"
)]
pub(crate) struct Cli {
    /// Personal configuration file; defaults to the user's MergeLane config directory.
    #[arg(long, global = true, conflicts_with = "no_config")]
    pub(crate) config: Option<PathBuf>,
    /// Ignore saved configuration and use only explicit arguments and environment.
    #[arg(long, global = true)]
    no_config: bool,
    #[command(subcommand)]
    pub(crate) mode: Mode,
}

#[derive(Args)]
pub(crate) struct Connection {
    #[arg(long, env = "GITEA_URL")]
    gitea_url: Url,
    #[arg(long, env = "GITEA_TOKEN_FILE")]
    gitea_token_file: PathBuf,
}

#[derive(Args)]
pub(crate) struct McpConnection {
    #[command(flatten)]
    trackers: TrackerConnection,
    #[arg(long, env = "GITEA_URL", requires = "gitea_token_file")]
    gitea_url: Option<Url>,
    #[arg(long, env = "GITEA_TOKEN_FILE", requires = "gitea_url")]
    gitea_token_file: Option<PathBuf>,
    #[arg(long, env = "PLANE_URL", requires = "plane_token_file")]
    plane_url: Option<Url>,
    #[arg(long, env = "PLANE_TOKEN_FILE", requires = "plane_url")]
    plane_token_file: Option<PathBuf>,
    /// Default Plane workspace slug for searching work items.
    #[arg(long, env = "PLANE_WORKSPACE", requires = "plane_url")]
    plane_workspace: Option<String>,
}

impl McpConnection {
    pub(crate) async fn open(self, writes: &WriteOptions) -> anyhow::Result<Connections> {
        let gitea = match (self.gitea_url, self.gitea_token_file) {
            (Some(url), Some(file)) => Some(Gitea::new(url, read_token(&file).await?)?),
            (None, None) => None,
            _ => anyhow::bail!("configure both GITEA_URL and GITEA_TOKEN_FILE"),
        };
        let plane = match (self.plane_url, self.plane_token_file) {
            (Some(url), Some(file)) => Some(plane::Plane::new(
                url,
                read_token(&file).await?,
                self.plane_workspace,
            )?),
            (None, None) => None,
            _ => anyhow::bail!("configure both PLANE_URL and PLANE_TOKEN_FILE"),
        };
        let trackers = self.trackers.open(writes).await?;
        ensure!(
            gitea.is_some() || plane.is_some() || !trackers.is_empty(),
            "configure at least one task platform"
        );
        Ok(Connections {
            gitea,
            plane,
            trackers,
        })
    }
}

pub(crate) struct Connections {
    pub gitea: Option<Gitea>,
    pub plane: Option<plane::Plane>,
    pub trackers: crate::tracker::Backends,
}

#[derive(Args, Default)]
pub(crate) struct WriteOptions {
    /// Disable all write tools, including those enabled in saved configuration.
    #[arg(long)]
    read_only: bool,
    /// Enable Gitea task and checkpoint publishing.
    #[arg(long, requires = "gitea_url")]
    pub gitea_write: bool,
    /// Enable Plane task and checkpoint publishing.
    #[arg(long, requires = "plane_url")]
    pub plane_write: bool,
    /// Enable GitHub task and checkpoint publishing.
    #[arg(long, requires = "github_url")]
    pub github_write: bool,
    /// Enable GitLab task and checkpoint publishing.
    #[arg(long, requires = "gitlab_url")]
    pub gitlab_write: bool,
    /// Enable Jira task and checkpoint publishing.
    #[arg(long, requires = "jira_url")]
    pub jira_write: bool,
}

impl WriteOptions {
    pub fn effective(mut self) -> Self {
        if self.read_only {
            self.gitea_write = false;
            self.plane_write = false;
            self.github_write = false;
            self.gitlab_write = false;
            self.jira_write = false;
        }
        self
    }
}

#[derive(Subcommand)]
pub(crate) enum Mode {
    /// Configure a platform and print a reviewed MCP client snippet.
    Init(Box<crate::setup::Init>),
    /// Validate credentials and check API connectivity without publishing.
    #[command(args_override_self = true)]
    Doctor {
        #[command(flatten)]
        connection: Box<McpConnection>,
        /// Validate configuration and token files without sending network requests.
        #[arg(long)]
        offline: bool,
    },
    /// Start the local MCP server using configured task platforms.
    #[command(args_override_self = true)]
    Mcp {
        #[command(flatten)]
        connection: Box<McpConnection>,
        #[command(flatten)]
        writes: WriteOptions,
    },
    /// Read an issue and check its file dependencies.
    GiteaContext {
        #[command(flatten)]
        gitea: Connection,
        issue: String,
    },
    /// Watch selected issues for dependency changes as JSON lines.
    GiteaWatch {
        #[command(flatten)]
        gitea: Connection,
        #[arg(long, default_value = "60", value_parser = clap::value_parser!(u64).range(10..=3600))]
        interval: u64,
        #[arg(required = true, num_args = 1..=32)]
        issues: Vec<String>,
    },
}

impl Connection {
    pub(crate) async fn open(self) -> anyhow::Result<Gitea> {
        Gitea::new(self.gitea_url, read_token(&self.gitea_token_file).await?)
    }
}
