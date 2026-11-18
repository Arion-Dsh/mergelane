# Changelog

## 0.1.0

First release of MergeLane: shared tasks, assignments, dependencies and progress for people collaborating through their own agents.

- MCP integrations for Gitea, Plane, GitHub, GitLab and Jira Cloud.
- Read-only remote access by default. Confirmed plan publishing and shared checkpoints are available when remote writes are enabled.
- Local task leases across agent sessions on the same computer and OS account, with automatic renewal, release and takeover after expiry.
- `mergelane init` for personal profiles and Codex, Claude or Cursor configuration snippets.
- `mergelane doctor` for read-only account checks, with an offline configuration check.
- Prebuilt binaries for Apple Silicon/Intel macOS, ARM64/x64 Linux and x64 Windows; Homebrew, shell and PowerShell installers.

GitHub, GitLab and Jira have passed simulated API tests. Live-account acceptance for these three integrations is deferred beyond v0.1.0; real service behavior and permissions remain unverified.
