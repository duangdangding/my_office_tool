# 办公工具箱

一个可扩展的 Windows／macOS 本地办公工具箱，基于 Tauri 2、TypeScript 和 Rust。插件以独立进程运行，工具箱负责导入、规则、启停和日志，不接管插件自己的窗口与数据。

## 内置插件

- **速启**：全局快捷启动面板，源码位于 `plugins/suqi/`。
- **共享剪贴板**：局域网／中继剪贴板同步，源码位于 `plugins/lscopy/`。

两个插件都保留原有构建和独立运行方式。第三方插件可通过根目录的 `office-plugin.json` 描述入口、能力声明和启动命令，再从工具箱界面导入。

> 能力规则是插件间的协作约定，需要插件主动适配，并非操作系统级沙箱。

## 插件目录、导入与更新

- **自定义插件目录**：在「设置」中选择存放位置，设置持久化保存。每次启动都会扫描该目录，新导入的插件也复制到这里。切换目录不会迁移或删除旧文件；托管中的导入插件会先停止，独立运行的插件需先退出。
- **本地项目**：选择插件目录。工具箱会复制源码和清单中已存在的 `entry.executable` 程序，但不会复制整个 `target/`、`dist/` 等编译缓存，也不会修改原项目。
- **压缩包（免 Git）**：输入 ZIP 下载地址或 GitHub 仓库地址，工具箱会直接下载、解压并导入，不需要本机安装 Git。GitHub 仓库自动使用默认分支的 `archive/HEAD.zip`；其他托管服务需提供直接的 `.zip` 链接。压缩包方式不包含 Git 子模块，依赖子模块的插件请使用 Git 导入。
- **Git 项目**：输入仓库地址，工具箱使用本机 Git 浅克隆默认分支及子模块，再按本地项目的方式导入。克隆失败会显示 Git 错误，不会保留临时副本。
- **无清单导入向导**：目录中没有 `office-plugin.json` 时，会扫描当前平台的可执行程序，并根据 `package.json`、`Cargo.toml` 或 Python 入口推荐开发命令。确认名称、id、版本和启动方式后，清单只写入工具箱中的导入副本，原始项目不变；取消向导会清理 Git／ZIP 临时文件。后续更新若来源仍没有清单，会沿用工具箱中已确认的配置。
- **兼容性检查**：导入时校验清单规范版本、插件 id、启动命令和路径。可在清单中使用 `platforms: ["windows", "macos"]` 声明支持平台；未声明时按通用插件处理。当前平台不受支持，或既没有已编译程序又没有有效开发启动目录时，会拒绝导入并说明原因。
- **多平台与多架构入口**：`entry.executable` 继续支持路径字符串；需要区分构建目标时，可改用包含 `path`、`platforms` 和 `architectures` 的对象。架构值为 `x86_64` 或 `aarch64`，省略的筛选字段表示不限。启动时只考虑匹配当前操作系统和 CPU 架构的入口，导入与产物提取仍保留所有已声明的构建产物。例如：

  ```json
  {
    "entry": {
      "mode": "executable",
      "executable": [
        { "path": "bin/tool-win-x64.exe", "platforms": ["windows"], "architectures": ["x86_64"] },
        { "path": "bin/tool-macos-arm64", "platforms": ["macos"], "architectures": ["aarch64"] },
        { "path": "bin/tool-macos-x64", "platforms": ["macos"], "architectures": ["x86_64"] }
      ]
    }
  }
  ```
- **插件更新**：本地插件从原项目重新同步，Git 插件从记录的仓库默认分支获取最新内容，压缩包插件重新下载并解压。更新会再次执行兼容性检查并要求插件 id 不变；先复制到临时目录，校验成功后替换旧副本，替换失败会恢复旧版本。内置插件随工具箱发布，不能单独更新。
- **删除插件**：默认只删除工具箱中的副本。对于本地项目导入的插件，可显式勾选同时删除原始项目；删除前会重新核对目录及插件 id，避免误删已变化的位置。早期未记录来源的导入插件不能删除原项目。
- **提取编译产物**：在插件卡片或规则窗口选择「提取产物」，再选择目标文件夹。工具箱只提取 `entry.executable` 中实际存在的文件，在 `<目标目录>/<插件id>-compiled/` 下保留相对路径，不执行编译，也不复制其他构建缓存。本地导入插件会优先提取原项目中的最新产物，找不到时再使用工具箱副本。

