//! Local task identity and expiring ownership, shared by independent MCP processes.

pub(crate) mod mcp;
mod store;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct TaskKey(String);

impl TaskKey {
    pub(crate) fn new(backend: &str, instance: &Url, id: &str) -> anyhow::Result<Self> {
        ensure!(!id.is_empty(), "task has no stable ID");
        let instance = instance.as_str().trim_end_matches('/');
        let bytes = serde_json::to_vec(&(backend, instance, id))?;
        Ok(Self(format!("{:x}", Sha256::digest(bytes))))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    lease_id: Uuid,
    expires_at_ms: u64,
}

impl Record {
    fn active_at(&self, now: u64) -> bool {
        self.expires_at_ms > now
    }

    fn require_holder(&self, id: Uuid, now: u64) -> anyhow::Result<()> {
        ensure!(
            self.lease_id == id && self.active_at(now),
            "task lease is no longer held by this session; claim again before working"
        );
        Ok(())
    }
}

fn now_ms() -> anyhow::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()
        .context("local clock is out of range")
}

fn expiry(now: u64, ttl: Duration) -> anyhow::Result<u64> {
    now.checked_add(ttl.as_millis().try_into()?)
        .context("task lease expiry is out of range")
}
