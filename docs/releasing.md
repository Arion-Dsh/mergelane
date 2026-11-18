# 发布与 Homebrew 分发

主仓库为 [Arion-Dsh/mergelane](https://github.com/Arion-Dsh/mergelane)，本地 origin 使用 `git@github.com:Arion-Dsh/mergelane.git`。

固定使用 cargo-dist 0.34.0，[dist-workspace.toml](../dist-workspace.toml) 是发布配置，[release.yml](../.github/workflows/release.yml) 为生成的工作流。维护者手动运行 Release：计算计划 → 各平台检查与构建 → 汇总安装器与校验和 → GitHub Release → 更新 Homebrew Tap。默认输入 `dry-run` 只构建安装包与安装器；指定 `v0.1.0` 才会正式发布。推送标签不会触发发布，PR 默认只检查发布计划。

| 目标 | 安装包 |
| --- | --- |
| Apple Silicon macOS | `mergelane-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mergelane-x86_64-apple-darwin.tar.xz` |
| Linux ARM64 | `mergelane-aarch64-unknown-linux-gnu.tar.xz` |
| Linux x64 | `mergelane-x86_64-unknown-linux-gnu.tar.xz` |
| Windows x64 | `mergelane-x86_64-pc-windows-msvc.zip` |

归档包含程序、README、LICENSE，并生成 SHA-256、安装脚本和 Homebrew Formula。工作流启用 GitHub Artifact Attestations，实际证明在 Actions 发布时生成。源码归档需要已有 Git 提交，没有 HEAD 的本地仓库无法完成全局打包。

## 首次发布前

1. 审查并提交源码，推送到主仓库。
2. 创建并初始化公开仓库 `Arion-Dsh/homebrew-tap`，使用 [Tap 初始文件](../packaging/homebrew-tap/README.md) 和其中的 `.github/workflows/install.yml`。初始仓库不放占位 Formula；正式发布时由 cargo-dist 写入。
3. 在主仓库设置 Actions Secret `HOMEBREW_TAP_TOKEN`，使用仅可访问 Tap 仓库、具有 Contents 读写权限的凭据。主仓库 Release 使用内置 `GITHUB_TOKEN`。不得把 token 写入源码或普通配置。
4. 确认 Actions、Artifact Attestations、公开 Release 可用，并配置私下漏洞报告渠道。
5. 核对版本号、[更新日志](../CHANGELOG.md)、自动化测试及各系统构建结果，完成下面的发布演练。

v0.1.0 暂缓 GitHub、GitLab、Jira 的真实账号接入验收，不作为首版发布前置条件。这三个适配器已通过本地模拟 API 测试，真实环境仍未验证；发布说明应保留这一状态。自动化测试、各系统构建和安装分发验证仍需完成。

正式发布的构建先检查 Tap token 与仓库可访问性，再执行格式、编译、Clippy 和测试；`dry-run` 无需 Tap Secret。实际写入权限仍须正确授予。`publish = false` 禁止 crates.io 发布，`package.metadata.dist.dist = true` 显式允许二进制分发。

## 发布演练

在 GitHub Actions 的 Release 页面选择 `main`，保持 `tag` 为 `dry-run`，或运行：

```sh
gh workflow run release.yml --repo Arion-Dsh/mergelane --ref main -f tag=dry-run
```

这会在五个原生 runner 上运行格式、编译、Clippy、测试和归档构建，随后生成带真实归档校验和的 Formula、shell 和 PowerShell 安装器。产物保存在 Actions Artifacts，不创建 Release 或更新 Tap。

成功后自动触发 [Package smoke](../.github/workflows/package-smoke.yml)，按同一份 dist 计划选择原生 runner：核对归档 SHA-256、解包、运行版本与帮助命令、使用临时假凭据执行 `init`、离线 `doctor` 和只读 MCP 握手。两个 macOS runner 还会通过临时 Tap 安装生成的 Formula，并运行同样的 CLI 检查。此阶段只把下载地址替换为本机归档服务，保留原始校验和；验证结束清理测试安装和 Tap。它不能代替发布后从正式 GitHub URL 安装的检查。

`Package smoke` 只接受主仓库 `main` 上维护者手动运行的成功构建，权限仅为读取源码和构建产物，不执行 PR 产物。演练与 Package smoke 均成功后，再开始正式发布。

## 修改和验证配置

安装固定版本 cargo-dist 后运行：

```sh
dist generate
dist generate --check
dist plan --tag v0.1.0
```

标签应匹配 Cargo.toml 的版本号。自定义构建检查放在 [.github/release-check-steps.yml](../.github/release-check-steps.yml)，通过 `github-build-setup` 注入；不要手改生成的工作流。升级 dist 后重新生成并审阅变更。

本机可执行 `dist build --artifacts=local --target aarch64-apple-darwin` 验证对应归档。这不能代替其他平台、全局安装器或真实 Homebrew 安装测试。完整全局打包需要有效 HEAD。

准备好后由维护者确认待发布的 `main` 提交与演练一致，再手动运行 Release 并填写匹配 Cargo.toml 的 `v0.1.0`。该操作创建版本标签、公开 Release，并更新稳定 Tap；预发布版本默认不更新稳定 Tap。

```sh
gh workflow run release.yml --repo Arion-Dsh/mergelane --ref main -f tag=v0.1.0
```

## 发布后验收

确认所有任务、Release 和 Tap 更新成功，并核对 Tap 的 Homebrew install 工作流：它在 Apple Silicon 和 Intel macOS 上通过正式下载地址安装刚更新的 Formula，并运行版本和帮助命令。随后在干净环境测试 `brew install Arion-Dsh/tap/mergelane`、`mergelane --version`、`mergelane init` 与 `mergelane doctor`，并核对各平台校验和及升级后的 MCP 路径。

参考：[Homebrew 分发](https://axodotdev.github.io/cargo-dist/book/installers/homebrew.html)、[构建前自定义步骤](https://axodotdev.github.io/cargo-dist/book/ci/customizing.html)、[维护 Tap](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap)。
