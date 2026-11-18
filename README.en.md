# MergeLane

English · [简体中文](README.md)

**People use their own agents to share tasks, assignments, dependencies, and progress.**

MergeLane provides MCP collaboration tools for teams using Gitea, Plane, GitHub, GitLab, or Jira Cloud. Leads split and assign work, teammates work through their own coding-agent clients, and managers query the same shared tasks and progress.

MergeLane runs on each developer's machine over MCP stdio. Issues and checkpoints stay in your existing task platform. The latest plan preview lives in the current MCP session; no additional MergeLane server or database is required.

Before starting work, the agent claims a local task lease. Sessions under the same OS user on the same computer share the lease store across clients and worktrees. The owning MCP connection renews its lease automatically. The agent releases it when done or paused, and a crashed session's lease can be reclaimed after expiry. See the [task lease workflow](docs/collaboration.md#本地任务租约).

All five integrations can run independently or together, with separate credentials, write flags, and pending previews. GitHub, GitLab, and Jira share a REST collaboration workflow while adapters preserve platform-specific identity, search, and dependency semantics. See the [GitHub](docs/github.md), [GitLab](docs/gitlab.md), and [Jira](docs/jira.md) setup guides (in Chinese).

Native blockers depend on the platform's APIs, permissions, and plan. GitLab blocking relations require an eligible plan; Jira support targets Cloud REST v3. Git file version comparisons remain available through Gitea and Plane. The three new adapters have mock API coverage; live account acceptance has not been performed.

## Getting started

After the first stable release, macOS users can install prebuilt binaries from the [MergeLane Homebrew tap](https://github.com/Arion-Dsh/homebrew-tap) using the full formula name:

```sh
brew install Arion-Dsh/tap/mergelane
mergelane init
mergelane doctor
```

**The tap is initialized. This command becomes available when the first release publishes the formula.** See [installation and setup](docs/installation.md) (in Chinese). Source builds require Rust 1.97.0, pinned in [rust-toolchain.toml](rust-toolchain.toml).

```sh
cargo install --locked --path .
mergelane --help
```

Store your personal API token in a file outside the repository. On Unix, set its permissions to `0600` or `0400`. Configure your MCP client with the absolute path to the installed executable:

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--gitea-url", "https://git.example.com", "--gitea-token-file", "/absolute/path/outside/repository/gitea-token"]
```

For Plane, replace the arguments with:

```toml
args = ["mcp", "--plane-url", "https://plane.example.com", "--plane-token-file", "/absolute/path/outside/repository/plane-api-key", "--plane-workspace", "team"]
```

Multiple integrations can run in the same process. HTTPS is required except for loopback HTTP; installation subpaths are supported. See the detailed [Gitea](docs/gitea.md) and [Plane](docs/plane.md) guides (in Chinese) for permissions, client setup, recovery, and compatibility notes.

[.env.example](.env.example) lists supported environment variables. MergeLane **does not automatically load `.env` files**. Export variables before starting the process or set them in your MCP client. Configuration contains token file paths, never token values.

`init` saves personal settings and prints a reviewed client snippet without changing client files. Use `--client claude` or `--client cursor` for JSON, or the default Codex TOML. `mcp` and `doctor` load saved settings; `--config` selects a file and `--no-config` ignores it. Explicit arguments override saved settings, which override environment variables. `doctor --offline` skips network checks. `mcp --read-only` disables all write tools.

## Team workflow

Each person configures their own client and personal credentials. The shared backend holds task assignments, blockers, native issue state, and progress comments. Use the backend UI to maintain issue state; MergeLane reads that state and publishes plans and checkpoints.

1. Ask the agent to find the parent issue and preview a task split with assignees, acceptance criteria, and dependencies.
2. Review the preview before publishing. Writes are disabled by default; add the corresponding `--<backend>-write` flag when needed. Publishing requires the latest preview ID and a confirmation flag. The caller supplies that flag, so retain human approval in your client.
3. Give teammates their issue links. Agents read the same remote issues, blockers, and checkpoints from their own sessions.
4. Before handing off, review and publish a checkpoint containing progress, checks, and next steps.
5. Link Gitea files using full commit URLs in issue descriptions. MergeLane compares their pinned versions with the default branch and reports `current`, `needs_review`, or `unavailable`.

Publishing is not transactional. After a partial failure, inspect the remote state and retry the unchanged plan. One person should publish a given plan at a time. Restarted sessions must preview the same plan again before retrying.

The CLI also provides `gitea-context` and `gitea-watch`; use `mergelane <command> --help` for arguments. Watch mode emits JSON lines when issue state, assignees, or dependency results change.

See the [documentation index](docs/README.md) and [team collaboration walkthrough](docs/collaboration.md) for the complete workflow (in Chinese).

## Development

```sh
cargo fmt --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

Integration tests use mock APIs on random loopback ports, temporary files, and fake credentials. No live instance or account is required. CI is configured for Linux, macOS, and Windows. See [CONTRIBUTING.md](CONTRIBUTING.md) and [architecture](docs/architecture.md) for contribution and maintenance guidance (in Chinese), and [SECURITY.md](SECURITY.md) for reporting security issues.

## License

[MIT](LICENSE). Source installation is available. Binary release and Homebrew workflows are prepared but not published; crates.io publishing remains disabled. See [the release guide](docs/releasing.md).
