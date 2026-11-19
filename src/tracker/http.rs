//! Bounded, nonredirecting HTTP transport for the REST tracker adapters.

use std::time::Duration;

use anyhow::{Context as _, ensure};
use reqwest::{Client, Method, Url, header};
use serde_json::Value;

enum Auth {
    Header(header::HeaderName, header::HeaderValue),
    Basic { email: String, token: String },
}

pub(crate) struct Http {
    pub web: Url,
    pub api: Url,
    client: Client,
    auth: Auth,
    name: &'static str,
}

impl Http {
    pub fn token(
        name: &'static str,
        base: Url,
        token: String,
        header_name: header::HeaderName,
        bearer: bool,
    ) -> anyhow::Result<Self> {
        ensure!(!token.is_empty(), "{name} token is empty");
        let value = if bearer {
            format!("Bearer {token}")
        } else {
            token
        };
        let mut value = header::HeaderValue::from_str(&value)
            .map_err(|_| anyhow::anyhow!("invalid {name} token"))?;
        value.set_sensitive(true);
        Self::new(name, base, Auth::Header(header_name, value))
    }

    pub fn basic(
        name: &'static str,
        base: Url,
        email: String,
        token: String,
    ) -> anyhow::Result<Self> {
        ensure!(
            crate::validation::simple_text(&email, 254) && email.contains('@'),
            "provide a valid Jira account email"
        );
        ensure!(!token.is_empty(), "Jira token is empty");
        Self::new(name, base, Auth::Basic { email, token })
    }

    fn new(name: &'static str, mut base: Url, auth: Auth) -> anyhow::Result<Self> {
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        ensure!(
            base.host().is_some()
                && (base.scheme() == "https" || (base.scheme() == "http" && loopback))
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "{name} URL must use HTTPS or loopback HTTP without credentials, query or fragment"
        );
        if !base.path().ends_with('/') {
            base.path_segments_mut()
                .expect("validated HTTP URL")
                .push("");
        }
        Ok(Self {
            web: base.clone(),
            api: base,
            auth,
            name,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .user_agent(concat!("mergelane/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }

    pub async fn get(&self, path: &[&str], query: &[(&str, String)]) -> anyhow::Result<Value> {
        self.send(Method::GET, path, query, None).await
    }

    pub async fn post(&self, path: &[&str], body: &Value) -> anyhow::Result<Value> {
        self.send(Method::POST, path, &[], Some(body)).await
    }

    pub async fn put(&self, path: &[&str], body: &Value) -> anyhow::Result<Value> {
        self.send(Method::PUT, path, &[], Some(body)).await
    }

    pub async fn patch(&self, path: &[&str], body: &Value) -> anyhow::Result<Value> {
        self.send(Method::PATCH, path, &[], Some(body)).await
    }

    async fn send(
        &self,
        method: Method,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let mut request = self
            .client
            .request(method, append(self.api.clone(), path))
            .header(header::ACCEPT, "application/json")
            .query(query);
        request = match &self.auth {
            Auth::Header(name, value) => request.header(name, value),
            Auth::Basic { email, token } => request.basic_auth(email, Some(token)),
        };
        if self.name == "GitHub" {
            request = request.header("X-GitHub-Api-Version", "2026-03-10");
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("{} API connection failed", self.name))?;
        ensure!(
            response.status().is_success(),
            "{} API returned HTTP {}",
            self.name,
            response.status()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("{} API response interrupted", self.name))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 1024 * 1024,
                "{} response exceeded 1 MiB",
                self.name
            );
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes)
            .with_context(|| format!("{} returned invalid JSON", self.name))
    }

    pub async fn list(
        &self,
        path: &[&str],
        query: &[(&str, String)],
    ) -> anyhow::Result<Vec<Value>> {
        let mut all = Vec::new();
        for page in 1..=20 {
            let mut query = query.to_vec();
            query.extend([("per_page", "100".to_owned()), ("page", page.to_string())]);
            let value = self.get(path, &query).await?;
            let batch = value.as_array().context("expected an API array")?;
            let last = batch.len() < 100;
            all.extend(batch.iter().cloned());
            if last {
                return Ok(all);
            }
        }
        anyhow::bail!("{} exceeded the supported 20-page limit", self.name)
    }
}

pub(crate) fn append(mut base: Url, path: &[&str]) -> Url {
    base.path_segments_mut()
        .expect("validated HTTP URL")
        .pop_if_empty()
        .extend(path);
    base
}

pub(crate) fn instance_path(base: &Url, input: &str) -> anyhow::Result<String> {
    let url = Url::parse(input).context("invalid issue URL")?;
    ensure!(
        url.origin() == base.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "issue URL must belong to the configured instance without credentials, query or fragment"
    );
    let raw_path = input
        .split_once("://")
        .and_then(|(_, authority)| authority.split_once('/'))
        .map(|(_, path)| path)
        .context("issue URL has no path")?;
    let path = raw_path
        .strip_prefix(base.path().trim_start_matches('/'))
        .context("issue URL is outside the configured installation path")?;
    ensure!(
        !path
            .split('/')
            .any(|part| matches!(part, "." | "..") || part.contains('%') || part.contains('\\')),
        "invalid issue URL path"
    );
    Ok(path.trim_end_matches('/').to_owned())
}

pub(crate) fn project_path(value: &str, nested: bool) -> anyhow::Result<String> {
    let parts: Vec<_> = value.split('/').collect();
    ensure!(
        parts.len() >= 2 && (nested || parts.len() == 2),
        "provide a namespaced project path"
    );
    ensure!(
        value.len() <= 512
            && parts.iter().all(|part| !matches!(*part, "" | "." | "..")
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))),
        "invalid project path"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_stay_with_secure_configured_origins_and_installation_paths() {
        for url in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com?secret=x",
            "https://example.com#fragment",
        ] {
            assert!(
                Http::token(
                    "GitHub",
                    Url::parse(url).unwrap(),
                    "fake".to_owned(),
                    header::AUTHORIZATION,
                    true
                )
                .is_err()
            );
        }
        let base = Url::parse("https://git.example.com/prefix/").unwrap();
        assert_eq!(
            instance_path(&base, "https://git.example.com/prefix/team/web/issues/42").unwrap(),
            "team/web/issues/42"
        );
        for url in [
            "https://elsewhere.example/prefix/team/web/issues/42",
            "https://git.example.com/other/team/web/issues/42",
            "https://git.example.com/prefix/team/../web/issues/42",
            "https://git.example.com/prefix/team/%2e%2e/web/issues/42",
        ] {
            assert!(instance_path(&base, url).is_err(), "{url}");
        }
        for project in ["team/../web", "team/%2e%2e/web", "team//web"] {
            assert!(project_path(project, true).is_err());
        }
    }
}
