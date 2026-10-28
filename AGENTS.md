# AGENTS.md

本文件面向在本仓库中工作的 AI 编程助手，说明项目结构、提交方式、发版流程与注意事项。

## 项目简介

**办公工具箱（office-toolbox）**——Windows／macOS 本地办公工具宿主，基于 **Tauri 2 + Vanilla TypeScript + Rust**。

- 通过 `office-plugin.json` 发现和导入独立进程插件。
- 支持插件启停、运行规则、日志、托管启动和独立启动。
- 内置「速启」与「共享剪贴板」，二者保留原有窗口、数据和独立构建方式。
- 主程序只管理插件，不合并插件的托盘、全局快捷键、单实例和 Rust 后端。

## 技术栈与目录

| 项目 | 技术 / 用途 |
|---|---|
| 主程序前端 | Vite 6 + TypeScript，原生 DOM |
| 主程序后端 | Rust + Tauri 2 |
| 主程序包管理 | **npm**，锁定文件为 `package-lock.json` |
| `plugins/suqi`、`plugins/lscopy` | 各自沿用原项目的 **bun** 和独立构建体系 |
| `scripts/prepare-bundled-plugins.mjs` | 生成发布用插件源码，排除依赖、缓存和数据 |
| `.generated/plugins/` | 自动生成的打包资源，不手工编辑、不提交 |
| `.github/workflows/release.yml` | Windows 与 macOS Release 构建发布 |

## 常用命令

```bash
npm install
npm run dev                 # 仅启动网页预览
npm run build               # TypeScript + Vite，提交前必跑
npm run tauri dev           # 完整桌面应用
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

插件目录仍使用原项目命令，例如 `cd plugins/suqi && bun install && bun run build`。不要用主程序的 npm 依赖替代插件自己的依赖。开发宿主必须从 `plugins/` 源码目录启动内置插件，主程序／速启／共享剪贴板前端端口固定为 `1420`／`1431`／`1441`，不要复用同一端口。

## 代码与文档约定

- 前端保持原生 TypeScript，不随意引入 UI 框架；后端功能按现有分区维护。
- 注释、界面文案、提交说明和 Release notes 使用中文。
- 新增功能时同步更新 `README.md`；新增插件字段时同步更新清单说明和校验。
- 能力规则通过环境变量和规则文件下发，需插件主动适配，**不是操作系统沙箱**。
- 主程序规则存储在 `%LOCALAPPDATA%/OfficeToolbox/`；插件数据仍由插件自己管理。
- 导入插件时复制到工具箱数据目录，不修改用户原始插件目录。

## 提交前检查

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run release:check
```

如果修改了插件，还需进入对应插件目录执行其 `bun run build` 和 Rust 检查。桌面交互改动应真机验证，不要用网页预览代替托盘、快捷键、进程启停等桌面验收。

**助手不得在本地打包本仓库的 Windows 安装包**，包括主程序和插件的 Windows Tauri 打包命令。Windows 安装包只通过 GitHub Actions 标签发版构建；本地仅做前端、测试、Clippy／`cargo check` 和必要的桌面运行验证。

## 提交与推送

沿用速启的方式：功能与文档一起提交到 `main`，版本标签基于已推送的提交创建。

```bash
git add -A
git commit
git push origin main
```

提交前检查暂存内容，不要提交密钥、证书、token、用户数据、日志、数据库、`node_modules/`、`dist/`、`target/` 或 `.generated/`。

## 发布流程

1. 同步版本号：`package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`；`Cargo.lock` 随构建更新。
2. 执行提交前检查，并用 `npm run release:check -- vX.Y.Z` 核对标签和版本。
3. 提交并推送 `main`，然后创建并推送标签：

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

4. `v*` 标签触发 `.github/workflows/release.yml`，产出 Windows NSIS 安装包、便携版 exe、macOS Apple Silicon／Intel DMG，以及分平台的 `sha256sums-windows.txt`、`sha256sums-macos-*.txt` 和对应 MD5 文件。Release 资产使用 ASCII 文件名，避免中文文件名被上传工具剥掉。
5. **构建结果由用户自行查看确认**。助手不要通过 GitHub API、gh CLI、轮询或日志下载查询 Actions 状态；推完标签即停下，等用户主动告知结果。
6. 用户确认成功后，新建 `release-notes-X.Y.Z.md`，更新 Release 说明：

   ```bash
   GITHUB_REPOSITORY=owner/repo GH_TOKEN=... npm run release:notes -- vX.Y.Z release-notes-X.Y.Z.md
   ```

   未设置 `GH_TOKEN` 时，脚本会尝试从 Git 凭据管理器读取；任何时候不得明文打印 token。

## 签名与发布注意事项

- Windows 代码签名为可选项。配置仓库 secrets `WINDOWS_CERT_PFX`（PFX 的 base64）和 `WINDOWS_CERT_PASSWORD` 后，CI 会注入 `signCommand` 并调用 `scripts/sign-windows.ps1`。
- macOS 当前仅配置 Apple Silicon／Intel 构建，未签名、未公证；正式公开发布前需要另行配置 Apple Developer ID 签名、公证、staple 和真机验收。
- 不要把证书、私钥或密码写入仓库、日志或 `tauri.conf.json`。
- 当前主程序没有接入 updater，因此发布流程**不生成 `latest.json`，也不需要 Tauri updater 私钥**。以后接入自动更新时再单独设计签名密钥和更新清单，不能直接复用速启的仓库地址。
- 发布资源必须由 `scripts/prepare-bundled-plugins.mjs` 生成，避免把插件依赖、构建缓存、日志和数据库打进安装包。
- 当前随包的是插件源码；插件已有可执行文件时优先运行，否则回退到开发命令并需要对应 Bun／Rust 环境。若要求最终用户零开发环境运行，需要另行实现插件预编译产物打包。
- GitHub 推送默认直连。直连失败时只可临时使用明确指定的代理命令，不要把本机代理写回全局 Git 配置。
- 插件开发若遇到 Bun shim 问题，确认 `where bun` 第一条指向真实 `bun.exe`，不要通过修改项目代码绕过环境问题。
