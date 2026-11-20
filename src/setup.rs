//! Guided configuration and user-level agent installation.

mod agent;

use std::{
    fs,
    io::{self, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::{
    credentials::{read_token, save_token},
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
    #[command(flatten)]
    platform: PlatformOptions,
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

#[derive(Args, Default)]
struct PlatformOptions {
    #[arg(long, value_enum)]
    backend: Option<Backend>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    token_file: Option<PathBuf>,
    #[arg(long)]
    workspace: Option<String>,
    /// Save a default destination for standalone issue creation.
    #[arg(long)]
    default_project: Option<String>,
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
}

impl Init {
    pub async fn run(self, path: PathBuf) -> anyhow::Result<()> {
        let path = settings::absolute(&path)?;
        let interactive = !self.non_interactive && io::stdin().is_terminal();
        let (backend, mut settings) =
            tokio::task::spawn_blocking(move || self.platform.collect(interactive)).await??;
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
            settings::save_backend(&path, backend, settings, self.replace)
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
        println!(
            "{}",
            client_snippet(self.client, &McpCommand::new(&output_path)?)?
        );
        eprintln!(
            "Check this configuration with: mergelane --config {} doctor",
            output_path.display()
        );
        Ok(())
    }
}

impl PlatformOptions {
    fn collect(self, interactive: bool) -> anyhow::Result<(Backend, BackendSettings)> {
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
            default_project: self.default_project,
            email,
            cloud_id,
            issue_type: self.issue_type,
            link_type: self.link_type,
            write: self.write,
        };
        Ok((backend, settings))
    }
}

pub(crate) async fn install(path: PathBuf) -> anyhow::Result<()> {
    ensure!(
        io::stdin().is_terminal(),
        "run mergelane setup in a terminal"
    );
    let path = settings::absolute(&path)?;
    tokio::task::spawn_blocking(move || {
        let selection = prompt("Agent (codex, claude, cursor, all)", "codex")?;
        let clients = if selection.eq_ignore_ascii_case("all") {
            Client::value_variants().to_vec()
        } else {
            vec![Client::from_str(&selection, true).map_err(anyhow::Error::msg)?]
        };
        let command = McpCommand::new(&path)?;
        let updates = agent::prepare(&clients, &settings::home()?, &command)?;
        if path.try_exists()? {
            settings::load(&path)?;
            eprintln!("Using existing MergeLane settings at {}", path.display());
        } else {
            configure_managed_profile(&path)?;
        }
        updates.apply()?;
        eprintln!("MergeLane MCP and Skill installed. Restart your agent to use them.");
        Ok(())
    })
    .await?
}

fn configure_managed_profile(path: &Path) -> anyhow::Result<()> {
    let token_path = settings::default_path()?
        .with_file_name("credentials")
        .join(format!("{}.token", uuid::Uuid::new_v4()));
    let options = PlatformOptions {
        token_file: Some(token_path),
        ..PlatformOptions::default()
    };
    let (backend, settings) = options.collect(true)?;
    settings.validate(backend)?;
    // Initialize the terminal backend before rendering the token prompt so an
    // immediate paste cannot reach the terminal while echo is still enabled.
    let token = inquire::Password::new("Paste the personal token you created on the platform:")
        .with_display_mode(inquire::PasswordDisplayMode::Hidden)
        .without_confirmation()
        .with_render_config(inquire::ui::RenderConfig::empty())
        .prompt()
        .context("cannot read the personal token from the terminal")?;
    save_managed_profile(path, backend, settings, &token)
}

fn save_managed_profile(
    path: &Path,
    backend: Backend,
    settings: BackendSettings,
    token: &str,
) -> anyhow::Result<()> {
    let token_path = settings.token_file.clone();
    save_token(&token_path, token)?;
    if let Err(error) = settings::save_backend(path, backend, settings, false) {
        fs::remove_file(&token_path).with_context(|| {
            format!(
                "{error:#}; cannot remove unused token file {}",
                token_path.display()
            )
        })?;
        return Err(error);
    }
    Ok(())
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

#[derive(Serialize)]
struct McpCommand {
    command: String,
    args: [String; 3],
}

impl McpCommand {
    fn new(profile: &Path) -> anyhow::Result<Self> {
        let executable = stable_executable(std::env::current_exe()?);
        Ok(Self {
            command: executable
                .to_str()
                .context("executable path must be UTF-8")?
                .to_owned(),
            args: [
                "mcp".into(),
                "--config".into(),
                profile
                    .to_str()
                    .context("config path must be UTF-8")?
                    .into(),
            ],
        })
    }
}

fn client_snippet(client: Client, launch: &McpCommand) -> anyhow::Result<String> {
    match client {
        Client::Codex => Ok(format!(
            "[mcp_servers.mergelane]\ncommand = {}\nargs = {}",
            serde_json::to_string(&launch.command)?,
            serde_json::to_string(&launch.args)?
        )),
        Client::Claude | Client::Cursor => Ok(serde_json::to_string_pretty(
            &serde_json::json!({"mcpServers":{"mergelane": launch}}),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn managed_setup_keeps_the_token_in_its_own_file() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("config.json");
        let token_path = directory.path().join("credentials/test.token");
        let backend = Backend::Github;
        let options = PlatformOptions {
            backend: Some(backend),
            token_file: Some(token_path.clone()),
            ..PlatformOptions::default()
        };
        let (_, configured) = options.collect(false).unwrap();

        save_managed_profile(&profile, backend, configured, "setup-fake-token").unwrap();

        assert!(
            !fs::read_to_string(&profile)
                .unwrap()
                .contains("setup-fake-token")
        );
        let saved = settings::load(&profile).unwrap();
        assert_eq!(saved.backends[&backend].token_file, token_path);
        assert_eq!(read_token(&token_path).await.unwrap(), "setup-fake-token");
    }

    #[test]
    fn failed_profile_save_removes_the_new_token() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("config.json");
        fs::write(&profile, "invalid profile").unwrap();
        let token_path = directory.path().join("credentials/test.token");
        let backend = Backend::Github;
        let (_, configured) = PlatformOptions {
            backend: Some(backend),
            token_file: Some(token_path.clone()),
            ..PlatformOptions::default()
        }
        .collect(false)
        .unwrap();

        assert!(save_managed_profile(&profile, backend, configured, "setup-fake-token").is_err());
        assert!(!token_path.exists());
        assert_eq!(fs::read_to_string(&profile).unwrap(), "invalid profile");
    }
}
