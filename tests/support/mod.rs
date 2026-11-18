//! Shared process lifecycle and fake credentials for black-box MCP tests.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mergelane"));
    command.env(
        "XDG_CONFIG_HOME",
        std::env::temp_dir().join(format!(
            "mergelane-unconfigured-tests-{}",
            std::process::id()
        )),
    );
    // Tests must never open a developer's credential files or remote instances.
    for variable in [
        "GITEA_URL",
        "GITEA_TOKEN_FILE",
        "PLANE_URL",
        "PLANE_TOKEN_FILE",
        "PLANE_WORKSPACE",
        "GITHUB_URL",
        "GITHUB_TOKEN_FILE",
        "GITLAB_URL",
        "GITLAB_TOKEN_FILE",
        "JIRA_URL",
        "JIRA_TOKEN_FILE",
        "JIRA_EMAIL",
        "JIRA_ISSUE_TYPE",
        "JIRA_LINK_TYPE",
        "JIRA_CLOUD_ID",
    ] {
        command.env_remove(variable);
    }
    command
}

pub struct Mcp {
    pub child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
}

impl Mcp {
    pub fn start(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start MergeLane test process");
        let input = child.stdin.take().expect("child stdin is piped");
        let output = BufReader::new(child.stdout.take().expect("child stdout is piped")).lines();
        Self {
            child,
            input,
            output,
        }
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        let request = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        self.input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .expect("write MCP request");
        let line = tokio::time::timeout(Duration::from_secs(5), self.output.next_line())
            .await
            .expect("MCP response within five seconds")
            .expect("read MCP response")
            .expect("MCP process must not close stdout before responding");
        let response: Value = serde_json::from_str(&line).expect("MCP response is JSON");
        assert_eq!(response["id"], 1, "{response}");
        assert!(response.get("error").is_none(), "{response}");
        response
            .get("result")
            .expect("MCP response has a result")
            .clone()
    }

    pub async fn tool(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":args}))
            .await
    }
}

pub fn tool_result(result: Value) -> Value {
    assert_ne!(result["isError"], true, "{result}");
    serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .expect("tool response contains text"),
    )
    .expect("tool response text is JSON")
}

pub fn token(dir: &Path, name: &str, value: &str) -> PathBuf {
    let path = dir.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(&path)
        .expect("create fake token file")
        .write_all(value.as_bytes())
        .expect("write fake token");
    path
}
