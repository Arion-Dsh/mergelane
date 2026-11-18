# GitHub 接入

每人使用自己的 GitHub 凭据，查询、分派和记录同一组 Issues。支持 GitHub.com；Enterprise Server 需要提供所用 REST API 和版本头，旧版本可能缺少依赖接口。完整流程见 [团队协作](collaboration.md)。

## 配置

创建可访问目标仓库的个人 token，放在仓库外；Unix 文件权限使用 `0600` 或 `0400`。细粒度 token 需要目标仓库的 Issues 读取权限；建单、指派、评论和依赖需要写入权限，账号也须具有允许指派和维护依赖的仓库权限。

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--github-url", "https://github.com", "--github-token-file", "/absolute/path/outside/repository/github-token"]
```

发布任务或检查点时添加 `--github-write`。环境变量为 `GITHUB_URL`、`GITHUB_TOKEN_FILE`。URL 填网站根地址；GitHub.com 自动使用 `https://api.github.com/`，其他实例使用安装前缀下的 `/api/v3/`。版本头为 `2026-03-10`，不跟随重定向。

## 工具与查询

提供 `github_search_issues`、`github_issue_context`、`github_preview_plan`；写入时增加 `github_apply_plan`、`github_checkpoint`。工单使用完整网页链接或 `owner/repo#42`。项目为 `owner/repo`，负责人使用上下文返回的可指派成员 login。计划任务默认在总任务仓库创建，也可指定 `project`。

```json
{"project":"team/web","query":"export","assignee":"alice","page":1}
```

搜索排除 PR，`mine` 查询当前账号的任务，不能与 `assignee` 同时提供。查询正文支持 GitHub 搜索语法。通过 `next_page` 翻页；达到搜索上限时返回 `truncated`，应缩小范围。

## 发布与恢复

预览后人工审阅，用当前会话的最新 `plan_id` 和 `confirmed: true` 发布。`after` 为更早任务的零基下标，保存为 GitHub 原生 `blocked_by` 关系。新任务通过来源正文与总任务汇总关联，不额外设置原生父子层级。

任务首行的 `mergelane:task:...`，评论首行的 `mergelane:plan:...` 和 `mergelane:checkpoint:...` 用于恢复，请勿移除。每类完整列表最多读取 20 页、每页 100 项，超出明确报错。失败可能已保存任务，先检查远端状态，再以原样计划重试；同一计划由一人发布。恢复时核对项目和负责人，不自动覆盖人工调整。

本后端暂不执行 GitHub 文件版本比较。实现依据官方 [Issues API](https://docs.github.com/en/rest/issues/issues)、[依赖 API](https://docs.github.com/en/rest/issues/issue-dependencies) 和 [搜索 API](https://docs.github.com/en/rest/search/search#search-issues-and-pull-requests)。模拟测试覆盖认证、安装前缀、分派、依赖方向、部分失败恢复和共享进度；尚未在真实 GitHub 账号或 Enterprise 实例验收。
