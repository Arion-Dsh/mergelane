# GitLab 接入

连接 GitLab.com 或自托管 GitLab 的 REST v4 Issues API，每人通过自己的 Agent 查询同一项目的任务和进度。完整流程见 [团队协作](collaboration.md)。

## 配置

个人访问 token 的读取使用 `read_api`；发布任务、分派、评论或依赖时使用 `api`，并受账号项目权限限制。token 放在仓库外，Unix 权限使用 `0600` 或 `0400`。

```toml
[mcp_servers.mergelane]
command = "/absolute/path/to/mergelane"
args = ["mcp", "--gitlab-url", "https://gitlab.com", "--gitlab-token-file", "/absolute/path/outside/repository/gitlab-token"]
```

发布操作加 `--gitlab-write`。环境变量为 `GITLAB_URL`、`GITLAB_TOKEN_FILE`。URL 填实例根地址，可带安装前缀，程序追加 `/api/v4/`，使用 `PRIVATE-TOKEN` 请求头。

## 工具与标识

提供 `gitlab_search_issues`、`gitlab_issue_context`、`gitlab_preview_plan`。启用写入时还提供 `gitlab_create_issue`、`gitlab_assign_issue`、`gitlab_edit_issue`、`gitlab_apply_plan`、`gitlab_checkpoint`。

工单支持完整网页链接（`/-/issues/42` 或 `/-/work_items/42`）或 `group/project#42`。项目支持 `group/subgroup/project`。任务的 `assignee` 使用上下文返回的**数字用户 ID 的字符串**，搜索支持 ID 或用户名。每项任务指派一人。

```json
{"project":"group/subgroup/project","query":"export","assignee":"alice","page":1}
```

普通搜索需指定 `project`；不指定项目时只支持 `mine: true` 的全局个人任务查询，避免依赖 GitLab.com 可能禁用的全局 `scope=all`。通过 `next_page` 翻页直到为空。

## 依赖与恢复

`after` 保存为原生 `is_blocked_by`。**Blocking issues 需要支持该功能的实例与套餐，官方将其列为 Premium／Ultimate。** 能力不足时明确报错，不把普通关联当作阻塞关系。无前置依赖的计划可独立使用。

发布需最新预览和确认，任务通过来源正文和总任务评论关联，不额外创建原生父子层级。保留任务与评论首行的 `mergelane:...` 标记。每类完整列表最多读取 20 页、每页 100 项，超出明确报错。

失败可能已创建工单，先检查实际状态和权限，再原样重试；更换会话需重新预览相同计划。同一计划由一人发布，远端负责人不一致时停止并要求核对。本后端暂不执行 GitLab 文件版本比较。

实现依据官方 [Issues API](https://docs.gitlab.com/api/issues/)、[Issue links API](https://docs.gitlab.com/api/issue_links/)、[Notes API](https://docs.gitlab.com/api/notes/) 和 [阻塞关系说明](https://docs.gitlab.com/user/project/issues/related_issues/#blocking-issues)。模拟测试覆盖认证、命名空间编码、分派、关系方向、失败恢复和共享评论；尚未在真实 GitLab 实例或不同套餐上验收。
