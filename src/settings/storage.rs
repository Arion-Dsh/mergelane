use std::{
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, ensure};

use super::{Backend, BackendSettings, Settings};

const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

pub(crate) fn load(path: &Path) -> anyhow::Result<Settings> {
    let file =
        fs::File::open(path).with_context(|| format!("cannot open settings {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "settings must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SETTINGS_BYTES,
        "settings exceeded 64 KiB"
    );
    let settings: Settings =
        serde_json::from_slice(&bytes).context("invalid MergeLane settings JSON")?;
    settings.arguments(path)?;
    Ok(settings)
}

/// Hold an OS file lock across read, merge, and atomic replacement.
pub(crate) fn save_backend(
    path: &Path,
    backend: Backend,
    settings: BackendSettings,
    replace: bool,
) -> anyhow::Result<()> {
    settings.validate(backend)?;
    let parent = path.parent().context("settings path has no parent")?;
    fs::create_dir_all(parent).context("cannot create settings directory")?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(path.with_extension("lock"))?;
    lock.try_lock()
        .context("settings are being edited by another process")?;
    let mut profile = if path.try_exists()? {
        ensure!(
            !path.symlink_metadata()?.file_type().is_symlink(),
            "refusing to replace a settings symlink"
        );
        load(path)?
    } else {
        Settings::default()
    };
    ensure!(
        replace || !profile.backends.contains_key(&backend),
        "{} is already configured; pass --replace to replace only this backend",
        backend.name()
    );
    profile.backends.insert(backend, settings);
    let mut bytes = serde_json::to_vec_pretty(&profile)?;
    bytes.push(b'\n');
    ensure!(
        bytes.len() as u64 <= MAX_SETTINGS_BYTES,
        "settings exceeded 64 KiB"
    );
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = parent.join(format!(".mergelane-{}-{unique}.tmp", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path).context("cannot replace settings file")?;
        Ok(())
    })();
    if result.is_err() && temporary.exists() {
        fs::remove_file(&temporary).context("cannot clean temporary settings file")?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(url: &str) -> BackendSettings {
        BackendSettings {
            url: url.to_owned(),
            token_file: "token".into(),
            workspace: None,
            default_project: None,
            email: None,
            cloud_id: None,
            issue_type: None,
            link_type: None,
            write: false,
        }
    }

    #[test]
    fn merge_replace_and_relative_paths_preserve_other_backends() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        save_backend(&path, Backend::Github, backend("https://github.com"), false).unwrap();
        save_backend(&path, Backend::Gitlab, backend("https://gitlab.com"), false).unwrap();
        assert!(
            save_backend(
                &path,
                Backend::Github,
                backend("https://other.example.com"),
                false
            )
            .is_err()
        );
        save_backend(
            &path,
            Backend::Github,
            backend("https://other.example.com"),
            true,
        )
        .unwrap();
        let profile = load(&path).unwrap();
        assert_eq!(profile.backends.len(), 2);
        assert_eq!(profile.backends[&Backend::Gitlab].url, "https://gitlab.com");
        assert_eq!(
            profile.backends[&Backend::Github].url,
            "https://other.example.com"
        );
        assert!(
            profile
                .arguments(&path)
                .unwrap()
                .contains(&dir.path().join("token").into_os_string())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_or_unrecognized_settings_are_not_replaced() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        let original = br#"{"version":2,"backends":{}}"#;
        std::fs::write(&path, original).unwrap();
        assert!(save_backend(&path, Backend::Github, backend("https://github.com"), true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&path, " ".repeat(65537)).unwrap();
        assert!(load(&path).is_err());
    }
}
