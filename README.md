# MergeLane

[English](README.en.md) · 简体中文

**多人通过各自的 Agent，共享任务、分工、依赖和进度。**

MergeLane 为使用 Gitea、Plane、GitHub、GitLab 或 Jira Cloud 的团队提供 MCP 协作工具。负责人拆分目标并分派任务，队员通过自己的 Codex、Claude Code 或 Cursor 读取需求并开展工作，管理者随时查询同一份任务和进展。

MergeLane 是运行在开发机上的 MCP stdio 命令。团队已有的任务平台保存工单、分工和检查点；计划预览暂存在当前 MCP 会话内存中，无需额外部署 MergeLane 服务或数据库。

## 能做什么

- 按任务、项目或队员查询进度，读取需求、阻塞关系和检查点。
- 用 MCP 工具 `mergelane_pull_tasks` 或命令 `mergelane pull` 拉取自己的待办或指定项目的未完成工单。[每日拉取说明](docs/configuration.md#每日拉取)
- 用标题创建未指派的工单，把链接交给管理者分配负责人。[先录入，再分配](docs/collaboration.md#先录入再分配)
- 按工单链接修改标题或描述，确认后保存到任务平台。[编辑工单](docs/collaboration.md#编辑工单)
- 先预览拆单计划，确认后创建任务、指派成员并建立原生依赖关系。
- 开工时领取本地租约，防止同一台电脑上的多个 Agent 会话通过 MergeLane 同时领取同一工单。
- 在工单评论中发布检查点，供其他成员或新的 Agent 会话接手。
- 对比工单引用的 Gitea 固定版本文件与默认分支，提示资料变更。
- 在部分发布失败后，使用相同计划继续补齐任务、依赖和汇总。

五个平台均可独立使用或同时配置，各自保留个人凭据、写入开关和计划预览。

| 平台 | 查询、分派与进度 | 原生任务阻塞关系 | Git 文件版本比较 |
| --- | --- | --- | --- |
| [Gitea](docs/gitea.md) | 支持 | 需仓库启用依赖 | 支持 |
| [Plane](docs/plane.md) | 支持 | 支持 | 配合 Gitea |
| [GitHub](docs/github.md) | 支持 | 需实例提供依赖 API | 暂无 |
| [GitLab](docs/gitlab.md) | 支持 | 需对应套餐与权限 | 暂无 |
| [Jira Cloud](docs/jira.md) | 支持 | 需启用相应链接类型 | 暂无 |

新接入的 GitHub、GitLab、Jira 已有本地模拟 API 验证，尚未进行真实账号验收；具体 API、权限与能力限制见各自接入文档。

## 开始使用

普通用户只需要支持 MCP stdio 的客户端和可访问的任务平台。首版通过 [MergeLane 自建 Tap](https://github.com/Arion-Dsh/homebrew-tap) 分发 macOS 预编译包。正式发布后，使用完整包名即可安装：

```sh
brew install Arion-Dsh/tap/mergelane
mergelane init
mergelane doctor
```

**Tap 已初始化，首个 Release 和 Formula 发布后此命令可用。** 完整操作见 [安装与首次配置](docs/installation.md)。当前源码安装需要 Rust 1.97.0，工具链在 [rust-toolchain.toml](rust-toolchain.toml) 固定。

1. 在源码目录安装：

   ```sh
   cargo install --locked --path .
   mergelane --help
   ```

2. 创建自己的实例凭据，存入仓库外的文件；Unix 上将权限设为 `0600` 或 `0400`。程序只接受 HTTPS 或回环 HTTP，支持实例安装子路径。
3. 从 [文档目录](docs/README.md) 选择团队使用的平台接入说明，把 `mergelane` 的绝对路径配置到 MCP 客户端，可同时配置多个平台。默认对远端只读。发布任务或检查点需显式添加对应的 `--<平台>-write`。

[.env.example](.env.example) 列出可用环境变量。MergeLane **不会自动加载 `.env`**；请在启动进程前导出变量，或在 MCP 客户端中设置它们。凭据通过文件读取，不直接写入配置或命令参数。

## 团队如何使用

| 角色 | 通过自己的 Agent 完成的工作 |
| --- | --- |
| 负责人 | 读取总任务、预览拆分、确认负责人和依赖、发布计划 |
| 队员 | 查询任务、核对依赖、领取、开发、发布检查点并释放 |
| 管理者 | 按任务、项目或队员查询状态，读取阻塞原因与最近进展 |

- 负责人：“读取这张总工单，把它拆成可分派的任务。先给我看计划。”
- 管理者：“查账单导出的进度”“Alice 手上的任务卡在哪里？”
- 队员：“做这张工单，先检查前置任务和资料版本。”
- 准备接手：“读取这张工单的最近检查点，从对应提交继续。”
- 依赖团队维护的资料时，把 Gitea 中该文件的固定提交链接贴到工单描述。

计划发布需要当前会话中的最新预览 ID 和确认标记。确认标记由调用方传入；团队应在客户端保留人工审批。发布不是跨系统事务，失败时可能已经创建部分工单；检查实例状态后，用相同计划重试，同一计划由一位负责人发布。

完整示例见 [团队协作流程](docs/collaboration.md)。安装、配置、操作与开发资料统一收录在 [文档目录](docs/README.md)。

## 项目结构与贡献

| 路径 | 内容 |
| --- | --- |
| `src/main.rs` | 进程入口、命令分发与退出 |
| `src/cli.rs`、`src/credentials.rs` | 参数和连接配置、凭据文件读取 |
| `src/mcp.rs` | 公共 MCP 协议与后端分发 |
| `src/gitea.rs`、`src/gitea/` | Gitea API、工具、计划、查询、写入和监听 |
| `src/plane.rs`、`src/plane/` | Plane API、工具、计划、查询和写入 |
| `src/github.rs`、`src/gitlab.rs`、`src/jira.rs` | GitHub、GitLab、Jira 的认证、标识、查询和原生关系 |
| `src/tracker/`、`src/tracker.rs` | 三个 REST 后端共用的计划、发布、检查点和有界 HTTP |
| `src/validation.rs` | 后端共用的纯文本校验 |
| `tests/`、`tests/support/` | 本地模拟 API 集成测试及共用 MCP 测试客户端 |
| `docs/` | 接入说明、架构与维护文档 |

参与开发请读 [贡献指南](CONTRIBUTING.md) 和 [架构说明](docs/architecture.md)。安全问题与凭据处理见 [SECURITY.md](SECURITY.md)。

## 开发验证

```sh
cargo fmt --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

集成测试使用临时目录、假凭据和回环地址上的随机端口，无需真实实例或账号，验证拆单、分派、部分失败后恢复、跨会话共享和版本检查。CI 配置覆盖 Linux、macOS 和 Windows；真实实例与客户端验收范围见接入文档。

## 许可证

本项目采用 [MIT License](LICENSE)。当前可从源码安装；二进制 Release 与 Homebrew 工作流已准备，尚未实际发布。crates.io 发布保持关闭。
