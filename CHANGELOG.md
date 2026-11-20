# Changelog

## 0.1.3

- Disable terminal echo before the setup wizard prompts for a personal token, including when the token is pasted immediately.

## 0.1.2

- Add a one-command installer for macOS, Linux, and Windows that installs `mergelane`, registers MCP in the selected clients, and installs the shared Skill for Codex, Claude Code, and Cursor.
- Add `mergelane setup` with hidden personal-token input and reuse of existing MergeLane profiles.

## 0.1.1

- Preview and publish a progress comment on a Plane parent work item using the assigned child's link. The preview shows the destination and comment for confirmation, and the published comment links back to the child.

## 0.1.0

First release of MergeLane: shared tasks, assignments, dependencies and progress for people collaborating through their own agents.

- MCP integrations for Gitea, Plane, GitHub, GitLab and Jira Cloud.
- Read-only remote access by default. Confirmed plan publishing and shared checkpoints are available when remote writes are enabled.
- Local task leases across agent sessions on the same computer and OS account, with automatic renewal, release and takeover after expiry.
- Read-only snapshots of personal or project work via `mergelane pull` or the `mergelane_pull_tasks` MCP tool.
- Standalone unassigned issue creation and assignment by issue link across accounts and MCP sessions.
- Confirmed title and description edits for existing issues on all five platforms, with live verification on Plane.
- `mergelane init` for personal profiles and Codex, Claude or Cursor configuration snippets.
- `mergelane doctor` for read-only account checks, with an offline configuration check.
- Prebuilt binaries for Apple Silicon/Intel macOS, ARM64/x64 Linux and x64 Windows; Homebrew, shell and PowerShell installers.

GitHub, GitLab and Jira have passed simulated API tests. Live-account acceptance for these three integrations is deferred beyond v0.1.0; real service behavior and permissions remain unverified.
