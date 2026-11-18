//! MCP execution grants and automatic renewal tied to the owning stdio session.

use std::{collections::HashMap, time::Duration};

use anyhow::{Context as _, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{sync::watch, time::Instant};
use uuid::Uuid;

use super::{
    TaskKey, now_ms,
    store::{LockedTask, Store},
};
use crate::{
    mcp::{object, tool},
    settings::Backend,
};

pub(crate) const CLAIM: &str = "mergelane_claim_task";
pub(crate) const STATUS: &str = "mergelane_task_lease";
pub(crate) const RELEASE: &str = "mergelane_release_task";
pub(crate) const RENEW: &str = "mergelane_renew_task";

const ACTIVITY_WINDOW: Duration = Duration::from_secs(3600);
const MAX_SESSION_LEASES: usize = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub backend: Backend,
    pub issue: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    pub lease_id: Uuid,
}

struct Held {
    task: TaskKey,
    activity: watch::Sender<Instant>,
}

pub(crate) struct Leases {
    ttl: Duration,
    held: HashMap<Uuid, Held>,
}

/// Keeps an authorized operation serialized with changes to the local lease.
pub(crate) struct CheckpointPermit {
    _task: LockedTask,
}

impl Leases {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            held: HashMap::new(),
        }
    }

    pub async fn claim(&mut self, task: TaskKey) -> anyhow::Result<Value> {
        self.held.retain(|_, held| !held.activity.is_closed());
        ensure!(
            self.held.len() < MAX_SESSION_LEASES,
            "release an existing task before claiming more"
        );
        let store = Store::open()?;
        let ttl = self.ttl;
        let record = store
            .with_task(&task, move |locked| locked.claim(ttl))
            .await?;
        let id = record.lease_id;
        let activity = start_renewal(store, task.clone(), id, ttl);
        self.held.insert(id, Held { task, activity });
        Ok(
            json!({"lease_id": id, "expires_at_unix_ms": record.expires_at_ms,
            "renewal_idle_limit_seconds": ACTIVITY_WINDOW.as_secs()}),
        )
    }

    pub async fn status(&self, task: TaskKey) -> anyhow::Result<Value> {
        Store::open()?
            .with_task(&task, |locked| {
                let now = now_ms()?;
                Ok(match locked.read()? {
                    Some(record) if record.active_at(now) => json!({
                        "status": "held", "expires_at_unix_ms": record.expires_at_ms
                    }),
                    _ => json!({"status":"available"}),
                })
            })
            .await
    }

    pub async fn release(&mut self, id: Uuid) -> anyhow::Result<Value> {
        let held = self
            .held
            .get(&id)
            .context("lease was not granted to this session")?;
        Store::open()?
            .with_task(&held.task, move |locked| locked.release(id))
            .await?;
        self.held.remove(&id);
        Ok(json!({"status":"released"}))
    }

    pub async fn renew(&mut self, id: Uuid) -> anyhow::Result<Value> {
        let held = self
            .held
            .get_mut(&id)
            .context("lease was not granted to this session")?;
        let store = Store::open()?;
        let ttl = self.ttl;
        let renewed = store
            .with_task(&held.task, move |locked| locked.renew(id, ttl))
            .await?;
        ensure!(
            renewed,
            "task lease is no longer held by this session; claim again before working"
        );
        if held.activity.is_closed() {
            held.activity = start_renewal(store, held.task.clone(), id, ttl);
        } else {
            held.activity.send_replace(Instant::now());
        }
        Ok(json!({"status":"renewed"}))
    }

    pub async fn checkpoint(
        &self,
        task: TaskKey,
        id: Option<Uuid>,
    ) -> anyhow::Result<CheckpointPermit> {
        let held = if let Some(id) = id {
            let held = self
                .held
                .get(&id)
                .context("lease was not granted to this session")?;
            ensure!(held.task == task, "lease does not belong to this task");
            Some(held)
        } else {
            None
        };
        let permit = Store::open()?
            .with_task(&task, move |locked| {
                match (locked.read()?, id) {
                    (Some(record), Some(id)) => record.require_holder(id, now_ms()?)?,
                    (None, None) => {}
                    _ => anyhow::bail!(
                        "claim this task and pass its current lease_id in the checkpoint"
                    ),
                }
                Ok(CheckpointPermit { _task: locked })
            })
            .await?;
        if let Some(held) = held {
            held.activity.send_replace(Instant::now());
        }
        Ok(permit)
    }
}

fn start_renewal(store: Store, task: TaskKey, id: Uuid, ttl: Duration) -> watch::Sender<Instant> {
    let (activity, receiver) = watch::channel(Instant::now());
    tokio::spawn(keep_alive(store, task, id, ttl, receiver));
    activity
}

async fn keep_alive(
    store: Store,
    task: TaskKey,
    id: Uuid,
    ttl: Duration,
    mut activity: watch::Receiver<Instant>,
) {
    let period = ttl / 3;
    loop {
        let delay = tokio::time::sleep_until(Instant::now() + period);
        tokio::pin!(delay);
        loop {
            let idle_deadline = *activity.borrow_and_update() + ACTIVITY_WINDOW;
            tokio::select! {
                biased;
                changed = activity.changed() => if changed.is_err() { return; },
                () = tokio::time::sleep_until(idle_deadline) => return,
                () = &mut delay => break,
            }
        }
        // A busy task can have a checkpoint in flight. Try again on the next
        // tick. A stale grant cannot renew even if the worker resumes late.
        if let Ok(false) = store
            .with_task(&task, move |locked| locked.renew(id, ttl))
            .await
        {
            return;
        }
    }
}

pub(crate) fn tools(write: bool) -> Vec<Value> {
    let target = object(
        json!({
            "backend": {"type":"string","enum":["gitea","plane","github","gitlab","jira"]},
            "issue": {"type":"string"}
        }),
        &["backend", "issue"],
    );
    let grant = object(
        json!({"lease_id":{"type":"string","format":"uuid"}}),
        &["lease_id"],
    );
    let mut tools = vec![tool(
        STATUS,
        "Check if a task has an active lease on this computer. This does not grant a lease.",
        target.clone(),
        true,
    )];
    if !write {
        return tools;
    }
    tools.extend([
        tool(
            CLAIM,
            "Claim a task locally before editing or implementing it. Stop if the claim fails. Keep the returned lease_id for checkpoints and release. The owning MCP session renews the lease while it is active for this task.",
            target,
            false,
        ),
        tool(
            RENEW,
            "Signal continued task activity for a long execution. Normal renewal is automatic. Call before the renewal idle limit if no checkpoint is needed.",
            grant.clone(),
            false,
        ),
        tool(
            RELEASE,
            "Release this session's local task lease after completion or when pausing. Save any needed checkpoint first.",
            grant,
            false,
        ),
    ]);
    tools
}
