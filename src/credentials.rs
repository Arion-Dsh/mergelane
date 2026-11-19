//! File-based credential loading at the process boundary.

use std::{fs, io::Write as _, path::Path};

use anyhow::{Context as _, ensure};
use tokio::io::AsyncReadExt as _;

const MAX_TOKEN_BYTES: usize = 4096;

pub(crate) fn save_token(path: &Path, token: &str) -> anyhow::Result<()> {
    let token = token.trim();
    ensure!(
        !token.is_empty() && token.len() <= MAX_TOKEN_BYTES && !token.chars().any(char::is_control),
        "enter a nonempty single-line token of at most {MAX_TOKEN_BYTES} bytes"
    );
    let directory = path.parent().context("token path has no parent")?;
    fs::create_dir_all(directory).context("cannot create credentials directory")?;
    // Named temporary files default to mode 0600 on Unix.
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(token.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)
        .map_err(|error| error.error)
        .context("cannot save personal token")?;
    Ok(())
}

pub(crate) async fn read_token(path: &Path) -> anyhow::Result<String> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("cannot open token file {}", path.display()))?;
    let meta = file.metadata().await?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_TOKEN_BYTES as u64,
        "token file must be a small regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "token file must have mode 0600 or 0400"
        );
    }
    let mut raw = String::new();
    file.take((MAX_TOKEN_BYTES + 1) as u64)
        .read_to_string(&mut raw)
        .await?;
    ensure!(
        raw.len() <= MAX_TOKEN_BYTES,
        "token file must be at most {MAX_TOKEN_BYTES} bytes"
    );
    Ok(raw.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn variable_length_api_tokens_are_bounded_and_trimmed() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let token = "x".repeat(512);
        tokio::fs::write(file.path(), format!("{token}\n"))
            .await
            .unwrap();
        assert_eq!(read_token(file.path()).await.unwrap(), token);
        tokio::fs::write(file.path(), "x".repeat(MAX_TOKEN_BYTES + 1))
            .await
            .unwrap();
        assert!(read_token(file.path()).await.is_err());
    }
}
