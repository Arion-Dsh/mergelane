#![forbid(unsafe_code)]

mod cli;
mod credentials;
mod doctor;
mod gitea;
mod github;
mod gitlab;
mod jira;
mod lease;
mod mcp;
mod plane;
mod settings;
mod setup;
mod tracker;
mod validation;

use std::time::Duration;

use cli::Mode;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::parse().await?;
    match cli.mode {
        Mode::Init(args) => {
            let path = match cli.config {
                Some(path) => path,
                None => settings::default_path()?,
            };
            args.run(path).await?;
        }
        Mode::Doctor {
            connection,
            offline,
        } => doctor::run(*connection, offline).await?,
        Mode::Mcp {
            connection,
            writes,
            lease_seconds,
        } => {
            let writes = writes.effective();
            let connection = connection.open(&writes).await?;
            mcp::run(connection, writes, Duration::from_secs(lease_seconds)).await?
        }
        Mode::GiteaContext { gitea, issue } => {
            println!("{}", gitea.open().await?.context(&issue).await?);
        }
        Mode::GiteaWatch {
            gitea,
            interval,
            issues,
        } => {
            let gitea = gitea.open().await?;
            tokio::select! {
                result = gitea.watch(&issues, Duration::from_secs(interval)) => result?,
                signal = tokio::signal::ctrl_c() => signal?,
            }
        }
    }
    Ok(())
}
