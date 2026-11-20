#!/usr/bin/env python3
"""Check a native release archive without using personal settings or live APIs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time
import tomllib
import zipfile


def run(binary, args, env, *, input=None):
    result = subprocess.run(
        [str(binary), *args], env=env, input=input, text=True,
        capture_output=True, timeout=30, check=True,
    )
    return result.stdout


def smoke_token_prompt(binary, root, env):
    import pty
    import select
    import termios

    config_dir = root / "setup-config"
    client_dir = root / "setup-claude"
    env = {**env, "XDG_CONFIG_HOME": str(config_dir), "CLAUDE_CONFIG_DIR": str(client_dir),
           "TERM": "xterm"}
    fake_token = b"release-smoke-interactive-token"
    master, slave = pty.openpty()
    with os.fdopen(master, "r+b", buffering=0) as terminal, os.fdopen(slave, "rb") as child_tty:
        mode = termios.tcgetattr(child_tty)
        mode[3] |= termios.ECHO
        termios.tcsetattr(child_tty, termios.TCSANOW, mode)
        termios.tcsetwinsize(child_tty, (24, 120))
        transcript = bytearray()
        deadline = time.monotonic() + 20

        def read_until(prompt):
            while prompt not in transcript:
                remaining = deadline - time.monotonic()
                assert remaining > 0, f"Timed out waiting for {prompt!r}"
                readable, _, _ = select.select([terminal], [], [], remaining)
                assert readable, f"Timed out waiting for {prompt!r}"
                transcript.extend(terminal.read(4096))

        with subprocess.Popen([str(binary), "setup"], env=env, stdin=child_tty,
                              stdout=child_tty, stderr=child_tty) as process:
            try:
                read_until(b"Agent (codex, claude, cursor, all) [codex]: ")
                terminal.write(b"claude\n")
                read_until(b"Platform: gitea, plane, github, gitlab, jira [github]: ")
                terminal.write(b"\n")
                read_until(b"Instance URL [https://github.com]: ")
                terminal.write(b"\n")
                read_until(b"Paste the personal token you created on the platform:")
                # A terminal can paste as soon as the prompt becomes visible.
                assert not termios.tcgetattr(child_tty)[3] & termios.ECHO, "Echo enabled at token prompt"
                terminal.write(b"\x1b[200~" + fake_token + b"\x1b[201~\r")
                read_until(b"MergeLane MCP and Skill installed.")
                assert process.wait(timeout=5) == 0
            finally:
                if process.poll() is None:
                    process.kill()

        assert fake_token not in transcript, "Terminal echoed the test token"
        assert termios.tcgetattr(child_tty) == mode, "Terminal mode was not restored"

    profile = config_dir / "mergelane" / "config.json"
    assert fake_token not in profile.read_bytes()
    credentials = json.loads(profile.read_text())["backends"]["github"]["token_file"]
    assert Path(credentials).read_bytes() == fake_token
    print(f"Interactive setup kept echo off during immediate paste: {binary}")


def smoke(binary, root, version):
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(("GITEA_", "PLANE_", "GITHUB_", "GITLAB_", "JIRA_")):
            env.pop(key)
    env["XDG_CONFIG_HOME"] = str(root / "unused-config")
    assert run(binary, ["--version"], env).strip() == f"mergelane {version}"
    for command in ("setup", "init", "doctor", "mcp", "pull"):
        run(binary, [command, "--help"], env)
    token = root / "fake-token"
    token.touch(mode=0o600)
    token.write_text("release-smoke-fake-token\n")
    config = root / "profile.json"
    snippet = json.loads(run(binary, [
        "--config", str(config), "init", "--backend", "github",
        "--token-file", str(token), "--non-interactive", "--client", "cursor",
    ], env))
    assert "mergelane" in snippet["mcpServers"]
    assert "release-smoke-fake-token" not in config.read_text()
    report = json.loads(run(binary, ["--config", str(config), "doctor", "--offline"], env))
    assert report["ok"] and report["offline"]
    requests = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": {"name": "release-smoke", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    ]
    output = run(binary, ["--config", str(config), "mcp", "--read-only"], env,
                 input="".join(json.dumps(request) + "\n" for request in requests))
    responses = {message["id"]: message for message in map(json.loads, output.splitlines())}
    assert "serverInfo" in responses[1]["result"]
    names = {tool["name"] for tool in responses[2]["result"]["tools"]}
    assert names == {
        "github_search_issues", "github_issue_context", "github_preview_plan",
        "mergelane_pull_tasks", "mergelane_task_lease",
    }
    print(f"CLI, init, offline doctor and read-only MCP passed: {binary}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("target")
    args = parser.parse_args()
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    windows = "windows" in args.target
    name = f"mergelane-{args.target}" + (".zip" if windows else ".tar.xz")
    archive = args.directory / name
    expected = (args.directory / (name + ".sha256")).read_text().split()[0]
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == expected, "SHA-256 mismatch"
    with tempfile.TemporaryDirectory(prefix="mergelane-package-") as temp:
        root = Path(temp)
        unpacked = root / "unpacked"
        if windows:
            with zipfile.ZipFile(archive) as handle:
                handle.extractall(unpacked)
        else:
            with tarfile.open(archive) as handle:
                handle.extractall(unpacked, filter="data")
        binary = next(unpacked.rglob("mergelane.exe" if windows else "mergelane"))
        assert next(unpacked.rglob("LICENSE")).is_file()
        assert next(unpacked.rglob("README.md")).is_file()
        smoke(binary, root, version)
        if os.name == "posix":
            smoke_token_prompt(binary, root, os.environ)
    print(f"Archive verified: {name}")


if __name__ == "__main__":
    main()
