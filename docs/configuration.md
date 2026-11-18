# 配置与排错

MergeLane 通过 CLI 参数或进程环境变量读取连接配置，命令参数优先。程序不会自动加载 `.env`；[.env.example](../.env.example) 仅是变量参考。MCP 客户端需使用已安装的 `mergelane` 的绝对路径。

`mergelane init` 可保存个人配置并生成客户端片段。`mcp`／`doctor` 自动加载默认配置，优先级为显式参数、保存设置、环境变量。`--config <路径>` 指定文件，`--no-config` 忽略保存设置。详见 [安装与首次配置](installation.md)。

## 连接配置

| CLI 参数 | 环境变量 | 用途 |
| --- | --- | --- |
| `--gitea-url` | `GITEA_URL` | Gitea 实例根地址，可带安装子路径 |
| `--gitea-token-file` | `GITEA_TOKEN_FILE` | Gitea 个人 token 文件路径 |
| `--plane-url` | `PLANE_URL` | Plane 实例根地址，可带安装子路径 |
| `--plane-token-file` | `PLANE_TOKEN_FILE` | Plane 个人 API key 文件路径 |
| `--plane-workspace` | `PLANE_WORKSPACE` | Plane 搜索默认使用的工作空间 slug |
| `--github-url` | `GITHUB_URL` | GitHub 网站根地址（自动选择 REST API 地址） |
| `--github-token-file` | `GITHUB_TOKEN_FILE` | GitHub 个人 token 文件 |
| `--gitlab-url` | `GITLAB_URL` | GitLab 实例根地址 |
| `--gitlab-token-file` | `GITLAB_TOKEN_FILE` | GitLab 个人 token 文件 |
| `--jira-url` | `JIRA_URL` | Jira Cloud 网页站点根地址 |
| `--jira-email` | `JIRA_EMAIL` | Atlassian 账号邮箱 |
| `--jira-token-file` | `JIRA_TOKEN_FILE` | Jira API token 文件 |
| `--jira-cloud-id` | `JIRA_CLOUD_ID` | Scoped token 的 Cloud ID，选择官方 API 网关 |
| `--jira-issue-type` | `JIRA_ISSUE_TYPE` | Jira 创建类型名称或 ID，默认 `Task` |
| `--jira-link-type` | `JIRA_LINK_TYPE` | Jira 阻塞关系类型名，默认 `Blocks` |

每个后端的地址和凭据文件必须同时提供。MCP 模式至少配置一个后端，可以同时配置多个平台。Jira 还需账号邮箱；scoped token 需 Cloud ID。Plane 未设置默认工作空间时，搜索调用需显式提供 `workspace`。

凭据文件放在仓库外，内容为 token 本身，允许末尾换行，总大小不超过 4096 字节。Unix 上使用 `0600` 或 `0400`，其他平台通过系统权限限制访问。实例地址使用 HTTPS，回环地址也允许 HTTP；URL 不能携带用户名、密码、查询参数或片段，客户端不跟随重定向。

## 命令

| 命令 | 行为 |
| --- | --- |
| `mergelane mcp` | 从标准输入读取按行 JSON-RPC，向标准输出返回 MCP 响应 |
| `mergelane init` | 配置一个平台，保留其他平台，打印客户端配置片段 |
| `mergelane doctor` | 验证配置和凭据后检查当前账号接口，不执行发布 |
| `mergelane doctor --offline` | 只验证本地设置与凭据文件 |
| `mergelane gitea-context 'team/web#42'` | 输出选定 Gitea 工单、阻塞关系、评论与资料版本信息 |
| `mergelane gitea-watch 'team/web#42'` | 轮询选定的 Gitea 工单，输出发生变化的 JSON 行 |

`gitea-context` 和 `gitea-watch` 需要 Gitea 地址与凭据。Watch 默认间隔 60 秒，`--interval` 可设为 10–3600 秒，一次选择 1–32 张工单。退出时使用 `Ctrl+C`。

完整参数以当前程序帮助为准：

```sh
mergelane --help
mergelane mcp --help
mergelane gitea-watch --help
```

## 读取与写入

默认提供每个已配置后端的 `*_search_issues`、`*_issue_context` 和 `*_preview_plan`。预览只读取远端，最新预览保留在当前 MCP 会话。

`--gitea-write`、`--plane-write`、`--github-write`、`--gitlab-write`、`--jira-write` 各自额外提供对应平台的 `*_apply_plan` 和 `*_checkpoint`。五个开关相互独立，并且仍受远端账号权限与客户端审批控制。发布计划需匹配当前会话的最新 `plan_id`，写入需调用方提供确认标记。团队流程见 [协作指南](collaboration.md)。

本地领取默认可用，与各平台发布开关独立。`mcp --read-only` 禁用远端发布及本地领取、续租和释放，保留租约状态查询。`doctor` 始终只读，账号连接成功不代表具备特定项目或写入权限。

租约默认 120 秒，`mergelane mcp --lease-seconds <秒数>` 可设为 3–600 秒。客户端共用 `~/.mergelane/leases-v1/`（Windows 使用用户配置文件目录下的 `.mergelane/leases-v1/`），与项目所在目录和 `--config` 无关。各客户端应使用同一个本机用户目录。不要在进程运行时移动或删除租约文件。见 [本地任务租约](collaboration.md#本地任务租约)。

## 常见问题

| 现象 | 检查方式 |
| --- | --- |
| MCP 客户端找不到命令 | 确认源码安装成功、二进制绝对路径正确，客户端能执行该文件 |
| 提示缺少后端配置 | 同时提供该后端的 URL 和 token 文件；确认变量传入了实际 MCP 子进程 |
| token 文件权限或大小错误 | 检查是否为普通文件、总大小不超过 4096 字节；Unix 上修正为 `0600` 或 `0400` |
| 请求返回 HTTP 401／403 | 核对个人凭据、账号的项目或仓库权限及 API 授权范围 |
| 写入工具不出现在工具列表 | 检查 MCP 参数是否包含对应的写入开关，并重新启动该 MCP 进程 |
| 客户端拒绝写入 | 检查客户端的审批提示和策略；账号权限与写入开关不能代替客户端批准 |
| 发布提示需要预览或 ID 不匹配 | 在同一会话重新预览，审阅后使用最新预览 ID；失败恢复需保留原计划内容 |
| 搜索没有找到同事的任务 | 核对实例、工作空间、项目、成员身份与访问权限；缩小筛选或确认同名对象 |
| 搜索结果不完整 | Gitea／GitHub／GitLab 继续读取 `next_page`；Jira 传递 `next_cursor`；Plane 检查 `truncated` 和 `project_errors`，缩小项目或关键词 |
| 文件版本显示 `unavailable` | 检查固定提交链接、文件是否存在，以及当前账号是否能读取资料仓库 |
| 安装子路径下请求失败 | 使用实例根地址及正确安装前缀，不把某个工单页面或 API endpoint 当作实例地址 |

提交问题时附版本、脱敏配置、错误与最小复现。不要发送真实 token 或私人工单内容；参见 [贡献指南](../CONTRIBUTING.md)。
