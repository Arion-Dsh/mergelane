//! Guided setup writes MergeLane settings and prints a client snippet for review.

use std::{
    io::{self, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use clap::{Args, ValueEnum};

use crate::{
    credentials::read_token,
    settings::{self, Backend, BackendSettings},
};

#[derive(Clone, Copy, Default, ValueEnum)]
enum Client {
    #[default]
    Codex,
    Claude,
    Cursor,
}

#[derive(Args)]
pub(crate) struct Init {
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    token_file: Option<PathBuf>,
    #[arg(long)]
    workspace: Option<String>,
    #[arg(long)]
    email: Option<String>,
    #[arg(long)]
    cloud_id: Option<String>,
    #[arg(long)]
    issue_type: Option<String>,
    #[arg(long)]
    link_type: Option<String>,
    /// Include this backend's write tools; publishing still requires user approval.
    #[arg(long)]
    write: bool,
    /// Replace only the selected backend, keeping other backends intact.
    #[arg(long)]
    replace: bool,
    /// Require all necessary arguments instead of prompting.
    #[arg(long)]
    non_interactive: bool,
    /// Print configuration for the selected client without editing its files.
    #[arg(long, value_enum, default_value = "codex")]
    client: Client,
}

impl Init {
    pub async fn run(self, path: PathBuf) -> anyhow::Result<()> {
        let path = settings::absolute(&path)?;
        let (backend, mut settings, client, replace) =
            tokio::task::spawn_blocking(move || self.collect()).await??;
        settings.token_file = tokio::fs::canonicalize(settings::expand_home(&settings.token_file)?)
            .await
            .context("cannot resolve token file; create it outside the repository first")?;
        ensure!(
            !read_token(&settings.token_file).await?.is_empty(),
            "token file is empty"
        );
        settings.validate(backend)?;
        let output_path = path.clone();
        tokio::task::spawn_blocking(move || {
            settings::save_backend(&path, backend, settings, replace)
        })
        .await??;
        eprintln!(
            "Saved {} settings to {}",
            backend.name(),
            output_path.display()
        );
        eprintln!(
            "Merge the following snippet into your MCP client configuration. No client files were changed."
        );
        println!("{}", client_snippet(client, &output_path)?);
        eprintln!(
            "Check this configuration with: mergelane --config {} doctor",
            output_path.display()
        );
        Ok(())
    }

    fn collect(self) -> anyhow::Result<(Backend, BackendSettings, Client, bool)> {
        let interactive = !self.non_interactive && io::stdin().is_terminal();
        let backend = match self.backend {
            Some(backend) => backend,
            None => {
                ensure!(interactive, "pass --backend gitea|plane|github|gitlab|jira");
                let input = prompt("Platform: gitea, plane, github, gitlab, jira", "github")?;
                Backend::from_str(&input, true)
                    .map_err(|_| anyhow::anyhow!("unsupported platform"))?
            }
        };
        let url = match self.url {
            Some(url) => url,
            None if interactive => prompt("Instance URL", backend.default_url())?,
            None if !backend.default_url().is_empty() => backend.default_url().to_owned(),
            None => anyhow::bail!("pass --url for {}", backend.name()),
        };
        let token_file = match self.token_file {
            Some(path) => path,
            None if interactive => {
                PathBuf::from(prompt("Token file path (token stays in that file)", "")?)
            }
            None => {
                anyhow::bail!("pass --token-file; token values must not be supplied as arguments")
            }
        };
        let email = match (backend, self.email) {
            (Backend::Jira, None) if interactive => Some(prompt("Jira account email", "")?),
            (_, email) => email,
        };
        let workspace = match (backend, self.workspace) {
            (Backend::Plane, None) if interactive => {
                optional_prompt("Plane workspace slug (optional)")?
            }
            (_, workspace) => workspace,
        };
        let cloud_id = match (backend, self.cloud_id) {
            (Backend::Jira, None) if interactive => {
                optional_prompt("Jira Cloud ID for scoped token (optional)")?
            }
            (_, cloud_id) => cloud_id,
        };
        let settings = BackendSettings {
            url,
            token_file,
            workspace,
            email,
            cloud_id,
            issue_type: self.issue_type,
            link_type: self.link_type,
            write: self.write,
        };
        Ok((backend, settings, self.client, self.replace))
    }
}

fn prompt(label: &str, default: &str) -> anyhow::Result<String> {
    if default.is_empty() {
        eprint!("{label}: ");
    } else {
        eprint!("{label} [{default}]: ");
    }
    io::stderr().flush()?;
    let mut line = String::new();
    ensure!(io::stdin().read_line(&mut line)? != 0, "setup input closed");
    let line = line.trim();
    Ok(if line.is_empty() {
        default.to_owned()
    } else {
        line.to_owned()
    })
}

fn optional_prompt(label: &str) -> anyhow::Result<Option<String>> {
    let value = prompt(label, "")?;
    Ok((!value.is_empty()).then_some(value))
}

fn client_snippet(client: Client, path: &Path) -> anyhow::Result<String> {
    let executable = stable_executable(std::env::current_exe()?);
    let args = [
        "mcp",
        "--config",
        path.to_str()
            .context("config path must be UTF-8 for client snippets")?,
    ];
    match client {
        Client::Codex => Ok(format!(
            "[mcp_servers.mergelane]\ncommand = {}\nargs = {}",
            serde_json::to_string(&executable)?,
            serde_json::to_string(&args)?
        )),
        Client::Claude | Client::Cursor => Ok(serde_json::to_string_pretty(
            &serde_json::json!({"mcpServers":{"mergelane":{"command":executable,"args":args}}}),
        )?),
    }
}

/// Homebrew's stable opt symlink survives replacement of a versioned Cellar directory.
fn stable_executable(executable: PathBuf) -> PathBuf {
    let Some(cellar) = executable
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Cellar"))
    else {
        return executable;
    };
    let Some(prefix) = cellar.parent() else {
        return executable;
    };
    let stable = prefix.join("opt/mergelane/bin/mergelane");
    if stable.is_file() { stable } else { executable }
}
