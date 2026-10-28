# AGENTS.md

本文件面向在本仓库中工作的 AI 编程助手，说明项目结构、常用命令与约定。

## 项目简介

**速启（suqi-launcher）**——一个 Windows 桌面快捷启动器，基于 **Tauri 2 + Vanilla TypeScript + Rust**。

- 全局热键（默认 `Win + \``）呼出/隐藏快捷启动面板
- 分组管理：固定「全部」（首位）、自定义分组、加密分组（末位密码保护）、未分组
- 搜索支持拼音首拼 / 全拼 / 模糊匹配
- 拖拽排序（分组排序）
- 桌面扫描批量导入（.lnk / .exe / 文件夹 / 文件），默认选中快捷方式，按真实目标地址去重
- 图标缓存（提取 exe 图标到本地缓存目录）
- 右键添加 / 编辑 / 删除，按真实目标地址判重
- 📌 钉住面板（alwaysOnTop）
- 系统托盘图标（显示/隐藏、设置、钉住、退出）
- 设置页面：全局快捷键、启动（自启/静默启动）、面板（记住大小/置顶）、外观（主题/字号/高亮色/窗口效果）、数据库（导入/导出/信息）、关于
- 开机自启、静默启动、记住窗口大小

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | Vite 6 + TypeScript 5.6（无框架，原生 DOM） |
| 后端 | Rust（Tauri 2），edition 2021 |
| 包管理 | **bun**（锁定文件为 `bun.lock`，不要用 npm/yarn/pnpm） |
| 数据 | JSON 文件（`%APPDATA%/suqi/`） |

## 目录结构

```
index.html          主窗口页面（快捷启动面板）
settings.html       设置窗口页面
src/
  main.ts           主窗口前端逻辑（列表渲染、分组标签页、搜索、拖拽排序）
  settings.ts       设置页逻辑（热键录制、自启、外观、数据库导入导出等）
  config.ts         前端共享配置（AppConfig 接口、加载/保存、外观应用）
  styles.css        全局样式
src-tauri/
  src/lib.rs        后端主体：数据存取、托盘菜单、全局热键、窗口控制、设置命令
  src/main.rs       入口（仅调用 lib）
  capabilities/     Tauri 权限声明（windows 列表需包含新增窗口 label）
  tauri.conf.json   窗口/打包配置（identifier: com.suqi.launcher）
vite.config.ts      多页面构建配置（index + settings）
.github/workflows/  CI / 发布流程
```

## 常用命令

```bash
bun install            # 安装依赖
bun run dev            # 仅起 Vite 前端（http://localhost:1420）
bun run build          # tsc 类型检查 + Vite 构建（提交前必跑）
bun run tauri dev      # 启动完整桌面应用（前端 + Rust 后端）
bun run tauri build    # 打包发布产物
```

Rust 侧（在 `src-tauri/` 下）：

```bash
cargo check            # 快速类型检查
cargo clippy           # lint
```

## 代码约定

- **前端**：原生 TypeScript，无框架；DOM 操作为主，保持与现有 `main.ts` / `settings.ts` 风格一致；不引入 UI 框架或新依赖，除非确有必要。
- **后端**：功能集中在 `src-tauri/src/lib.rs`，按 `// ---------- xxx ----------` 注释分区；新增 Tauri command 需同步在 `capabilities/default.json` 中放行（如适用）。
- **注释与 UI 文案**：中文。
- **配置持久化**：`AppConfig`（serde）+ JSON 文件，新增配置字段注意 `#[serde(default)]` 向后兼容。
- **README 同步**：每次新增功能都要同步重写 `README.md`（更新功能特性等对应小节），写入文件后与功能代码一并提交。

## 验证方式

- 前端改动：`bun run build` 通过 tsc 检查。
- 后端改动：`cargo check` / `cargo clippy` 通过后，`bun run tauri dev` 真机验证热键唤起、托盘菜单、设置页。
- 本项目无自动化测试，以手动验证为主。

## 注意事项

- **构建必须走 Tauri CLI**（`bun run tauri build` / `tauri dev`），不要裸 `cargo build --release`：CLI 会开启 `custom-protocol` 特性并正确处理前端资源协议，裸 cargo 构建的 exe 会显示"无法访问页面"。
- Windows 为主要目标平台。
- 数据文件存储在 `%APPDATA%/suqi/` 下：`launcher.json`（条目/分组/密码哈希）、`config.json`（应用配置）、`icons/`（图标缓存）。
- 全局热键通过 `tauri-plugin-global-shortcut` 注册，`parse_hotkey` / `code_from_name` 负责字符串到 `Shortcut` 的转换；修改默认热键时注意 `Code` 枚举使用具名变体（`KeyA`、`Digit0`、`F1` 等）。
- 设置窗口通过 `WebviewWindowBuilder` 动态创建（label: "settings"），`vite.config.ts` 需配置多页入口使 `settings.html` 进入 `dist/`。
- 托盘菜单在 `setup` 闭包中构建，包含显示/隐藏、设置、钉住（CheckMenuItem）、退出；左键单击托盘图标切换面板显隐。
- 主窗口关闭按钮默认隐藏（`CloseRequested` → `prevent_close` + `hide`），通过托盘菜单或热键呼出。
- **静默启动**：`silent_start` 为 true 时，setup 中主窗口创建后不自动 show。
- **钉住面板**：`always_on_top` 与 `pinned` 状态联动，主窗口 header 有 📌 按钮切换。
- 版本号需同步修改 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 三处（`Cargo.lock` 随构建自动更新）。
- `dist/`、`target/`、`node_modules/` 为构建产物，不要提交或编辑。

