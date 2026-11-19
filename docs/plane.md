# 开源 Plane 接入

完整团队流程见 [多人协作指南](collaboration.md)，变量和错误排查见 [配置参考](configuration.md)。

MergeLane 连接团队已经部署好的 Plane Community Edition，使用其 API v1 的工单、成员、关系和评论接口。每位成员在自己的开发机运行同一个 MCP 命令，个人 API key 对应自己的 Plane 账号及项目权限。

## 配置

在 Plane 的个人设置中创建 API key，保存到仓库外的文件，Unix 文件权限设置为 `0600`。实例地址使用 HTTPS，本机回环 HTTP 也可用于本地实例。URL 可以带安装子路径。

Codex MCP 配置：

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--plane-url", "https://plane.example.com", "--plane-token-file", "/private/path/plane-api-key", "--plane-workspace", "team"]
```

Claude Code：

```sh
claude mcp add --transport stdio --scope user mergelane -- /absolute/path/to/mergelane \
  mcp --plane-url https://plane.example.com --plane-token-file /private/path/plane-api-key --plane-workspace team
```

Cursor 用户级 `~/.cursor/mcp.json`：

```json
{
  "mcpServers": {
    "mergelane": {
      "type": "stdio",
      "command": "/absolute/path/to/mergelane",
      "args": ["mcp", "--plane-url", "https://plane.example.com", "--plane-token-file", "/private/path/plane-api-key", "--plane-workspace", "team"]
    }
  }
}
```

需要负责人通过工具发布子任务、指派和记录检查点时，在参数数组中加 `--plane-write`。环境变量 `PLANE_URL`、`PLANE_TOKEN_FILE`、`PLANE_WORKSPACE` 也可以提供连接配置。读写权限仍由 Plane 对当前账号执行校验。`team` 替换成实际工作空间的 slug，由管理员配置一次。每次搜索也可以传入 `workspace` 来选择另一个有权限的工作空间。

MCP 客户端的审批策略也会影响写入操作。即使 Plane 账号有权限且 MCP 已开启 `--plane-write`，客户端仍可能拒绝工具调用；需要在客户端确认相应操作。工具中的 `confirmed` 字段由调用方传入，不能替代人工审阅。

## 管理者查询进度

配置好工作空间后，直接对 Agent 说“查账单导出的进度”“Alice 现在做哪些任务”或“看 Billing 项目进度”。Agent 调用 `plane_search_issues`，可以按标题、工单编号、项目名称或编号、队员显示名搜索，也可以读取整个工作空间的可见工单。搜索返回可读状态、负责人和工单链接，Agent 再用链接读取检查点及阻塞关系。查询沿用当前账号的 Plane 权限。

同名工单需要选择，成员重名时用项目成员列表核对。结果超过返回上限会标注 `truncated`，可缩小项目或关键词。单次搜索覆盖最多二十个项目，更多项目时需选定项目。部分项目读取失败会列入 `project_errors`，Agent 汇总时应说明缺失范围。`mine` 可以单独用于当前账号的任务。`assignee` 用于查其他队员。

## 日常流程

1. 负责人在会话里说出总工单标题或编号，让 Agent 搜索并读取需求和项目成员。也可以直接贴 Plane 页面中的完整工单链接，例如 `https://plane.example.com/team/browse/PROJ-123/`。工具通过编号解析 API 查到项目和工单 UUID，返回的链接也使用这一原生格式。项目内包含 UUID 的 `/projects/<项目 UUID>/issues/<工单 UUID>/` 和 `/work-items/<工单 UUID>/` 链接也可作为输入。
2. Agent 调用 `plane_preview_plan`，展示任务、负责人、验收条件和前置依赖。成员 UUID 从 `plane_issue_context` 的成员列表获取。队员日常用名字沟通，Agent 在调用工具时使用对应 UUID。子任务默认在总工单的项目里创建，也可以指定同工作空间下另一个项目的 UUID。
3. 负责人确认后，Agent 调用 `plane_apply_plan`。工具在 Plane 创建和指派子任务，建立原生 `blocked_by` 关系，并在总工单评论中汇总链接。同项目子任务还会设置原生父工单。
4. 队员说出自己的任务标题或让 Agent 查分给自己的任务。选择工单后，Agent 调用 `plane_issue_context` 获取需求、负责人、关系、最近评论和检查点。不同客户端读取的是同一份 Plane 工单。
5. 换人或暂停前，让 Agent 拟好进度、验证结果和下一步。确认后由 `plane_checkpoint` 发布到工单评论，可附 HTTPS 的 Git commit 链接。接手人读取相同工单继续。

