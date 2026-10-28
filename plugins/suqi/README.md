# 速启 (Suqi Launcher)

## 办公工具箱插件

本目录同时是一个独立的办公工具箱插件，根目录的 `office-plugin.json` 描述启动方式与能力声明。通过工具箱启动时会收到 `OFFICE_PLUGIN_RULES_FILE`、`OFFICE_PLUGIN_DATA_DIR` 和 `OFFICE_PLUGIN_ALLOW_*` 环境变量；这些变量是协作规则，不是操作系统沙箱。

脱离工具箱运行的方式不变：在本目录执行 `bun install`，然后执行 `bun run tauri dev`；需要发布时执行 `bun run tauri build`。已有数据、窗口和全局快捷键仍由速启自己管理。

跨平台桌面快捷启动器（Windows / macOS），基于 Tauri 2 + Vite + TypeScript + Rust。

## ⭐ 免费开源声明

**速启是完全免费的个人开源软件，永久免费、无任何付费功能。**

- 🚫 如果你是花钱买到本软件的（任何付费渠道均非官方行为），说明你被骗了——请**直接申请退款**，然后到 [GitHub Releases](https://github.com/duangdangding/ls_quick_start/releases) 免费下载正版。
- ⭐ 如果这个小工具对你有帮助，欢迎到 [GitHub 仓库](https://github.com/duangdangding/ls_quick_start) 点一下 **Star**，这是对我持续更新最大的鼓励！


## 功能

- 分组管理（全部 / 自定义分组 / 加密 / 未分组）；「全部」视图下条目卡片显示所属分组标签
- 加密分组密码保护（未解锁时其中的启动项既不显示也不参与搜索；解锁后可在「全部」中搜索并显示；解锁策略可选：每次打开面板都需输入密码 / 仅重启应用才需输入密码；加密分组内可点「加密」按钮一键上锁；忘记密码可清空加密分组并重新设置）
- 启动项关键字：添加 / 编辑时可填写搜索关键字（留空则使用名称），卡片上会展示关键字标签
- 启动项说明：添加 / 编辑时可填写备注（不参与搜索），鼠标悬停条目时在气泡中显示完整名称、说明与实际路径
- 搜索（优先按关键字，同时匹配名称 / 拼音首拼 / 全拼 / 模糊匹配，命中字符高亮，支持一键清空）
- 直接运行（仿 `Win + R`）：搜索框输入 `\\IP` 网络路径、盘符路径（支持 `%appdata%` 等环境变量与不带引号的含空格路径）、`shell:` 目标、网址或命令名（可带参数，如 `cmd`、`calc`、`services.msc`、`notepad xxx.txt`）时，结果列表末尾出现「直接运行」条目（虚线卡片），Enter / 双击即执行并自动隐藏面板；命令名按 Win+R 规则解析（App Paths 注册表 → PATH + PATHEXT），找不到时给出明确提示；可在设置中关闭（默认开启），关闭后搜索框仅匹配启动项

- 排序（匹配度 / 名称 / 添加时间 / 类型，默认匹配度：搜索时命中越靠前 / 越连续排得越前，无搜索词时等同于按名称排序）；分组标签支持拖拽排序
- 扫描导入：默认扫描桌面（Windows 会一并列出当前显示在桌面的系统图标——此电脑 / 回收站 / 控制面板 / 网络 / 用户的文件，按系统「桌面图标设置」的显示状态读取，名称与图标取系统本地化资源，默认勾选；macOS 默认扫描「应用程序」目录 `/Applications` 与系统自带应用目录 `/System/Applications`，App Store 等系统应用也能扫到），也可选择任意文件夹或系统开始菜单（.lnk / .exe / 文件夹 / 文件）；可选择是否扫描子文件夹（默认不扫描，开始菜单 / 应用程序目录也只扫当前层，需要时手动勾选）与是否按所在文件夹名称自动分组（默认不勾选，勾选后分组不存在则新建）；macOS 应用按系统语言显示本地化名称（如「微信」而非「WeChat」）；结果中快捷方式默认排前，每条显示所在文件夹名称；结果列表支持全选 / 取消全选 / 按类型快选，支持树状 / 卡片 / 列表三种布局（默认树状，目录节点默认展开、可折叠，同级的文件夹与文件按列对齐显示，父子勾选互不关联：勾选目录只代表导入该目录本身，勾选目录下的快捷方式不会连带选中父目录），弹窗可最大化铺满全屏、可拖拽调整大小，结果列表高度随弹窗自适应占满剩余空间；扫描与图标提取在后台线程执行，不阻塞界面；导入完成后弹出结果明细——重复项逐条列出已存在于哪个分组（含加密分组 / 未分组），失败项逐条列出失败原因；若成功导入的条目包含 .lnk 快捷方式，可选择一键删除原位置的快捷方式文件（只删 .lnk 本身，不影响其指向的程序 / 文件，条目启动自动回退为直接打开真实目标）
- 还原到文件夹：把启动项批量还原到指定文件夹，分组与条目均可勾选（默认全选），可选是否按分组创建子文件夹（「未分组」直接放在根目录，加密分组需先解锁才能导出）；程序 / 快捷方式还原为 .lnk（原始 .lnk 还在时直接复制，保留参数与工作目录），CMD 命令还原为 .bat，网址还原为 .url（macOS 为 .webloc / 符号链接 / .command）；文件和文件夹不导出，结果页逐条列出未导出与失败明细，可一键打开目标文件夹查看
- 图标缓存（条目图标 + 扫描结果图标：Windows 取系统图标，macOS 用 sips 把应用包内 icns / 图片文件转成缩略图，其余用系统通用图标兜底，全程不用 qlmanage，避免其崩溃导致系统反复弹「"qlmanage"意外退出」）
- 右键添加 / 编辑 / 删除；点击已选中条目的名称即可就地修改（失焦 / Enter 保存，Esc 取消）
- 条目「⋯」菜单支持编辑 / 打开 / 移动到分组 / 删除
- 多选模式：工具栏「多选」进入后，点击卡片勾选，支持全选 / 反选 / 清空，可批量删除或批量移动到其他分组（未分组 / 自定义分组 / 加密）
- 删除分组时可选择处理方式：仅删除分组（组内项目转为未分组）或同时删除组内全部项目
- 全局快捷键呼出 / 隐藏（Windows 默认 `Win + \``，macOS 默认 `Alt + Space`）；面板已显示但被其他窗口盖住时，可在设置中选择按快捷键「显示到最前面」还是「隐藏面板」
- Esc 逐层关闭：主面板中依次关闭最上层弹窗 → 「⋯」菜单 → 隐藏面板；设置页中依次关闭弹窗 → 关闭设置窗口（录制快捷键时 Esc 仅取消录制，内联改名时 Esc 仅取消改名）
- 启动条目（双击 / Enter /「⋯」菜单打开）成功后面板自动隐藏，回到之前的窗口
- 快捷方式来源启动：扫描导入或手动添加 .lnk 时记录其原始路径，启动时经快捷方式打开，由系统 Shell 还原 目标+参数+工作目录，避免参数或工作目录丢失导致启动失败；重新扫描导入可自动为历史条目补齐来源
- 每次呼出面板自动清空上次的搜索词（可在设置「搜索」中关闭，关闭后重新打开面板保留上次搜索词）；切换到其他窗口再切回时，光标自动回到搜索框
- 单实例运行（重复启动时提示"速启已在运行"，不再开启第二个进程）
- 系统托盘图标
- 钉住面板（置顶，按钮与托盘菜单实时同步）
- 开机自启 / 静默启动（静默启动时主窗口从启动到就绪完全不出现，无闪窗）
- 记住窗口大小
- 全局显示方式切换（卡片 / 列表，默认卡片；卡片为紧凑布局；列表视图下超长名称与路径自动省略号截断，不产生横向滚动条，「⋯」按钮位于行首（图标之前）始终可见）
- 外观设置（主题 / 字号 / 高亮色（可跟随主题）/ 窗口效果 / 面板背景图）
  - 面板背景支持拉伸 / 平铺、宽高百分比、旋转 0/90/180/270°、框选区域裁剪、透明度，并可应用到设置窗口
- 数据存储于 SQLite，旧版 JSON 数据自动迁移
- 自定义配置文件目录（默认软件同目录，目标目录已有配置文件时直接使用）
- 自定义数据库文件目录（默认软件同目录）
- 配置文件 / 数据库目录设置跨重启与版本更新保留（记录在数据目录的指针文件中，更新后无需重新配置）
- 自动更新（启动自动检查 + 面板横幅提示；Windows 便携版就地替换重启 / 可下载到其他目录，安装版下载 NSIS 安装包；macOS 走官方 updater 通道下载安装并重启）；插件查不到更新时比对 GitHub 已发布的 Release 兜底，避免「最新 Release 未及时切换」时误判为已是最新，且只提示真正可下载的版本
- 更新下载支持系统代理：优先走 `HTTPS_PROXY` / Windows 系统代理，代理不可用时自动回退直连
- 更新文件 SHA-256 校验（Release 的 `sha256sums.txt`）；Release 同时提供 MD5 校验文件（`md5sums.txt` / `md5sums-macos-*.txt`）
- 便携版更新辅助进程：退出后替换 exe 并自动重启（%TEMP% 副本执行）
- 代码签名（Windows / macOS 均由 GitHub Actions 注入）
- 数据库信息查看

## 技术栈

- **前端**: Vite + TypeScript
- **后端**: Rust (Tauri 2)
- **插件**: global-shortcut, autostart, process, fs, dialog, opener, updater, single-instance
- **存储**: SQLite（`suqi.db`，Windows 默认在软件同目录，macOS 默认在 `~/Library/Application Support/com.suqi.launcher`）

## 开发

```bash
bun install
bun run tauri dev
```

## 构建

```bash
bun run build        # tsc + vite
cargo check          # Rust 检查（src-tauri/ 下）
bun run tauri build  # 打包
```

macOS 打包（Apple Silicon 原生 / Intel 交叉编译，Intel 需先 `rustup target add x86_64-apple-darwin`）：

```bash
bun run tauri build                              # Apple Silicon（arm64）
bun run tauri build --target x86_64-apple-darwin # Intel（x86_64）
```

## 发布

打 `v*` 标签推送即可触发 GitHub Actions 自动构建 Release：

```bash
git tag v1.0.0
git push origin v1.0.0
```

产物（Windows + macOS 双平台，同一 Release 内发布）：
- Windows：NSIS 安装包 + 便携版 exe + `sha256sums.txt` / `md5sums.txt`
- macOS：arm64（Apple Silicon）与 x86_64（Intel）各自的 dmg + app.tar.gz + `sha256sums-macos-*.txt` / `md5sums-macos-*.txt`
- `latest.json`（updater）：同时包含 `windows-x86_64` / `darwin-aarch64` / `darwin-x86_64`

### 代码签名

GitHub 仓库需配置以下 secrets：
- `WINDOWS_CERT_PFX`：base64 编码的代码签名证书（.pfx）
- `WINDOWS_CERT_PASSWORD`：证书密码
- `TAURI_SIGNING_PRIVATE_KEY`：Tauri updater 签名私钥
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥密码

macOS 如需公证（notarization），需额外配置 `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` / `APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID`；未配置时产出的 dmg 未签名，首次打开需右键「打开」或在「系统设置 → 隐私与安全性」中放行。

生成 updater 密钥对：`npx @tauri-apps/cli signer generate -w ~/.tauri/suqi.key`。

### 图标

`node design/make-icon.mjs`（或 `bun design/make-icon.mjs`）由 `design/icon.svg` 生成 Windows 的 `icon.ico` 与 macOS 的 `icon.icns`。

## macOS 安装与常见问题

### 首次打开提示「"速启"已损坏，无法打开」

这是**未签名 / 未公证**应用被 Gatekeeper 的隔离属性（quarantine）拦截导致的正常现象，并非文件真的损坏。任选一种方式解决：

**方式一：右键打开（最简单）**

在「访达 → 应用程序」中右键「速启」→ 选择「打开」→ 在弹窗中再次点击「打开」。

**方式二：移除隔离属性（终端）**

先把「速启」拖入「应用程序」，再执行：

```bash
xattr -cr /Applications/速启.app
```

若提示权限不足，前面加 `sudo`：

```bash
sudo xattr -cr /Applications/速启.app
```

**方式三：系统设置放行**

「系统设置 → 隐私与安全性」→ 在「安全性」区域点击「仍要打开」。

> 若下载的是 dmg：先双击挂载，把「速启」拖到「应用程序」后再执行上述操作。
