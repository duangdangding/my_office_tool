# 办公工具箱

一个可扩展的 Windows／macOS 本地办公工具箱，基于 Tauri 2、TypeScript 和 Rust。插件以独立进程运行，工具箱负责导入、规则、启停和日志，不接管插件自己的窗口与数据。

## 内置插件

- **速启**：全局快捷启动面板，源码位于 `plugins/suqi/`。
- **共享剪贴板**：局域网／中继剪贴板同步，源码位于 `plugins/lscopy/`。

两个插件都保留原有构建和独立运行方式。第三方插件可通过根目录的 `office-plugin.json` 描述入口、能力声明和启动命令，再从工具箱界面导入。

> 能力规则是插件间的协作约定，需要插件主动适配，并非操作系统级沙箱。

## 启动状态与就绪信号

点击启动后，插件会先进入「启动中」，初始化完成后才显示「运行中」。开发启动最长等待 30 分钟，首次编译不会过早被判定为成功；启动或编译失败会显示详细错误和最近 64 KB 日志，也可以取消启动。

插件应在自身全部初始化完成后，向标准输出写入一行就绪信号（`id` 替换为清单中的插件 id）：

```text
OFFICE_TOOLBOX_READY:suqi
```

两个内置插件已适配。未提供信号的单文件可执行程序会在进程存活 2 秒后按运行中处理；开发命令必须提供信号，避免长时间编译被误判。工具箱每秒同步进程状态，插件手动退出后会更新为未运行或异常退出；退出工具箱会停止所有托管插件，独立模式插件不受影响。

## 系统托盘

主程序启动后显示系统托盘图标。单击图标或选择「显示办公工具箱」可恢复主窗口，右键菜单支持显示、隐藏和退出。点击窗口关闭按钮只会隐藏到托盘；从托盘选择「退出」才会结束主程序并停止托管插件。

## 开发

主程序使用 npm：

```bash
npm install
npm run dev
npm run tauri dev
```

构建与检查：

```bash
npm run build
node scripts/prepare-bundled-plugins.mjs
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

**不要在本地打包 Windows 安装包**。本仓库的 Windows 安装包只通过 GitHub Actions 标签发版构建；本地验证不执行主程序或插件的 Windows Tauri 打包命令。

插件仍使用各自的 Bun 环境，例如：

```bash
cd plugins/suqi
bun install
bun run build
bun run tauri build
```

开发模式会优先从 `plugins/` 源码目录启动内置插件，不使用 `target/debug/plugins/` 里的残留副本。主程序、速启和共享剪贴板的开发前端分别使用 `1420`、`1431`、`1441` 端口，避免插件窗口加载到主程序页面。

**必须通过 Tauri CLI 构建桌面程序**，不要用裸 `cargo build --release` 代替，否则前端资源和 `custom-protocol` 特性可能未正确处理。

## 提交方式

功能、代码和对应文档放在同一次提交中。提交前至少运行 `npm run build`、Rust 测试和 Clippy；修改了插件时，还要运行该插件自己的检查。

```bash
git add -A
git commit
git push origin main
```

不要提交 `node_modules/`、`dist/`、`target/`、`.generated/`、日志、数据库、证书或任何密钥。

## 发版流程

### 1. 同步版本并验证

修改以下文件中的版本号：

- `package.json`
- `package-lock.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.lock` 由 Cargo 构建更新

然后执行：

```bash
npm run build
node scripts/prepare-bundled-plugins.mjs
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run release:check -- v0.1.0
```

### 2. 推送提交和版本标签

```bash
git add -A
git commit
git push origin main
git tag v0.1.0
git push origin v0.1.0
```

推送 `v*` 标签会触发 `.github/workflows/release.yml`，并行构建 Windows 和 macOS，再上传到同一个 GitHub Release。

发布产物：

- `office-toolbox_X.Y.Z_x64-setup.exe`：Windows NSIS 安装包。
- `office-toolbox_X.Y.Z_x64_portable.exe`：Windows 便携版。
- `office-toolbox_X.Y.Z_arm64.dmg`：macOS Apple Silicon 版本。
- `office-toolbox_X.Y.Z_x86_64.dmg`：macOS Intel 版本。
- `sha256sums-windows.txt`、`sha256sums-macos-*.txt` 及对应 MD5 文件：各平台校验文件。

发布资产统一使用 ASCII 文件名，避免中文名称在上传时被移除。标签、三处项目版本和锁文件版本必须一致，否则 CI 会在构建前停止。

### 3. 确认后填写 Release notes

先在 GitHub Actions 页面确认构建成功，再创建 `release-notes-X.Y.Z.md`，执行：

```bash
GITHUB_REPOSITORY=owner/repo GH_TOKEN=... npm run release:notes -- v0.1.0 release-notes-0.1.0.md
```

脚本会更新说明、回读核对正文并列出资产。未提供 `GH_TOKEN` 时会尝试读取 Git 凭据管理器，**不要打印或提交 token**。

## Windows 代码签名（可选）

在 GitHub 仓库配置：

- `WINDOWS_CERT_PFX`：PFX 证书的 base64 内容。
- `WINDOWS_CERT_PASSWORD`：证书密码。

配置后，CI 会通过 `scripts/sign-windows.ps1` 给主程序和安装包签名；未配置时仍可构建未签名版本。

当前主程序未接入自动更新，所以不发 `latest.json`，也不需要 updater 签名私钥。

## macOS 签名与公证

当前发布流程构建未签名、未公证的 Apple Silicon 和 Intel DMG。首次打开可能被 Gatekeeper 拦截；正式公开发布前应配置 Apple Developer ID 签名、公证和 staple，不能把本机未验收的构建当作已完成真机适配。

## 打包与运行注意事项

- `npm run tauri build` 会先执行 `scripts/prepare-bundled-plugins.mjs`，把精简后的插件源码放入 `.generated/plugins/`。
- 插件依赖、编译缓存、日志、数据库不会进入安装包。
- 工具箱优先启动插件已构建的可执行程序；找不到时回退到开发命令，此时需要 Bun／Rust 环境。
- 如果希望安装后的最终用户无需开发环境即可运行两个插件，需要进一步把插件预编译产物纳入发布流程。
- 插件的单实例、托盘和全局快捷键仍归插件自己管理，启动多个插件时注意快捷键冲突。
