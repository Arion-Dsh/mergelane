# 文档目录

MergeLane 让多人通过各自的 Agent，共享任务、分工、依赖和进度。任务平台是团队共同读取和维护的事实源，每位成员保留自己的客户端、凭据和工作区。

## 使用

| 文档 | 适合什么时候阅读 |
| --- | --- |
| [项目说明与安装](../README.zh-CN.md) | 第一次了解项目、安装命令行工具 |
| [安装与首次配置](installation.md) | Homebrew、安装包、init 向导和 doctor |
| [团队协作流程](collaboration.md) | 建立团队的拆分、分派、开发、进度查询和协同方式 |
| [Gitea 接入](gitea.md) | 配置凭据与客户端、使用工单和 Git 文件版本跟踪 |
| [Plane 接入](plane.md) | 配置工作空间、成员、工单和跨项目依赖 |
| [GitHub 接入](github.md) | 配置个人凭据、仓库 Issues 和原生依赖 |
| [GitLab 接入](gitlab.md) | 配置命名空间项目、成员 ID 和原生阻塞关系 |
| [Jira Cloud 接入](jira.md) | 配置邮箱、scoped token、项目和 accountId |
| [配置与排错](configuration.md) | 查环境变量、命令、写入开关与常见错误 |

## 开发与维护

| 文档 | 内容 |
| --- | --- |
| [贡献指南](../CONTRIBUTING.md) | 本地开发、验证、问题反馈与 PR 要求 |
| [发布与 Homebrew 分发](releasing.md) | Release、Tap、凭据和发布后验收 |
| [架构与后端扩展](architecture.md) | 模块边界、数据流、计划恢复和新增平台的步骤 |
| [安全指引](../SECURITY.md) | 凭据、信任边界和漏洞报告 |
| [许可证](../LICENSE) | MIT License |

支持 Gitea、Plane、GitHub、GitLab 和 Jira Cloud。各平台的原生依赖、身份和权限有差异，接入前阅读对应能力限制；多平台同时配置不代表自动跨平台同步工单。
