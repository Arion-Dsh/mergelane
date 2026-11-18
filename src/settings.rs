//! Versioned personal settings contain credential paths, never credential contents.

mod storage;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use clap::ValueEnum;
use reqwest::Url;
use serde::{Deserialize, Serialize};

pub(crate) use storage::{load, save_backend};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Backend {
    Gitea,
    Plane,
    Github,
    Gitlab,
    Jira,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gitea => "gitea",
            Self::Plane => "plane",
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Jira => "jira",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Self::Github => "https://github.com",
            Self::Gitlab => "https://gitlab.com",
            _ => "",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub version: u32,
    pub backends: BTreeMap<Backend, BackendSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            backends: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BackendSettings {
    pub url: String,
    pub token_file: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(default)]
    pub write: bool,
}

impl Settings {
    pub fn arguments(&self, path: &Path) -> anyhow::Result<Vec<OsString>> {
        ensure!(
            self.version == 1,
            "unsupported MergeLane settings version {}",
            self.version
        );
        ensure!(
            !self.backends.is_empty(),
            "settings must configure at least one backend"
        );
        let mut args = Vec::new();
        let parent = path.parent().context("configuration path has no parent")?;
        for (backend, settings) in &self.backends {
            settings.validate(*backend)?;
            args.extend([
                format!("--{}-url", backend.name()).into(),
                settings.url.clone().into(),
            ]);
            let token = expand_home(&settings.token_file)?;
            let token = if token.is_absolute() {
                token
            } else {
                parent.join(token)
            };
            args.extend([
                format!("--{}-token-file", backend.name()).into(),
                token.into_os_string(),
            ]);
            for (field, value) in [
                ("workspace", &settings.workspace),
                ("email", &settings.email),
                ("cloud-id", &settings.cloud_id),
                ("issue-type", &settings.issue_type),
                ("link-type", &settings.link_type),
            ] {
                if let Some(value) = value {
                    args.extend([
                        format!("--{}-{field}", backend.name()).into(),
                        value.clone().into(),
                    ]);
                }
            }
        }
        Ok(args)
    }

    pub fn write_arguments(&self) -> Vec<OsString> {
        self.backends
            .iter()
            .filter(|(_, settings)| settings.write)
            .map(|(backend, _)| OsString::from(format!("--{}-write", backend.name())))
            .collect()
    }
}

impl BackendSettings {
    pub fn validate(&self, backend: Backend) -> anyhow::Result<()> {
        let url = Url::parse(&self.url).context("invalid backend URL in settings")?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        ensure!(
            url.host().is_some()
                && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "backend URL must use HTTPS or loopback HTTP without credentials, query or fragment"
        );
        ensure!(
            !self.token_file.as_os_str().is_empty(),
            "token_file must not be empty"
        );
        ensure!(
            backend == Backend::Plane || self.workspace.is_none(),
            "workspace is only supported for Plane"
        );
        let jira = backend == Backend::Jira;
        ensure!(
            jira || [
                &self.email,
                &self.cloud_id,
                &self.issue_type,
                &self.link_type
            ]
            .iter()
            .all(|value| value.is_none()),
            "Jira settings cannot be used for another backend"
        );
        if jira {
            let email = self
                .email
                .as_deref()
                .context("Jira settings require email")?;
            ensure!(
                crate::validation::simple_text(email, 254) && email.contains('@'),
                "invalid Jira email"
            );
            if let Some(id) = &self.cloud_id {
                uuid::Uuid::parse_str(id).context("invalid Jira Cloud ID")?;
            }
        }
        for value in [&self.workspace, &self.issue_type, &self.link_type]
            .into_iter()
            .flatten()
        {
            ensure!(
                crate::validation::simple_text(value, 255),
                "settings values must be nonempty single-line text"
            );
        }
        Ok(())
    }
}

pub(crate) fn default_path() -> anyhow::Result<PathBuf> {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return absolute(&PathBuf::from(root).join("mergelane/config.json"));
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        return absolute(&PathBuf::from(root).join("mergelane/config.json"));
    }
    Ok(home()?.join(".config/mergelane/config.json"))
}

fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("cannot locate home directory; pass --config explicitly")
}

pub(crate) fn expand_home(path: &Path) -> anyhow::Result<PathBuf> {
    if path == Path::new("~") {
        return home();
    }
    if let Ok(tail) = path.strip_prefix("~") {
        return Ok(home()?.join(tail));
    }
    Ok(path.to_owned())
}

pub(crate) fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    let path = expand_home(path)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
