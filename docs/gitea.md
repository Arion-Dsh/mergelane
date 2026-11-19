# Gitea 接入与协作

完整团队流程见 [多人协作指南](collaboration.md)，变量和错误排查见 [配置参考](configuration.md)。

每位成员在开发机安装一个 `mergelane`，用个人 Gitea API token 连接已有实例。Git 分支、工作区、PR 和 CI 沿用团队现有流程。

## 配置一次

在 Gitea 的账号设置中创建凭据，授权 `read:repository` 和 `read:issue`。要通过 MCP 建单、指派或发布检查点时，使用有对应仓库权限的 `write:issue` 凭据，并在下面的命令参数中追加 `--gitea-write`。Gitea 会根据该账号和仓库权限决定允许的操作。

把 token 保存在仓库外的文件中。在 Unix 上设置文件权限为 `0600`。实例地址支持 HTTPS、回环 HTTP 和安装子路径。以下示例中的地址与文件路径按实际值替换。MCP 命令也支持用 `GITEA_URL` 和 `GITEA_TOKEN_FILE` 提供这两项配置。

### Codex

在 MCP 配置中添加：

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--gitea-url", "https://git.example.com", "--gitea-token-file", "/private/path/gitea-token"]
```

### Claude Code

```sh
claude mcp add --transport stdio --scope user mergelane -- /absolute/path/to/mergelane \
  mcp --gitea-url https://git.example.com --gitea-token-file /private/path/gitea-token
```

### Cursor

将下面的 MCP 项合入用户级 `~/.cursor/mcp.json`：

```json
{
  "mcpServers": {
    "mergelane": {
      "type": "stdio",
      "command": "/absolute/path/to/mergelane",
      "args": ["mcp", "--gitea-url", "https://git.example.com", "--gitea-token-file", "/private/path/gitea-token"]
    }
  }
}
```

## 拆单、指派和开工

管理者可以直接问“查导出进度”或“Bob 负责哪些工单”。Agent 调用 `gitea_search_issues`，按关键词、仓库所有者和队员用户名搜索，再从结果链接读取进展。搜索覆盖当前账号可访问的工单。返回的 `next_page` 用于继续翻页，直到为空。多张工单同名时先选择。成员也可以用 `mine` 查当前账号的任务。

1. **负责人贴总工单。** 告诉 Agent 目标、队员的 Gitea 用户名和仓库，让它草拟任务。Agent 调用 `gitea_preview_plan`，展示每项的标题、负责人、验收条件和依赖。此时 Gitea 工单保持原样。
2. **负责人确认后发布。** Agent 调用 `gitea_apply_plan`。MergeLane 在 Gitea 创建和指派各张工单，添加原生阻塞关系，并在总工单留下一条包含链接的汇总评论。目标仓库需启用 Issues，添加阻塞关系的仓库需允许工单依赖。
3. **队员贴自己的工单。** 任何已配置的客户端都可以用 `gitea_issue_context` 查工单、阻塞关系、依赖文件和最新检查点。每个会话显式带工单链接或 `owner/repo#42`，同一目录里的不同会话也能分别查各自工单。同时改代码时可使用独立 worktree。
4. **离开或换人前记录上下文。** Agent 拟好进度、已完成验证和下一步，由人确认后通过 `gitea_checkpoint` 写到工单评论。也可以附已推送的 Gitea 提交链接。新的 Agent 用相同工单即可读取。成员的会话记录保留在各自的客户端中。

如果发布计划时部分请求失败，先到 Gitea 查看已创建的工单。再用原样的预览重试，MergeLane 会按工单中的计划标记查找已创建的工作，并继续补依赖和汇总。请保留计划标记，并由一位负责人提交同一计划。

启用 `--gitea-write` 后，可确认调用 `gitea_edit_issue` 修改现有工单的标题或描述。[编辑工单](collaboration.md#编辑工单)

## 跟踪资料版本

在工单描述贴上同一 Gitea 实例里的文件永久链接，格式如下。可以单独占一行，也可以用 Markdown 链接。链接里的完整 commit 就是这张工单目前使用的版本。

```text
https://git.example.com/team/specs/src/commit/0123456789abcdef0123456789abcdef01234567/fixtures/export.csv
```

MergeLane 将固定版本和资料仓库**默认分支**上的同一文件作比较。无关文件的提交不会触发提醒。文件内容变化显示 `needs_review`，并给出旧版和新版的固定链接。文件删除或权限不足显示 `unavailable`。队员适配新版并完成验证后，在 Gitea 工单里替换为新版的固定链接。每张工单最多跟踪八个文件链接。

想从终端快速核对，或把变更接入已有通知流程，可以运行：

```sh
export GITEA_URL=https://git.example.com
export GITEA_TOKEN_FILE=/private/path/gitea-token
mergelane gitea-context 'team/web#42'
mergelane gitea-watch --interval 60 'team/web#42' 'team/qa#17'
```

`gitea-watch` 首次输出状态，随后在工单状态、负责人或依赖结果变化时输出一行 JSON。