## 环境排障备忘（2026-10-14 记录）

- **bun 必须解析到 `C:\Users\Administrator\.bun\bin\bun.exe`**。用户 PATH 中 `...\daimon-share\daimon\npm-global` 目录下有一组 npm 安装的 bun shim（`bun` / `bun.cmd` / `bun.ps1`），其指向的 bun.exe 不在 PATH 内，`bun run` 会报 `bun is not installed in %PATH%`（exit 255），与项目代码无关。
- 已将用户 PATH 调整为 `C:\Users\Administrator\.bun\bin` 在最前（真实 bun 1.4.2）。**新开的终端生效；已打开的终端 / 已启动的进程需重启**才能拿到新 PATH。
- 若所在 shell 仍是旧 PATH（如 Kimi 会话），临时绕过：`export PATH="/c/Users/Administrator/.bun/bin:$PATH"` 再执行 `bun run build`。
- 验证 PATH 修复是否生效：新开 cmd 执行 `where bun`，第一条应为 `.bun\bin\bun.exe`。
- 参考 `F:\workspace\my\vscode\copy-pc\tauri-dev.log`：其中 `script "dev" exited with code 1` / `beforeDevCommand terminated` 也是前端进程问题导致 dev 中断，排查时先看 beforeDevCommand（vite）是否存活，别急着怀疑 Rust 代码。
- 已知无害警告：`src/settings.ts` 对 `@tauri-apps/plugin-fs` 同时静态 + 动态 import，vite 提示不拆 chunk，可忽略或后续统一为静态 import。
- **GitHub 推送默认直连**（2026-10-21 起）：全局 `.gitconfig` 中 `http.https://github.com.proxy` 已移除。直连失败（如报 `Failed to connect to github.com` 超时）时才临时走代理：`git -c http.https://github.com.proxy=http://127.0.0.1:7897 push ...`（前提是代理软件已开启）；不要把代理写回配置文件。历史上曾配置过该代理，代理软件未运行时推送会报 `Failed to connect to 127.0.0.1 port 7897`。
- **updater 签名密钥**：密钥对位于 `C:\Users\Administrator\.tauri\suqi.key` / `suqi.key.pub`，密码保存在同目录 `suqi.key.password`。CI secrets 中的 `TAURI_SIGNING_PRIVATE_KEY` 必须等于私钥文件的完整内容（单行 base64，可整文件复制粘贴），`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 等于密码文件内容；粘贴时不能带换行或多余空白，否则报 `failed to decode base64 key: Invalid padding`。tauri.conf.json 的 pubkey 必须等于 `base64(suqi.key.pub 文件内容)`。修改任何一处后，用 `bun run tauri signer sign <文件>` 本地验证通过再发版。

## 发布流程

1. 验证：`bun run build` + `cargo check` / `cargo clippy` 全绿；版本号三处已同步。
2. 提交并推送：`git add -A && git commit` → `git push origin main` → `git tag vX.Y.Z && git push origin vX.Y.Z`。
   推 `v*` tag 触发 `.github/workflows/release.yml`，Windows 构建并上传 Release 产物（约 5–10 分钟）。
   - 产物：NSIS 安装包（suqi_X.Y.Z_x64-setup.exe）+ 对应 .sig + 便携版 exe + latest.json + SHA-256 校验文件。
3. **构建结果由用户自行查看确认**（GitHub Actions 页面）。助手**不要用任何方式查询构建状态**——包括 GitHub API、gh CLI、轮询、下载日志等；推完 tag 即停下，等用户主动告知结果后再继续下一步。（用户明确要求，2026-10-14）
4. 写 release notes（用户确认构建成功后执行）：先在工作区写 `release-notes-X.Y.Z.md`（新功能 / 升级提醒 / 其他），`GET /releases/tags/vX.Y.Z` 拿 release id → `PATCH /releases/{id}` 写入 `body`。
   核对：重新 GET，确认 body 首尾完整、assets 数量正确（通常 5 个：setup + .sig + portable exe + latest.json + sha256sums）。

### GitHub API 凭据

- 本机无 `gh` CLI、无系统 `python`/`uv`。token 在 Windows 凭据管理器（git credential manager）里，PowerShell 获取方式：

```powershell
$env:GH_TOKEN = ("protocol=https`nhost=github.com`n`n" | git credential fill 2>$null | Where-Object { $_ -like "password=*" }) -replace "^password=", ""
```

- **任何时候不得明文打印 token**。
- 写 release notes 用 Node 18 原生 fetch（`https.get` 也行，不用装依赖），参考实现：

```js
const [tag, notesPath] = process.argv.slice(2);
const repo = "duangdangding/ls_quick_start";
const headers = { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: "application/vnd.github+json", "Content-Type": "application/json" };
const rel = await (await fetch(`https://api.github.com/repos/${repo}/releases/tags/${tag}`, { headers })).json();
const body = require("fs").readFileSync(notesPath, "utf-8");
await fetch(`https://api.github.com/repos/${repo}/releases/${rel.id}`, { method: "PATCH", headers, body: JSON.stringify({ body }) });
```
