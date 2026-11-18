//! A stable lock file serializes each task's atomic record replacement.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, ensure};
use uuid::Uuid;

use super::{Record, TaskKey, expiry, now_ms};

const MAX_RECORD_BYTES: u64 = 1024;

#[derive(Clone)]
pub(super) struct Store {
    root: PathBuf,
}

/// Retain the lock during a checkpoint to serialize it with local takeover.
pub(super) struct LockedTask {
    path: PathBuf,
    _lock: File,
}

impl Store {
    pub fn open() -> anyhow::Result<Self> {
        // Independent of --config, current directory and the client's environment overrides
        // for configuration. All clients of this OS user must use the same local directory.
        let root = crate::settings::home()?.join(".mergelane/leases-v1");
        Ok(Self { root })
    }

    pub async fn with_task<T: Send + 'static>(
        &self,
        key: &TaskKey,
        operation: impl FnOnce(LockedTask) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let root = self.root.clone();
        let key = key.clone();
        tokio::task::spawn_blocking(move || operation(LockedTask::open(&root, &key)?)).await?
    }
}

impl LockedTask {
    fn open(root: &Path, key: &TaskKey) -> anyhow::Result<Self> {
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            directory.mode(0o700);
        }
        directory
            .create(root)
            .context("cannot create local task lease directory")?;
        let path = root.join(&key.0);
        let lock = private_file()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        match lock.try_lock() {
            Ok(()) => Ok(Self { path, _lock: lock }),
            Err(TryLockError::WouldBlock) => {
                anyhow::bail!("another session is operating on this task; retry shortly")
            }
            Err(TryLockError::Error(error)) => Err(error).context("cannot lock local task lease"),
        }
    }

    pub fn read(&self) -> anyhow::Result<Option<Record>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("cannot open local task lease"),
        };
        ensure!(
            file.metadata()?.is_file(),
            "task lease must be a regular file"
        );
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_RECORD_BYTES,
            "task lease too large"
        );
        let record = serde_json::from_slice(&bytes).context("invalid local task lease")?;
        Ok(Some(record))
    }

    pub fn write(&self, record: &Record) -> anyhow::Result<()> {
        let temporary = self.path.with_extension("tmp");
        // An interrupted write can leave this file behind. Remove it while
        // holding the task lock and create a fresh file without following links.
        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot remove incomplete local task lease"),
        }
        let mut file = private_file()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(record)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &self.path).context("cannot save local task lease")
    }

    pub fn claim(&self, ttl: Duration) -> anyhow::Result<Record> {
        let record = self.read()?;
        let now = now_ms()?;
        if let Some(record) = record.filter(|record| record.active_at(now)) {
            anyhow::bail!(
                "task is held by another local execution (lease expires in {}s unless renewed)",
                (record.expires_at_ms - now).div_ceil(1000)
            );
        }
        let record = Record {
            lease_id: Uuid::new_v4(),
            expires_at_ms: expiry(now, ttl)?,
        };
        self.write(&record)?;
        Ok(record)
    }

    pub fn renew(&self, id: Uuid, ttl: Duration) -> anyhow::Result<bool> {
        let mut record = self.read()?.context("task lease is missing")?;
        let now = now_ms()?;
        if record.lease_id != id || !record.active_at(now) {
            return Ok(false);
        }
        record.expires_at_ms = expiry(now, ttl)?;
        self.write(&record)?;
        Ok(true)
    }

    pub fn release(&self, id: Uuid) -> anyhow::Result<()> {
        let mut record = self.read()?.context("task lease is missing")?;
        record.require_holder(id, now_ms()?)?;
        record.expires_at_ms = 0;
        self.write(&record)
    }
}

fn private_file() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_grant_cannot_renew_or_release_a_new_grant() {
        let root = tempfile::tempdir().unwrap();
        let key = TaskKey("issue".into());
        let task = LockedTask::open(root.path(), &key).unwrap();
        let expired = Record {
            lease_id: Uuid::new_v4(),
            expires_at_ms: now_ms().unwrap() - 1,
        };
        task.write(&expired).unwrap();
        let ttl = Duration::from_secs(120);
        assert!(!task.renew(expired.lease_id, ttl).unwrap());
        assert!(task.release(expired.lease_id).is_err());

        let takeover = task.claim(ttl).unwrap();
        assert!(!task.renew(expired.lease_id, ttl).unwrap());
        assert!(task.release(expired.lease_id).is_err());
        assert_eq!(task.read().unwrap().unwrap().lease_id, takeover.lease_id);
        takeover
            .require_holder(takeover.lease_id, now_ms().unwrap())
            .unwrap();
    }
}
