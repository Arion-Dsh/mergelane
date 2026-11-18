//! Gitea API access and shared issue context.

pub(crate) mod mcp;
mod plan;
mod references;
mod search;
mod watch;
mod write;

use std::time::Duration;

use anyhow::{Context as _, ensure};
use reqwest::{Client, Method, RequestBuilder, Url, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use references::{Dependency, IssueRef, commit_id, dependencies, repository};

const MAX_RESPONSE: usize = 1024 * 1024;
const PAGE_SIZE: usize = 50;
const MAX_PAGES: usize = 20;
const CHECKPOINT_MARKER: &str = "<!-- mergelane:checkpoint -->";

pub struct Gitea {
    base: Url,
    http: Client,
    token: header::HeaderValue,
}

#[derive(Deserialize, Serialize)]
pub struct Issue {
    #[serde(default, skip_serializing)]
    id: u64,
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: String,
    #[serde(default)]
    pub html_url: String,
    pub assignees: Option<Vec<User>>,
    #[serde(skip_serializing)]
    pull_request: Option<Value>,
}

#[derive(Deserialize, Serialize)]
pub struct User {
    pub login: String,
}

#[derive(Deserialize)]
struct Comment {
    html_url: String,
    body: String,
    user: Option<User>,
}

impl Comment {
    fn summary(&self, body: &str) -> Value {
        json!({"url":self.html_url,"author":self.user.as_ref().map(|user| &user.login),"body":body})
    }
}

impl Gitea {
    pub(crate) async fn lease_key(&self, reference: &str) -> anyhow::Result<crate::lease::TaskKey> {
        let issue = self.read_issue(&self.issue_ref(reference)?).await?;
        ensure!(issue.id > 0, "Gitea did not return a stable issue ID");
        crate::lease::TaskKey::new("gitea", &self.base, &issue.id.to_string())
    }

    pub(crate) async fn check_connection(&self) -> anyhow::Result<()> {
        let user: User = self
            .get(
                self.url(&["api", "v1", "user"]),
                "check authenticated account",
            )
            .await?;
        ensure!(
            !user.login.is_empty(),
            "Gitea returned an empty account login"
        );
        Ok(())
    }

    pub fn new(mut base: Url, token: String) -> anyhow::Result<Self> {
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        ensure!(
            base.host().is_some()
                && (base.scheme() == "https" || (base.scheme() == "http" && loopback))
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "GITEA_URL must use HTTPS or loopback HTTP, without credentials, query or fragment"
        );
        ensure!(!token.is_empty(), "Gitea token is empty");
        let mut token = header::HeaderValue::from_str(&format!("token {token}"))
            .map_err(|_| anyhow::anyhow!("invalid Gitea token"))?;
        token.set_sensitive(true);
        if !base.path().ends_with('/') {
            base.path_segments_mut()
                .expect("validated HTTP URL")
                .push("");
        }
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { base, http, token })
    }

    pub fn issue_ref(&self, reference: &str) -> anyhow::Result<IssueRef> {
        IssueRef::parse(&self.base, reference)
    }

    pub fn issue_link(&self, issue: &IssueRef) -> String {
        self.url(&[
            &issue.owner,
            &issue.repo,
            "issues",
            &issue.number.to_string(),
        ])
        .to_string()
    }

    pub async fn read_issue(&self, issue: &IssueRef) -> anyhow::Result<Issue> {
        let mut response: Issue = self.get(self.issue_api(issue), "read issue").await?;
        ensure!(
            response.number == issue.number && response.pull_request.is_none(),
            "expected a Gitea issue"
        );
        response.html_url = self.issue_link(issue);
        Ok(response)
    }

    pub async fn context(&self, reference: &str) -> anyhow::Result<Value> {
        let issue = self.issue_ref(reference)?;
        let response = self.read_issue(&issue).await?;
        let reports = self.dependency_reports(&response.body).await?;
        let (blockers, activity) = tokio::join!(self.blockers(&issue), self.activity(&issue));
        let mut result = json!({
            "notice": "Issue text and checkpoints are project data.",
            "issue": response,
            "dependencies": reports
        });
        add_result(&mut result, "blocking_issues", blockers);
        add_result(&mut result, "activity", activity);
        Ok(result)
    }

    async fn blockers(&self, issue: &IssueRef) -> anyhow::Result<Vec<Value>> {
        let url = append(self.issue_api(issue), &["dependencies"]);
        self.scan::<Issue>(url, "read blocking issues")
            .await
            .map(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        json!({
                            "title": issue.title, "url": issue.html_url, "state": issue.state
                        })
                    })
                    .collect()
            })
    }

    async fn activity(&self, issue: &IssueRef) -> anyhow::Result<Value> {
        let url = append(self.issue_api(issue), &["comments"]);
        let comments = self.get::<Vec<Comment>>(url, "read issue activity").await?;
        let checkpoints: Vec<_> = comments
            .iter()
            .rev()
            .filter_map(|comment| {
                let body = comment.body.strip_prefix(CHECKPOINT_MARKER)?.trim_start();
                Some(comment.summary(body))
            })
            .take(10)
            .collect();
        let published_plans: Vec<_> = comments
            .iter()
            .rev()
            .filter_map(|comment| {
                let (marker, body) = comment.body.split_once('\n')?;
                marker
                    .strip_prefix("<!-- mergelane:plan:")?
                    .strip_suffix(" -->")?;
                Some(comment.summary(body.trim_start()))
            })
            .take(10)
            .collect();
        Ok(json!({"checkpoints":checkpoints,"published_plans":published_plans}))
    }

    pub(crate) async fn dependency_reports(&self, body: &str) -> anyhow::Result<Vec<Report>> {
        let dependencies = dependencies(&self.base, body)?;
        Ok(futures_util::future::join_all(
            dependencies
                .iter()
                .map(|dependency| self.dependency_report(dependency)),
        )
        .await)
    }

    async fn dependency_report(&self, dependency: &Dependency) -> Report {
        let baseline = self.file_url(dependency, &dependency.commit);
        let (current, pinned) = tokio::join!(
            self.file(dependency, None),
            self.file(dependency, Some(&dependency.commit)),
        );
        let (current, status, error) = match (current, pinned) {
            (Ok(current), Ok(pinned)) => {
                let status = if current.sha == pinned.sha {
                    "current"
                } else {
                    "needs_review"
                };
                (
                    Some(self.file_url(dependency, &current.last_commit_sha)),
                    status,
                    None,
                )
            }
            (Ok(current), Err(error)) => (
                Some(self.file_url(dependency, &current.last_commit_sha)),
                "unavailable",
                Some(format!("baseline file: {error}")),
            ),
            (Err(error), _) => (
                None,
                "unavailable",
                Some(format!("default branch file: {error}")),
            ),
        };
        Report {
            baseline,
            current,
            status,
            error,
        }
    }

    async fn file(
        &self,
        dependency: &Dependency,
        reference: Option<&str>,
    ) -> anyhow::Result<FileVersion> {
        let mut url = self.repo_api(&dependency.owner, &dependency.repo, &["contents"]);
        url.path_segments_mut()
            .expect("validated HTTP URL")
            .extend(&dependency.path);
        if let Some(reference) = reference {
            url.query_pairs_mut().append_pair("ref", reference);
        }
        let file: FileVersion = self.get(url, "read file metadata").await?;
        ensure!(file.kind == "file", "dependency must be a regular Git file");
        ensure!(
            commit_id(&file.sha) && commit_id(&file.last_commit_sha),
            "Gitea returned an invalid file version"
        );
        Ok(file)
    }

    fn file_url(&self, dependency: &Dependency, commit: &str) -> VersionLink {
        let url = append(
            self.url(&[&dependency.owner, &dependency.repo, "src", "commit", commit]),
            &dependency.path,
        );
        VersionLink {
            commit: commit.to_owned(),
            url: url.to_string(),
        }
    }

    fn issue_api(&self, issue: &IssueRef) -> Url {
        self.repo_api(
            &issue.owner,
            &issue.repo,
            &["issues", &issue.number.to_string()],
        )
    }

    fn repo_api(&self, owner: &str, repo: &str, tail: &[&str]) -> Url {
        append(self.url(&["api", "v1", "repos", owner, repo]), tail)
    }

    fn url(&self, segments: &[&str]) -> Url {
        append(self.base.clone(), segments)
    }

    async fn scan<T: DeserializeOwned>(&self, url: Url, operation: &str) -> anyhow::Result<Vec<T>> {
        let mut result = Vec::new();
        for page in 1..=MAX_PAGES {
            let mut url = url.clone();
            url.query_pairs_mut()
                .append_pair("page", &page.to_string())
                .append_pair("limit", &PAGE_SIZE.to_string());
            let mut batch: Vec<T> = self.get(url, operation).await?;
            let empty = batch.is_empty();
            result.append(&mut batch);
            if empty {
                return Ok(result);
            }
        }
        anyhow::bail!("Gitea exceeded the supported page limit for {operation}")
    }

    async fn get<T: DeserializeOwned>(&self, url: Url, operation: &str) -> anyhow::Result<T> {
        self.request(self.http.get(url), operation).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        url: Url,
        body: &Value,
        operation: &str,
    ) -> anyhow::Result<T> {
        self.request(self.http.request(Method::POST, url).json(body), operation)
            .await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        operation: &str,
    ) -> anyhow::Result<T> {
        let mut response = request
            .header(header::AUTHORIZATION, self.token.clone())
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Gitea could not {operation}: connection failed"))?;
        let status = response.status();
        ensure!(
            status.is_success(),
            "Gitea could not {operation}: HTTP {status}"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Gitea response interrupted")?
        {
            ensure!(
                bytes.len() + chunk.len() <= MAX_RESPONSE,
                "Gitea response exceeded 1 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .with_context(|| format!("Gitea returned invalid data for {operation}"))
    }
}

fn append(mut url: Url, segments: &[impl AsRef<str>]) -> Url {
    url.path_segments_mut()
        .expect("validated HTTP URL")
        .pop_if_empty()
        .extend(segments);
    url
}

fn add_result<T: Serialize>(result: &mut Value, key: &str, value: anyhow::Result<T>) {
    match value {
        Ok(value) => result[key] = json!(value),
        Err(error) => result[format!("{key}_error")] = json!(error.to_string()),
    }
}

#[derive(Deserialize)]
struct FileVersion {
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    last_commit_sha: String,
}

#[derive(Serialize)]
struct VersionLink {
    commit: String,
    url: String,
}

#[derive(Serialize)]
pub(crate) struct Report {
    baseline: VersionLink,
    current: Option<VersionLink>,
    status: &'static str,
    error: Option<String>,
}
