//! Expand reviewed personal settings into ordinary command arguments before Clap validation.

use std::{ffi::OsString, path::PathBuf};

use anyhow::Context as _;
use clap::Parser;

use super::Cli;
use crate::settings;

pub(crate) async fn parse() -> anyhow::Result<Cli> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h" | "--version" | "-V")))
    {
        return Ok(Cli::parse());
    }
    let selection = select(&args)?;
    let Some((index, command)) = selection
        .command
        .filter(|(_, command)| matches!(command.as_str(), "mcp" | "doctor"))
    else {
        return Ok(Cli::parse());
    };
    if selection.disabled {
        return Ok(Cli::parse());
    }
    let explicit = selection.path.is_some();
    let path = match selection.path {
        Some(path) => path,
        None => settings::default_path()?,
    };
    let path = settings::absolute(&path)?;
    if explicit || path.try_exists()? {
        let profile_path = path.clone();
        let profile = tokio::task::spawn_blocking(move || settings::load(&profile_path)).await??;
        let mut defaults = profile.arguments(&path)?;
        if command == "mcp" {
            defaults.extend(profile.write_arguments());
        }
        args.splice(index + 1..index + 1, defaults);
    }
    Ok(Cli::try_parse_from(args).unwrap_or_else(|error| error.exit()))
}

struct Selection {
    path: Option<PathBuf>,
    disabled: bool,
    command: Option<(usize, String)>,
}

fn select(args: &[OsString]) -> anyhow::Result<Selection> {
    let mut selection = Selection {
        path: None,
        disabled: false,
        command: None,
    };
    let mut iter = args.iter().enumerate().skip(1);
    while let Some((index, arg)) = iter.next() {
        if arg == "--config" {
            selection.path = Some(PathBuf::from(
                iter.next().context("--config requires a path")?.1,
            ));
        } else if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--config=")) {
            selection.path = Some(PathBuf::from(path));
        } else if arg == "--no-config" {
            selection.disabled = true;
        } else if selection.command.is_none()
            && matches!(
                arg.to_str(),
                Some("mcp" | "doctor" | "init" | "gitea-context" | "gitea-watch")
            )
        {
            selection.command = Some((index, arg.to_string_lossy().into_owned()));
        }
    }
    Ok(selection)
}
