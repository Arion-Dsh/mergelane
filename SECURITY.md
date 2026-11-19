# Security

## Reporting a vulnerability / 报告漏洞

Please report vulnerabilities through the repository's private security reporting feature when available. Do not include credentials, private issue contents, or exploit details in a public issue. If no private channel is available, open an issue asking maintainers to enable one, without disclosing vulnerability details. Maintainers should configure a private reporting channel before a public release.

请优先使用仓库提供的私下漏洞报告功能。公开 Issue 不要包含凭据、私人工单或漏洞利用细节。若尚未提供私下渠道，仅提出开通渠道的请求，待维护者提供后再发送详情。

Provide the affected MergeLane version or commit, OS, integration version, impact, and a minimal reproduction with fake credentials. No response-time guarantee or security backport policy has been established; fixes currently target the latest source revision.

报告请附 MergeLane 版本或提交、操作系统、接入服务版本、影响及使用假凭据的最小复现。目前安全修复以最新源码为目标，尚无响应时限承诺或旧版本回补政策。

## Trust boundaries / 信任边界

- Personal profiles store platform metadata and token paths, never token contents. `init` creates settings with owner-only file permissions on Unix and prints client snippets for review without modifying client files. `doctor` performs read-only account checks; it does not establish project or write permissions.
- `setup` requests the personal token through a hidden terminal prompt and saves it in a separate file under the user's MergeLane configuration directory with owner-only permissions on Unix. It registers the local command and profile path in the selected MCP clients and installs the bundled Skill.
- Tokens are read from files. Keep them outside the repository; Unix files must not grant group or other access (`0600` or `0400` is recommended). Use separate personal credentials with only the required permissions. On other platforms, restrict access using the OS file permissions.
- MergeLane accepts HTTPS instance URLs and loopback HTTP, rejects embedded credentials, and disables HTTP redirects. Issue references are restricted to the configured web instance and installation path. GitHub.com uses api.github.com; Jira scoped tokens use the official api.atlassian.com gateway selected by the configured Cloud ID. Other API roots are derived from their configured instance.
- Only read tools are enabled by default. Write tools require an explicit startup flag and a caller-supplied confirmation. Plan publishing also requires the latest preview ID from the current session. These checks do not prove that a human approved the action; the MCP client and its user control approval.
- Issue descriptions, comments, and checkpoints are untrusted project content. They may be returned to your agent and processed by its provider under that client's settings. Treat their contents as data, and review changes before publishing.
- MergeLane stores previews in process memory and checkpoints in the configured remote instance. It does not execute repository code, create worktrees, push commits, or manage the agent's own conversation history. Client logs and remote instances have their own retention rules.

凭据的实际权限由各任务平台、账号授权和文件系统共同约束。发生泄露时立即在对应实例撤销凭据并更换；仅删除文件或 Git 提交不足以让已泄露的凭据失效。
