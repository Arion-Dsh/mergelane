# 配置与排错

MergeLane 通过 CLI 参数或进程环境变量读取连接配置，命令参数优先。程序不会自动加载 `.env`；[.env.example](../.env.example) 仅是变量参考。MCP 客户端需使用已安装的 `mergelane` 的绝对路径。

`mergelane init` 可保存个人配置并生成客户端片段。`mcp`、`doctor` 和 `pull` 自动加载默认配置，优先级为显式参数、保存设置、环境变量。`--config <路径>` 指定文件，`--no-config` 忽略保存设置。详见 [安装与首次配置](installation.md)。

## 连接配置

| CLI 参数 | 环境变量 | 用途 |
| --- | --- | --- |
| `--gitea-url` | `GITEA_URL` | Gitea 实例根地址，可带安装子路径 |
| `--gitea-token-file` | `GITEA_TOKEN_FILE` | Gitea 个人 token 文件路径 |
| `--gitea-default-project` | `GITEA_DEFAULT_PROJECT` | 单独建单的默认仓库，格式为 `owner/repo` |
| `--plane-url` | `PLANE_URL` | Plane 实例根地址，可带安装子路径 |
| `--plane-token-file` | `PLANE_TOKEN_FILE` | Plane 个人 API key 文件路径 |
| `--plane-workspace` | `PLANE_WORKSPACE` | Plane 搜索和单独建单使用的默认工作空间 slug |
| `--plane-default-project` | `PLANE_DEFAULT_PROJECT` | 单独建单的默认项目标识、名称或 UUID |
| `--github-url` | `GITHUB_URL` | GitHub 网站根地址（自动选择 REST API 地址） |
| `--github-token-file` | `GITHUB_TOKEN_FILE` | GitHub 个人 token 文件 |
| `--github-default-project` | `GITHUB_DEFAULT_PROJECT` | 单独建单的默认仓库，格式为 `owner/repo` |
| `--gitlab-url` | `GITLAB_URL` | GitLab 实例根地址 |
| `--gitlab-token-file` | `GITLAB_TOKEN_FILE` | GitLab 个人 token 文件 |
| `--gitlab-default-project` | `GITLAB_DEFAULT_PROJECT` | 单独建单的默认项目路径，如 `group/subgroup/project` |
| `--jira-url` | `JIRA_URL` | Jira Cloud 网页站点根地址 |
| `--jira-email` | `JIRA_EMAIL` | Atlassian 账号邮箱 |
| `--jira-token-file` | `JIRA_TOKEN_FILE` | Jira API token 文件 |
| `--jira-cloud-id` | `JIRA_CLOUD_ID` | Scoped token 的 Cloud ID，选择官方 API 网关 |
| `--jira-issue-type` | `JIRA_ISSUE_TYPE` | Jira 创建类型名称或 ID，默认 `Task` |
| `--jira-link-type` | `JIRA_LINK_TYPE` | Jira 阻塞关系类型名，默认 `Blocks` |
| `--jira-default-project` | `JIRA_DEFAULT_PROJECT` | 单独建单的默认项目 key |

每个后端的地址和凭据文件必须同时提供。MCP 模式至少配置一个后端，可以同时配置多个平台。Jira 还需账号邮箱；scoped token 需 Cloud ID。Plane 未设置默认工作空间时，搜索调用需显式提供 `workspace`。

凭据文件放在仓库外，内容为 token 本身，允许末尾换行，总大小不超过 4096 字节。Unix 上使用 `0600` 或 `0400`，其他平台通过系统权限限制访问。实例地址使用 HTTPS，回环地址也允许 HTTP；URL 不能携带用户名、密码、查询参数或片段，客户端不跟随重定向。

## 命令

