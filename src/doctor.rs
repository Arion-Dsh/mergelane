//! Read-only diagnostics report configuration and API connectivity without publishing.

use serde::Serialize;

use crate::{
    cli::{Connections, McpConnection, WriteOptions},
    tracker::Api,
};

#[derive(Serialize)]
struct Check {
    backend: &'static str,
    status: &'static str,
    message: String,
}

pub(crate) async fn run(connection: McpConnection, offline: bool) -> anyhow::Result<()> {
    let connection = match connection.open(&WriteOptions::default()).await {
        Ok(connection) => connection,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"ok":false,"checks":[{"backend":"configuration","status":"error","message":format!("{error:#}")}]})
            );
            anyhow::bail!("configuration check failed");
        }
    };
    let checks = inspect(connection, offline).await;
    let ok = checks.iter().all(|check| check.status == "ok");
    println!(
        "{}",
        serde_json::json!({"ok":ok,"offline":offline,"checks":checks})
    );
    anyhow::ensure!(ok, "one or more connection checks failed");
    Ok(())
}

async fn inspect(connection: Connections, offline: bool) -> Vec<Check> {
    let mut checks = Vec::new();
    if let Some(gitea) = connection.gitea {
        checks.push(check(
            "gitea",
            if offline {
                Ok(())
            } else {
                gitea.check_connection().await
            },
            offline,
        ));
    }
    if let Some(plane) = connection.plane {
        checks.push(check(
            "plane",
            if offline {
                Ok(())
            } else {
                plane.check_connection().await
            },
            offline,
        ));
    }
    if let Some(session) = connection.trackers.github {
        checks.push(inspect_tracker(session, offline).await);
    }
    if let Some(session) = connection.trackers.gitlab {
        checks.push(inspect_tracker(session, offline).await);
    }
    if let Some(session) = connection.trackers.jira {
        checks.push(inspect_tracker(session, offline).await);
    }
    checks
}

async fn inspect_tracker<A: Api>(session: crate::tracker::Session<A>, offline: bool) -> Check {
    let name = session.name();
    check(
        name,
        if offline {
            Ok(())
        } else {
            session.check_connection().await
        },
        offline,
    )
}

fn check(backend: &'static str, result: anyhow::Result<()>, offline: bool) -> Check {
    match result {
        Ok(()) => Check { backend, status: "ok", message: if offline { "configuration and token file validated; network check skipped" } else { "authenticated API connection succeeded; project and write permissions were not tested" }.to_owned() },
        Err(error) => Check { backend, status: "error", message: format!("{error:#}") },
    }
}
