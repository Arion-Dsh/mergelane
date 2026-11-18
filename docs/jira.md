# Jira Cloud 接入

通过 Jira Cloud REST v3 共享任务、负责人、阻塞关系与进度。使用新的 `/search/jql` 和 Atlassian Document Format（ADF）描述／评论。Jira Data Center、OAuth 登录和自定义必填字段写入暂不支持。

## 配置与权限

使用 Atlassian 账号邮箱和 API token，token 放在仓库外，Unix 权限为 `0600` 或 `0400`。账号需要浏览项目、读取工单和可指派成员的权限；发布还需要创建、指派、评论和链接工单权限。Scoped token 须包含对应 API 的 scopes。

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--jira-url", "https://team.atlassian.net", "--jira-email", "you@example.com", "--jira-token-file", "/absolute/path/outside/repository/jira-token"]
```

发布时加 `--jira-write`，支持这些环境变量：

| 变量 | 用途 |
| --- | --- |
| `JIRA_URL` | 网页站点根地址 |
| `JIRA_EMAIL` | 创建 token 的账号邮箱 |
| `JIRA_TOKEN_FILE` | token 文件路径，最多 4096 字节 |
| `JIRA_CLOUD_ID` | Scoped token 使用的站点 Cloud ID（UUID），可选 |
| `JIRA_ISSUE_TYPE` | 创建类型名称或数字 ID，默认 `Task` |
| `JIRA_LINK_TYPE` | 阻塞关系类型名称，默认 `Blocks` |

都有对应的 `--jira-*` 参数。Scoped token 必须设置 `--jira-cloud-id`／`JIRA_CLOUD_ID`，程序使用官方 `https://api.atlassian.com/ex/jira/<cloudId>/rest/api/3/`，网页链接仍使用 `JIRA_URL`。未设置时直接使用站点 `/rest/api/3/`，适用于无 scopes 的 API token。认证均为邮箱加 token 的 Basic 认证。

项目必须允许所选类型及默认字段组合；若创建界面有其他必填字段，当前创建会失败，需要调整配置或扩展字段支持。`JIRA_LINK_TYPE` 需保持“向外阻塞、向内被阻塞”的含义，不能直接替换为反向语义的关系。

## 工具与查询

提供 `jira_search_issues`、`jira_issue_context`、`jira_preview_plan`；写入时增加 `jira_apply_plan`、`jira_checkpoint`。工单输入支持 `WEB-42` 或当前站点 `/browse/WEB-42` 链接，项目用 key，负责人用上下文返回的 **accountId**。

```json
{"project":"WEB","query":"export","mine":true}
```

搜索至少提供项目、关键词、负责人或 `mine`。合法工单 key 按 key 查询，否则按标题搜索。工具转义并构造 JQL，`query` 不是原始 JQL。将 `next_cursor` 作为下一次的 `cursor`，不要递增 `page`。可指派成员与评论读取均有数量上限；成员列表也受 Jira 接口自身的候选用户过滤限制。

## 发布和恢复

预览不写远端，发布需同一会话的最新 ID 与确认。创建的是独立工单，通过来源描述与总任务汇总关联，不额外创建 subtask 层级。前置任务保存为原生 `Blocks` 关系。

任务描述首行保留 `mergelane:task:...`，并添加 `mergelane-task-...` label；计划和检查点评论也有首行标记。ADF 转换保留段落边界。恢复时核对 label、描述、项目和负责人，重复或不一致时停止。

**Jira 搜索存在索引延迟。** 创建报错不代表没有保存，立即重试可能尚查不到已有任务。先在 Jira 核对状态，等待索引可见后再原样重试；同一计划由一人发布。工具不自动重试创建请求，不承诺跨会话或并发情况下的“恰好一次”写入。暂不执行 Git 文件版本比较。

实现依据官方 [工单 API](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/)、[增强搜索](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/)、[关系模型](https://developer.atlassian.com/cloud/jira/platform/issue-linking-model/)、[token 规则](https://support.atlassian.com/atlassian-account/docs/manage-api-tokens-for-your-atlassian-account/) 和 [Basic 认证](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)。模拟测试覆盖 ADF、认证、游标、分派、依赖方向、失败恢复和跨会话进度；尚未在真实 Jira Cloud 站点验收。
