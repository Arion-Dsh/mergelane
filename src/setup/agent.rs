use std::{
    fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde_json::{Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, value};

use super::{Client, McpCommand};

const SKILL: &str = include_str!("../../skills/mergelane/SKILL.md");
const SKILL_PATH: &str = "skills/mergelane/SKILL.md";
const MAX_CLIENT_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct Installation(Vec<FileUpdate>);

pub(super) fn prepare(
    clients: &[Client],
    home: &Path,
    launch: &McpCommand,
) -> anyhow::Result<Installation> {
    let mut updates = Vec::new();
    for &client in clients {
        let (config_path, skill_path) = client_paths(client, home);
        updates.push(FileUpdate::prepare(skill_path, |_| Ok(SKILL.to_owned()))?);
        updates.push(FileUpdate::prepare(config_path, |source| match client {
            Client::Codex => merge_toml(source, launch),
            Client::Claude | Client::Cursor => merge_json(source, launch),
        })?);
    }
    Ok(Installation(updates))
}

impl Installation {
    pub(super) fn apply(self) -> anyhow::Result<()> {
        for update in self.0 {
            update
                .apply()
                .with_context(|| format!("cannot update {}", update.path.display()))?;
        }
        Ok(())
    }
}

fn client_paths(client: Client, home: &Path) -> (PathBuf, PathBuf) {
    let shared_skill = home.join(".agents").join(SKILL_PATH);
    match client {
        Client::Codex => (
            config_directory("CODEX_HOME")
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            shared_skill,
        ),
        Client::Claude => match config_directory("CLAUDE_CONFIG_DIR") {
            Some(directory) => (directory.join(".claude.json"), directory.join(SKILL_PATH)),
            None => (
                home.join(".claude.json"),
                home.join(".claude").join(SKILL_PATH),
            ),
        },
        Client::Cursor => (home.join(".cursor/mcp.json"), shared_skill),
    }
}

fn config_directory(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn merge_toml(source: &str, launch: &McpCommand) -> anyhow::Result<String> {
    let mut document = source
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("invalid Codex TOML"))?;
    let servers = table(document.as_table_mut(), "mcp_servers")?;
    let server = table(servers, "mergelane")?;
    ensure!(
        server.get("url").is_none(),
        "MergeLane is already configured as a remote server"
    );
    server.insert("command", value(&launch.command));
    server.entry("args").or_insert_with(|| {
        let args = launch.args.iter().map(String::as_str).collect::<Array>();
        value(args)
    });
    Ok(document.to_string())
}

fn table<'a>(parent: &'a mut dyn TableLike, name: &str) -> anyhow::Result<&'a mut dyn TableLike> {
    parent
        .entry(name)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut()
        .with_context(|| format!("expected a TOML table for {name}"))
}

fn merge_json(source: &str, launch: &McpCommand) -> anyhow::Result<String> {
    let mut document: Value = if source.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(source).map_err(|_| anyhow::anyhow!("invalid MCP JSON"))?
    };
    let servers = object(&mut document, "mcpServers")?;
    let server = object(servers, "mergelane")?;
    let server = server.as_object_mut().context("invalid MergeLane entry")?;
    ensure!(
        !server.contains_key("url") && server.get("type").is_none_or(|kind| kind == "stdio"),
        "MergeLane is already configured with a different transport"
    );
    server.insert("command".to_owned(), json!(launch.command));
    server.entry("type").or_insert_with(|| json!("stdio"));
    server.entry("args").or_insert_with(|| json!(launch.args));
    Ok(serde_json::to_string_pretty(&document)? + "\n")
}

fn object<'a>(parent: &'a mut Value, name: &str) -> anyhow::Result<&'a mut Value> {
    let parent = parent
        .as_object_mut()
        .context("expected an MCP JSON object")?;
    let object = parent.entry(name).or_insert_with(|| json!({}));
    ensure!(object.is_object(), "expected a JSON object for {name}");
    Ok(object)
}

struct FileUpdate {
    path: PathBuf,
    previous: Option<String>,
    content: String,
}