### 多架构示例插件

`examples/multi-arch-plugin/` 是一个无第三方依赖的 Rust 示例，声明了 Windows x64、macOS ARM64 和 macOS x64 三种入口，并输出标准就绪信号。构建当前平台的产物后，可从工具箱直接导入该目录：

```bash
# Windows x64
cargo build --release --manifest-path examples/multi-arch-plugin/Cargo.toml --target x86_64-pc-windows-msvc

# macOS Apple Silicon
cargo build --release --manifest-path examples/multi-arch-plugin/Cargo.toml --target aarch64-apple-darwin

# macOS Intel
cargo build --release --manifest-path examples/multi-arch-plugin/Cargo.toml --target x86_64-apple-darwin
```

只需构建并导入当前平台的产物；其他平台的入口可以保留在清单中，不影响当前系统启动。

## 启动状态与就绪信号

点击启动后，插件会先进入「启动中」，初始化完成后才显示「运行中」。开发启动最长等待 30 分钟，首次编译不会过早被判定为成功；启动或编译失败会显示详细错误和最近 64 KB 日志，也可以取消启动。

插件应在自身全部初始化完成后，向标准输出写入一行就绪信号（`id` 替换为清单中的插件 id）：

```text
OFFICE_TOOLBOX_READY:suqi
```

两个内置插件已适配。未提供信号的单文件可执行程序会在进程存活 2 秒后按运行中处理；开发命令必须提供信号，避免长时间编译被误判。工具箱每秒同步进程状态，插件手动退出后会更新为未运行或异常退出；退出工具箱会停止所有托管插件，独立模式插件不受影响。

## 设置与系统托盘

侧边栏「设置」提供以下选项，均持久化保存在工具箱数据目录的 `settings.json`：

- **开机启动**：登录系统后自动启动主程序。Windows 写入当前用户的 `Run` 注册表项，macOS 使用当前用户的 LaunchAgent，更改立即生效。
- **静默启动**：下次启动时不显示主窗口，只驻留系统托盘，可与开机启动配合使用。
- **插件目录**：显示并更改插件存放位置。

主程序启动后显示系统托盘图标。单击图标或选择「显示办公工具箱」可恢复主窗口，右键菜单支持显示、隐藏和退出。点击窗口关闭按钮只会隐藏到托盘；从托盘选择「退出」才会结束主程序并停止托管插件。主程序只允许运行一个实例，重复打开时会唤起并聚焦已有窗口。

## 主程序更新

在「关于与插件规范」页面顶部可查看当前版本和安装方式，并手动检查更新：

- **Windows 安装版与 macOS**：发现新版本后点击「下载并安装」。更新包来自本仓库 GitHub Releases，下载后验证 Tauri updater 签名，再安装并重启。安装期间主程序和托管插件会退出。
- **Windows 便携版**：点击「前往下载」打开最新发布页，退出主程序后用新版便携 exe 替换原文件。便携版不会调用 NSIS 安装器，避免在电脑中生成另一个安装副本。
- 下载或安装失败会保留错误信息，可重试或打开发布页面手动下载。`latest.json` 在 Windows 和两种 macOS 架构都构建完成后才发布。

旧版本没有主程序更新入口，需要先从 GitHub Releases 手动安装首个包含更新功能的版本；之后的版本才能直接从此页面升级。

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

