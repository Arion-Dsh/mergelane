# 安装与首次配置

普通用户使用预编译安装包，无需安装 Rust；源码安装保留给开发者。

## Homebrew 与安装包

首版使用 [MergeLane 自建 Tap](https://github.com/Arion-Dsh/homebrew-tap)。首次正式 Release 与 Formula 发布后，Mac 用户可以直接用完整包名安装：

```sh
brew install Arion-Dsh/tap/mergelane
mergelane --version
```

支持 Apple Silicon 和 Intel Mac，升级用 `brew upgrade Arion-Dsh/tap/mergelane`。Linux ARM64／x64 和 Windows x64 用户从 [Releases](https://github.com/Arion-Dsh/mergelane/releases) 下载对应归档并核对 SHA-256，或使用该版本页面提供的 shell／PowerShell 安装器。

**Tap 已初始化，正在等待首个 Release 和 Formula。** 发布配置见 [发布指南](releasing.md)。在本地源码目录可用 `cargo install --locked --path .` 安装，需要 Rust 1.97.0。

Windows 用户可在 PowerShell 中运行 `Get-FileHash <下载的 ZIP 路径> -Algorithm SHA256`，与同名 `.sha256` 文件比较后解压使用。当前生成的 PowerShell 安装器不内置归档校验；需要校验时使用 ZIP 下载方式。

## 首次配置

先在平台创建个人 token，保存到仓库外的文件；Unix 权限设为 `0600` 或 `0400`，不要把 token 值放进命令参数。

```sh
mergelane init
```

向导询问平台、地址、token 文件路径和平台专属设置，默认对远端只读。再次运行可添加另一个平台，保留已有平台；替换同一平台必须使用 `--replace`。要启用发布工具，显式加 `--write`，实际发布仍需确认。设置完成后可在 Agent 内按 [本地任务租约](collaboration.md#本地任务租约) 开工。

非交互示例：

```sh
mergelane init --backend github --token-file /absolute/path/github-token --non-interactive
mergelane init --backend plane --url https://plane.example.com --token-file /absolute/path/plane-token --workspace team --non-interactive
```

GitHub／GitLab 默认使用官方站点，其他平台需填写 URL。Jira 使用 `--email`，scoped token 还需 `--cloud-id`；可用 `--issue-type`、`--link-type` 调整类型。具体权限见对应接入文档。

## 保存位置与客户端配置

默认配置路径依次选择：`XDG_CONFIG_HOME/mergelane/config.json`、Windows 的 `APPDATA/mergelane/config.json`、`~/.config/mergelane/config.json`。`--config <路径>` 可显式指定。

配置有版本号，只保存平台设置与 token 文件路径，不复制 token 内容。相对 token 路径按配置文件所在目录解析；向导保存的是绝对路径。更新采用文件锁和原子替换。

`init` 默认在标准输出打印 Codex TOML；使用 `--client claude` 或 `--client cursor` 打印 `mcpServers` JSON。将片段合入客户端配置并重新启动 MCP 进程。程序不修改客户端文件。片段指向同一份个人设置，增加平台后无需重写所有启动参数。

Homebrew 安装优先使用稳定的 `opt/mergelane/bin/mergelane` 路径，避免升级后指向旧版 Cellar 目录。

## 检查是否可用

```sh
mergelane doctor
mergelane doctor --offline
```

`doctor` 验证配置和凭据文件后，请求各平台当前账号接口，输出 JSON 结果；失败返回非零退出码。文件解析失败时先在标准错误给出原因。它不创建任务、不发布评论，也不测试写入权限。

`--offline` 完全跳过网络，不能确认 token 是否有效。连接成功也不代表能访问某个项目，仍需结合工单和账号权限核对。

`mcp` 和 `doctor` 自动加载默认设置，也可使用 `--config`。相同字段的优先级为显式 CLI 参数、保存设置、环境变量；环境仍可补充未配置的平台。`--no-config` 忽略保存设置。`mcp --read-only` 禁用所有写入工具，包括设置中开启的写入。