启用 `--plane-write` 后，可确认调用 `plane_edit_issue` 修改现有工单的标题或描述。[编辑工单](collaboration.md#编辑工单) · [Plane 更新工单 API](https://developers.plane.so/api-reference/issue/update-issue-detail)

计划预览只读取 Plane，写入工具需要 `--plane-write` 且发布时传入确认后的预览 ID。部分写入失败后，可以在同一会话重试，或在新会话预览原样的计划后重试。工具使用 `external_source=mergelane` 和确定性的 `external_id` 查找已创建任务与评论，继续补齐关系和汇总。同一计划由一位负责人提交。

## 向父工单汇报子工单进度

把分给自己的子工单链接交给 Agent，说“把这张子工单的进度发到父工单评论，先给我确认”。Agent 读取子工单上下文，根据已知进度拟好摘要、验证结果和下一步，然后用 `plane_preview_parent_checkpoint` 查找 Plane 原生父工单并预览评论。你只需提供子工单链接，预览会显示父工单和即将发布的正文。

确认后，Agent 调用 `plane_publish_parent_checkpoint`，传入相同的子工单链接、进度和 `preview_id`，以及 `confirmed: true`。如果在本机领取过该子工单，还需传入子工单的有效 `lease_id`。发布要求启用 `--plane-write` 且当前账号有权在父工单评论。正文会附上子工单链接。

这条流程要求子工单分配给当前 Plane 账号，且在同项目内有可读取的原生父工单。父工单或评论内容变化时需要重新预览和确认。发布结果不明确时，可核对父工单后重试未变更的已确认预览，工具会复用已发布的同一条评论。

## Gitea 文件版本跟踪

如果规范、样例或接口文件在 Gitea，在上述 MCP 命令后加：

```text
--gitea-url https://git.example.com --gitea-token-file /private/path/gitea-token
```

在 Plane 工单描述中贴 Gitea 文件的固定提交链接，例如：

```text
https://git.example.com/team/specs/src/commit/0123456789abcdef0123456789abcdef01234567/schema.yaml
```

链接中的 commit 是这项工作的使用基线。Agent 查询 Plane 工单时，工具比较该版文件和 Gitea 默认分支的当前内容，返回 `current`、`needs_review` 或 `unavailable`，并附固定版本链接。适配并验证后，由负责人把描述里的链接换成新版。最多跟踪八个文件。两个系统分别使用各自的凭据。

只配置 Plane 时，工单、拆分、分派和评论均可直接使用，返回结果的 `dependency_tracking` 会注明 Git 文件跟踪待配置。

## API 与验证范围

实现依据开源仓库中的 [API v1 路由](https://github.com/makeplane/plane/blob/preview/apps/api/plane/api/urls/work_item.py)、[成员接口](https://github.com/makeplane/plane/blob/preview/apps/api/plane/api/views/member.py) 和 [工单序列化字段](https://github.com/makeplane/plane/blob/preview/apps/api/plane/api/serializers/issue.py)。目标实例需提供 `/api/v1/workspaces/.../projects/.../work-items/`，以及成员、评论和关系子接口。

本地模拟服务器测试覆盖游标分页、确认前只读、指派、请求提交后响应失败的恢复、关系写入失败的恢复、共享检查点和 Gitea 文件变化。

2026-11-18 已在真实 Plane Community v1.4.2 上完成拆单、两名成员指派、原生阻塞关系、按项目／标题／队员查询、重复计划重试和检查点回读。三个独立 Codex CLI 会话分别完成产品确认、程序员接手和管理者查询，使用了两个不同的 Plane 账号。跨会话通过 Plane 里的交接标记核对上下文。Claude Code 和 Cursor 的实际客户端操作仍需在已配置对应客户端的环境中验证。升级自托管实例后可再次运行这套验收流程。

2026-11-19 已在真实 Plane 实例通过 `plane_edit_issue` 验证单独更新标题、单独替换描述、清空描述和同时更新两项。独立只读会话回读确认了多行文本、特殊字符和工单元数据。只读模式和未经确认的调用均拒绝编辑。
