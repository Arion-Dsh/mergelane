//! One-shot, read-only task snapshots suitable for a local scheduler.

use std::{
    collections::HashSet,
    io::{self, Write as _},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, ensure};
use clap::Args;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{Instant, timeout_at};

use crate::{cli::Connections, gitea::Gitea, plane::Plane, settings::Backend, tracker};

pub(crate) const DEFAULT_MAX_PAGES: u32 = 10;
pub(crate) const MAX_PAGES: u32 = 34;

const fn default_max_pages() -> u32 {
    DEFAULT_MAX_PAGES
}

#[derive(Args, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Filter {
    /// Use one configured platform. By default, read all configured platforms.
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    /// Fetch all open work in this project instead of your assigned work.
    #[arg(long, requires = "backend")]
    project: Option<String>,
    /// Maximum number of search pages per paginated platform.
    #[arg(long, default_value_t = DEFAULT_MAX_PAGES, value_parser = clap::value_parser!(u32).range(1..=i64::from(MAX_PAGES)))]
    #[serde(default = "default_max_pages")]
    max_pages: u32,
}

impl Filter {
    fn validate(&self) -> anyhow::Result<()> {
        if let Some(project) = &self.project {
            ensure!(self.backend.is_some(), "project requires a backend");
            ensure!(
                crate::validation::simple_text(project, 250),
                "invalid project filter"
            );
        }
        ensure!(
            (1..=MAX_PAGES).contains(&self.max_pages),
            "max_pages must be between 1 and {MAX_PAGES}"
        );
        Ok(())
    }
}

pub(crate) struct Sources<'a> {
    pub gitea: Option<&'a Gitea>,
    pub plane: Option<&'a Plane>,
    pub trackers: &'a tracker::Backends,
}

#[derive(Deserialize, Serialize)]
struct Item {
    title: String,
    url: String,
    #[serde(alias = "repository")]
    project: String,
    state: Option<String>,
    assignees: Option<Vec<Assignee>>,
}

#[derive(Deserialize, Serialize)]
struct Assignee {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(alias = "login")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    items: Vec<Item>,
    next_page: Option<u32>,
    next_cursor: Option<String>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    project_errors: Vec<Value>,
}

#[derive(Serialize)]
struct Source {
    backend: Backend,
    items: Vec<Item>,
    truncated: bool,
    errors: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct Report {
    fetched_at_unix: u64,
    scope: &'static str,
    project: Option<String>,
    complete: bool,
    sources: Vec<Source>,
}

pub(crate) async fn run(connection: Connections, filter: Filter) -> anyhow::Result<()> {
    let sources = Sources {
        gitea: connection.gitea.as_ref(),
        plane: connection.plane.as_ref(),
        trackers: &connection.trackers,
    };
    let report = fetch(sources, filter, Duration::from_secs(120)).await?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    stdout.flush()?;
    ensure!(
        report.complete,
        "task snapshot is incomplete; check source errors or narrow the scope"
    );
    Ok(())
}

pub(crate) async fn fetch(
    connection: Sources<'_>,
    filter: Filter,
    time_limit: Duration,
) -> anyhow::Result<Report> {
    filter.validate()?;
    let backends = [
        (Backend::Gitea, connection.gitea.is_some()),
        (Backend::Plane, connection.plane.is_some()),
        (Backend::Github, connection.trackers.github.is_some()),
        (Backend::Gitlab, connection.trackers.gitlab.is_some()),
        (Backend::Jira, connection.trackers.jira.is_some()),
    ];
    let backends: Vec<_> = backends
        .into_iter()
        .filter(|(backend, configured)| *configured && filter.backend.is_none_or(|b| b == *backend))
        .map(|(backend, _)| backend)
        .collect();
    ensure!(
        !backends.is_empty(),
        "the requested pull platform is not configured"
    );
    let fetched_at_unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let deadline = Instant::now() + time_limit;
    // Accounts are independent, so a slow platform cannot consume another platform's time budget.
    let sources = join_all(
        backends
            .into_iter()
            .map(|backend| pull_source(&connection, backend, &filter, deadline)),
    )
    .await;
    let complete = sources
        .iter()
        .all(|source| !source.truncated && source.errors.is_empty());
    Ok(Report {
        fetched_at_unix,
        scope: if filter.project.is_some() {
            "project"
        } else {
            "assigned_to_me"
        },
        project: filter.project,
        complete,
        sources,
    })
}

async fn pull_source(
    connection: &Sources<'_>,
    backend: Backend,
    filter: &Filter,
    deadline: Instant,
) -> Source {
    let mut source = Source {
        backend,
        items: Vec::new(),
        truncated: false,
        errors: Vec::new(),
    };
    let mut page = 1;
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut cursors = HashSet::new();
    for _ in 0..filter.max_pages {
        let next = timeout_at(deadline, query(connection, backend, filter, page, cursor)).await;
        let batch = match next {
            Ok(Ok(batch)) => batch,
            Ok(Err(error)) => {
                source.errors.push(format!("{error:#}"));
                return source;
            }
            Err(_) => {
                source
                    .errors
                    .push("platform did not finish within the pull time limit".to_owned());
                return source;
            }
        };
        source.items.extend(
            batch
                .items
                .into_iter()
                .filter(|item| seen.insert(item.url.clone())),
        );
        source.truncated |= batch.truncated;
        source.errors.extend(
            batch
                .project_errors
                .into_iter()
                .map(|error| error.to_string()),
        );
        match (batch.next_page, batch.next_cursor) {
            (None, None) => return source,
            (Some(next), None) if next > page => {
                page = next;
                cursor = None;
            }
            (None, Some(next)) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next);
            }
            _ => {
                source
                    .errors
                    .push("platform returned invalid or repeated pagination data".to_owned());
                return source;
            }
        }
    }
    source.truncated = true;
    source
}

async fn query(
    connection: &Sources<'_>,
    backend: Backend,
    filter: &Filter,
    page: u32,
    cursor: Option<String>,
) -> anyhow::Result<Page> {
    let project = filter.project.clone();
    let mine = project.is_none();
    let search = tracker::Search {
        query: None,
        project: project.clone(),
        assignee: None,
        mine,
        page,
        cursor,
        open_only: true,
    };
    let value = match backend {
        Backend::Gitea => {
            connection
                .gitea
                .context("Gitea is not configured")?
                .search(crate::gitea::Search {
                    query: None,
                    mine,
                    owner: None,
                    assignee: None,
                    page,
                    project,
                    open_only: true,
                })
                .await?
        }
        Backend::Plane => {
            connection
                .plane
                .context("Plane is not configured")?
                .pull_open(project)
                .await?
        }
        Backend::Github => {
            connection
                .trackers
                .github
                .as_ref()
                .context("GitHub is not configured")?
                .search(search)
                .await?
        }
        Backend::Gitlab => {
            connection
                .trackers
                .gitlab
                .as_ref()
                .context("GitLab is not configured")?
                .search(search)
                .await?
        }
        Backend::Jira => {
            connection
                .trackers
                .jira
                .as_ref()
                .context("Jira is not configured")?
                .search(search)
                .await?
        }
    };
    serde_json::from_value(value).context("invalid platform task list")
}