- `office-toolbox_X.Y.Z_x64-setup.exe` 及 `.sig`：Windows NSIS 安装包与 updater 签名。
- `office-toolbox_X.Y.Z_x64_portable.exe`：Windows 便携版。
- `office-toolbox_X.Y.Z_arm64.dmg`、`office-toolbox_X.Y.Z_x86_64.dmg`：macOS Apple Silicon／Intel 安装镜像。
- `office-toolbox_X.Y.Z_arm64.app.tar.gz`、`office-toolbox_X.Y.Z_x86_64.app.tar.gz` 及各自 `.sig`：macOS 更新包与签名。
- `latest.json`：合并 Windows x64、macOS ARM64 和 macOS x64 后的应用更新清单。
- `sha256sums-windows.txt`、`sha256sums-macos-*.txt` 及对应 MD5 文件：各平台校验文件。

发布资产统一使用 ASCII 文件名，避免中文名称在上传时被移除；安装包或更新包改名时，对应 `.sig` 必须同步改名。标签、三处项目版本和锁文件版本必须一致，否则 CI 会在构建前停止。`scripts/update-manifest.mjs` 负责生成各平台片段并合并清单，同时校验平台数量、版本和最终资产 URL。

### 3. 确认后填写 Release notes

先在 GitHub Actions 页面确认构建成功，再创建 `release-notes-X.Y.Z.md`，执行：

```bash
GITHUB_REPOSITORY=owner/repo GH_TOKEN=... npm run release:notes -- v0.1.0 release-notes-0.1.0.md
```

脚本会更新说明、回读核对正文并列出资产。未提供 `GH_TOKEN` 时会尝试读取 Git 凭据管理器，**不要打印或提交 token**。

## 更新签名与 Windows 代码签名

主程序使用独立的 Tauri updater minisign 密钥，不能复用速启或共享剪贴板的密钥。首次发版前必须在 GitHub 仓库配置：

- `TAURI_SIGNING_PRIVATE_KEY`：主程序私钥文件的完整内容。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：该私钥的密码。

当前本机密钥位于被 Git 忽略的 `.office-toolbox/updater/`，其中 `.key` 是私钥、`.key.password` 是密码、`.key.pub` 是公钥。请妥善备份，配置 secrets 时不要在日志或终端中打印内容。`src-tauri/tauri.conf.json` 只保存 base64 编码后的公钥；缺少私钥 secret 时 CI 会停止发布。密钥丢失后，旧版本将无法信任使用新密钥签名的更新包。

Windows 代码签名是另一套可选机制。配置 `WINDOWS_CERT_PFX`（PFX 证书的 base64）和 `WINDOWS_CERT_PASSWORD` 后，CI 会通过 `scripts/sign-windows.ps1` 给主程序和安装包签名；未配置时仍可构建未签名版本。代码签名证书不能替代 updater 私钥。

## macOS 签名与公证

当前发布流程构建未签名、未公证的 Apple Silicon 和 Intel DMG。首次打开可能被 Gatekeeper 拦截；正式公开发布前应配置 Apple Developer ID 签名、公证和 staple，不能把本机未验收的构建当作已完成真机适配。updater 的 minisign 只验证更新包来源和完整性，不等同于 Apple 代码签名或公证。

## 打包与运行注意事项

- `npm run tauri build` 会先执行 `scripts/prepare-bundled-plugins.mjs`，把精简后的插件源码放入 `.generated/plugins/`；`.generated/plugin-binaries/` 中按相对路径暂存的预编译程序会自动合并进去。
- macOS Release 构建会先分别编译 Apple Silicon／Intel 对应的速启和共享剪贴板，再把可执行文件放入主程序资源目录，因此安装后启动内置插件不需要 Bun／Rust。CI 会检查两个二进制确实进入发布资源。
- Windows 当前仍随包插件源码；已有可执行文件时直接运行，否则回退到开发命令并需要 Bun／Rust 环境。
- 插件依赖、编译缓存、日志、数据库不会进入安装包。
- 插件的单实例、托盘和全局快捷键仍归插件自己管理，启动多个插件时注意快捷键冲突。