impl FileUpdate {
    fn prepare(
        path: PathBuf,
        merge: impl FnOnce(&str) -> anyhow::Result<String>,
    ) -> anyhow::Result<Self> {
        let path = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_symlink() => path.canonicalize()?,
            Ok(_) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => path,
            Err(error) => return Err(error.into()),
        };
        let previous = read(&path)?;
        let source = previous
            .as_deref()
            .unwrap_or("")
            .trim_start_matches('\u{feff}');
        let content =
            merge(source).with_context(|| format!("cannot prepare {}", path.display()))?;
        Ok(Self {
            path,
            previous,
            content,
        })
    }

    fn apply(&self) -> anyhow::Result<()> {
        let current = read(&self.path)?;
        if current.as_deref() == Some(&self.content) {
            return Ok(());
        }
        ensure!(
            current == self.previous,
            "file changed during setup; run setup again"
        );
        let parent = self.path.parent().context("missing client directory")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(self.content.as_bytes())?;
        file.as_file().sync_all()?;
        ensure!(
            read(&self.path)? == self.previous,
            "file changed during setup; run setup again"
        );
        file.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(metadata.is_file(), "expected a regular client file"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut text = String::new();
    fs::File::open(path)?
        .take(MAX_CLIENT_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    ensure!(
        text.len() as u64 <= MAX_CLIENT_FILE_BYTES,
        "client file exceeds 16 MiB"
    );
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> McpCommand {
        McpCommand {
            command: "/test directory/mergelane".into(),
            args: ["mcp".into(), "--config".into(), "/profile.json".into()],
        }
    }

    #[test]
    fn codex_merge_keeps_existing_comments_and_server_options() {
        let source = "# preferences\nmodel = 'example'\n\n[mcp_servers.other]\ncommand = 'other-server'\n\n[mcp_servers.mergelane]\ncommand = 'old'\nargs = ['mcp', '--read-only']\n";
        let output = merge_toml(source, &launch()).unwrap();
        let parsed: DocumentMut = output.parse().unwrap();
        assert!(output.starts_with("# preferences\nmodel = 'example'\n"));
        assert_eq!(
            parsed["mcp_servers"]["other"]["command"].as_str(),
            Some("other-server")
        );
        let server = &parsed["mcp_servers"]["mergelane"];
        assert_eq!(server["command"].as_str(), Some(launch().command.as_str()));
        assert_eq!(
            server["args"].as_array().unwrap().get(1).unwrap().as_str(),
            Some("--read-only")
        );
    }

    #[test]
    fn codex_setup_adds_the_profile_to_an_inline_server_map() {
        let source = "mcp_servers = { other = { command = 'other-server' } }\n";
        let output = merge_toml(source, &launch()).unwrap();
        let parsed: DocumentMut = output.parse().unwrap();
        let servers = &parsed["mcp_servers"];
        assert_eq!(servers["other"]["command"].as_str(), Some("other-server"));
        assert_eq!(
            servers["mergelane"]["args"]
                .as_array()
                .unwrap()
                .get(2)
                .unwrap()
                .as_str(),
            Some("/profile.json")
        );
    }

    #[test]
    fn cursor_installs_embedded_skill_and_merges_an_existing_config() {
        let home = tempfile::tempdir().unwrap();
        let (config, skill) = client_paths(Client::Cursor, home.path());
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            r#"{"mcpServers":{"other":{"command":"other"}},"option":true}"#,
        )
        .unwrap();

        for _ in 0..2 {
            prepare(&[Client::Cursor], home.path(), &launch())
                .unwrap()
                .apply()
                .unwrap();
            let output: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
            assert_eq!(output["mcpServers"]["other"]["command"], "other");
            assert_eq!(output["option"], true);
            assert_eq!(
                output["mcpServers"]["mergelane"]["args"],
                json!(launch().args)
            );
            assert_eq!(fs::read_to_string(&skill).unwrap(), SKILL);
        }
    }

    #[test]
    fn invalid_config_is_left_in_place_before_any_installation() {
        let home = tempfile::tempdir().unwrap();
        let (config, skill) = client_paths(Client::Cursor, home.path());
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "{broken").unwrap();

        assert!(prepare(&[Client::Cursor], home.path(), &launch()).is_err());
        assert_eq!(fs::read_to_string(config).unwrap(), "{broken");
        assert!(!skill.exists());
    }
}