| 命令 | 行为 |
| --- | --- |
| `mergelane mcp` | 从标准输入读取按行 JSON-RPC，向标准输出返回 MCP 响应 |
| `mergelane init` | 配置一个平台，保留其他平台，打印客户端配置片段 |
| `mergelane doctor` | 验证配置和凭据后检查当前账号接口，不执行发布 |
| `mergelane doctor --offline` | 只验证本地设置与凭据文件 |
| `mergelane pull` | 拉取分给当前账号的未完成工单和任务 |
| `mergelane gitea-context 'team/web#42'` | 输出选定 Gitea 工单、阻塞关系、评论与资料版本信息 |
| `mergelane gitea-watch 'team/web#42'` | 轮询选定的 Gitea 工单，输出发生变化的 JSON 行 |

`gitea-context` 和 `gitea-watch` 需要 Gitea 地址与凭据。Watch 默认间隔 60 秒，`--interval` 可设为 10–3600 秒，一次选择 1–32 张工单。退出时使用 `Ctrl+C`。

完整参数以当前程序帮助为准：

```sh
mergelane --help
mergelane mcp --help
mergelane gitea-watch --help
```

## 每日拉取

`pull` 每次读取一份当前快照。默认汇总五种已支持平台中当前已配置账号的待办。指定项目后，读取该项目所有可见的未完成工单，包括未指派工单。

MCP 客户端连接 `mergelane mcp` 后，可以让 Agent 调用只读工具 `mergelane_pull_tasks`。传 `{}` 拉取「我的待办」，传 `{"backend":"github","project":"owner/repo"}` 拉取指定项目。工具沿用同一套账号配置和快照格式，在 `mcp --read-only` 下也可用。

```sh
mergelane pull
mergelane pull --backend github --project owner/repo
mergelane pull --backend plane --project BILL
```

项目筛选使用 Gitea 或 GitHub 仓库路径、GitLab 带命名空间的项目路径、Jira 项目 key，或 Plane 的项目标识、名称或 UUID。Plane 需要配置默认工作空间。快照包含标题、链接、项目、状态和负责人。分页平台默认每个来源最多查询 10 页，可用 CLI 的 `--max-pages` 或 MCP 的 `max_pages` 调整。Plane 每个来源最多返回 50 条，可缩小到单个项目。查询失败、超时或截断时仍返回已取得的结果，并标记 `complete: false`。CLI 同时以非零状态退出。

每天拉取可由支持定时调用的 MCP 客户端触发，也可使用本机定时任务运行 CLI。macOS 或 Linux 可以在 `crontab -e` 中配置每天 09:00 拉取，时间以机器本地时区为准。

```cron
0 9 * * * umask 077; mkdir -p "$HOME/.mergelane" && /absolute/path/to/mergelane pull > "$HOME/.mergelane/daily-tasks.tmp" && mv "$HOME/.mergelane/daily-tasks.tmp" "$HOME/.mergelane/daily-tasks.json"
```

将可执行文件改为安装位置。若账号保存在自定义配置路径，加上 `--config /absolute/path/to/profile.json`。只有完整拉取才替换上一次快照，部分结果留在临时文件。Windows 可在任务计划程序中每天运行相同命令，并用 PowerShell 将输出保存到用户目录。定时任务按本机调度器和开机状态运行。

## 读取与写入

默认提供每个已配置后端的 `*_search_issues`、`*_issue_context` 和 `*_preview_plan`。预览只读取远端，最新预览保留在当前 MCP 会话。

`--gitea-write`、`--plane-write`、`--github-write`、`--gitlab-write`、`--jira-write` 各自额外提供对应平台的 `*_create_issue`、`*_assign_issue`、`*_edit_issue`、`*_apply_plan` 和 `*_checkpoint`。五个开关相互独立，并且仍受远端账号权限与客户端审批控制。发布计划需匹配当前会话的最新 `plan_id`，写入需调用方提供确认标记。使用 `mergelane init --default-project <项目>` 时，个人配置会为所选平台保存 `default_project`。团队流程见 [协作指南](collaboration.md)。

Plane 的 `plane_preview_parent_checkpoint` 可预览发往原生父工单的子工单进度，开启 `--plane-write` 后可通过 `plane_publish_parent_checkpoint` 发布已确认的预览。[向父工单汇报子工单进度](plane.md#向父工单汇报子工单进度)

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
