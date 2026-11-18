# 贡献指南

欢迎使用中文或英文提交问题、文档改进和代码变更。贡献按项目的 [MIT 许可证](LICENSE) 提供。

## 本地开发

1. 使用 rustup 安装 Rust。进入仓库后，[rust-toolchain.toml](rust-toolchain.toml) 会选择开发工具链及 rustfmt、Clippy。
2. 阅读 [README](README.md)、[团队协作流程](docs/collaboration.md)、[架构说明](docs/architecture.md) 和本次涉及的接入文档。完整入口见 [文档目录](docs/README.md)。
3. 使用 `cargo run -- --help` 查看 CLI，用 `cargo run -- mcp --help` 查看连接参数。

`Cargo.lock` 随仓库维护，开发和 CI 使用 `--locked`。只有需要更新依赖的变更才修改锁文件，并在 PR 中说明原因。不要把 token、真实工单内容或个人 MCP 配置加入仓库；`.env.example` 只提供变量名与占位示例，不会被程序自动加载。

## 修改与验证

保持 CLI、MCP 协议、计划校验和外部 API 的边界。新增工具或修改配置时，同步更新工具 schema、接入文档及中英文 README 中受影响的说明。标准输出是 MCP 协议或 CLI JSON 输出通道，诊断信息不能混入其中。

| 要修改的内容 | 优先阅读的位置 |
| --- | --- |
| CLI 参数、连接配置、凭据读取 | `src/cli.rs`、`src/credentials.rs` |
| MCP 协议和后端分发 | `src/mcp.rs` |
| 某个平台的工具、预览或计划校验 | `src/<backend>/mcp.rs`、`src/<backend>/plan.rs` |
| 某个平台的 API、查询、发布与恢复 | `src/<backend>.rs` 及其 `search.rs`、`write.rs`、`references.rs` |
| 多会话协作与远端 API 行为 | `tests/gitea.rs`、`tests/plane.rs`、`tests/trackers.rs` 与 `tests/support/` |

新增其他任务平台时，先遵循 [后端扩展步骤](docs/architecture.md#扩展其他后端)，再根据实际共同语义决定是否提取公共抽象。现有 REST 平台通过 `tracker::Api` 和 `tracker::Session` 复用协作流程；后端之间不应为复用普通校验而互相依赖。

提交 PR 前运行：

```sh
cargo fmt --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

集成测试会启动本地子进程，并监听 `127.0.0.1` 随机端口。受限沙箱需允许本地监听；测试无需连接真实任务平台或提供凭据。首次下载依赖后，可在上述 Cargo 构建与测试命令中增加 `--offline`。

测试应验证可观察的行为与失败后的恢复。涉及发布计划、评论、权限或分页时，覆盖失败路径，使用假账号、稳定的测试数据和临时目录。文档变更不需要新增实现细节测试。

需要真实实例验收时，使用专用测试项目和个人测试凭据，显式启用写入并确认计划。PR 中分别记录模拟 API 测试与真实实例验收的结果；后者不能由前者代替。

## 报告问题

普通问题使用仓库的 Issue 模板，描述期望行为、实际行为、版本和最小复现。删除 token、私人实例地址、工单正文和个人路径。安全漏洞按 [SECURITY.md](SECURITY.md) 私下报告。

## 提交 Pull Request

- 说明解决的问题、最终行为和受影响的接口；一个 PR 聚焦一个问题。
- 列出实际运行的检查及结果；未完成的检查说明原因。
- 接口或行为变化给出迁移方式，必要时补充回归测试。
- 保留远端计划标记和检查点的兼容性；变更其格式时说明重试与接手影响。
- 不夹带依赖升级、生成文件、私密配置或无关格式化。

## 发布前检查

由维护者确认版本号和许可证、文档链接、配置示例、依赖变更及 CI 结果。发布前运行 Release 的 `dry-run`，核对五个平台的原生构建以及后续 Package smoke 归档与 Homebrew 安装检查。确认仓库已配置安全问题的私下报告渠道，再公开发布。首次发布前检查完整待提交文件和 Git 历史中的凭据；忽略规则不会删除历史中的秘密。

`publish = false` 禁止 crates.io 发布，二进制分发由固定版本 cargo-dist 管理。修改 `dist-workspace.toml` 后运行 `dist generate` 和 `dist generate --check`；构建前检查在 `.github/release-check-steps.yml` 中维护。Tap、Actions Secret、手动发布和验收步骤见 [发布指南](docs/releasing.md)。
