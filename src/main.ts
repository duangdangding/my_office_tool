import "./styles.css";
import {
  chooseExtractDirectory,
  choosePluginDirectory,
  choosePluginStorageDirectory,
  extractPlugin,
  getPluginDirectory,
  importGitPlugin,
  importPlugin,
  isDesktop,
  launchPlugin,
  listPlugins,
  openPluginDirectory,
  readPluginLog,
  removePlugin,
  saveRules,
  setPluginDirectory,
  stopPlugin,
  updatePlugin,
} from "./api";
import type { PluginRules, PluginStatus, PluginView } from "./types";
import type { Update } from "@tauri-apps/plugin-updater";
import {
  checkAppUpdate,
  getAppVersion,
  getInstallKind,
  installAppUpdate,
  openReleasePage,
  type InstallKind,
} from "./updater";

type ViewKey = "all" | "running" | "rules" | "about";
type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "installing" | "error";

const app = document.querySelector<HTMLElement>("#app")!;
const modalRoot = document.querySelector<HTMLElement>("#modal-root")!;
let plugins: PluginView[] = [];
let currentView: ViewKey = "all";
let query = "";
let loading = true;
let busyId = "";
let appVersion = "…";
let installKind: InstallKind | null = null;
let updatePhase: UpdatePhase = "idle";
let updateMessage = "尚未检查更新";
let pendingAppUpdate: Update | null = null;
let updateDownloaded = 0;
let updateTotal = 0;

const escape = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, (char) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
}[char]!));

const icons: Record<string, string> = {
  grid: '<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>',
  activity: '<path d="M3 12h4l2.5-7 5 14 2.5-7h4"/>',
  sliders: '<path d="M4 6h16M4 12h16M4 18h16"/><circle cx="9" cy="6" r="2" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="2" fill="currentColor" stroke="none"/><circle cx="8" cy="18" r="2" fill="currentColor" stroke="none"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  rocket: '<path d="M14 5c3.5-3.5 6-2 6-2s1.5 2.5-2 6l-5 5-4-4 5-5Z"/><path d="m9 10-4 1-2 3 6 1M13 14l-1 4-3 2-1-6M15 8h.01"/><path d="M6 17c-2 0-3 1-3 4 3 0 4-1 4-3"/>',
  clipboard: '<rect x="5" y="4" width="14" height="17" rx="3"/><path d="M9 4.5V3h6v1.5M9 10h6M9 14h6M9 18h4"/>',
  puzzle: '<path d="M19 13h-2a2 2 0 1 0 0 4h2v4h-4v-2a2 2 0 1 0-4 0v2H7v-4H5a2 2 0 1 1 0-4h2V9H3V5h4V3a2 2 0 1 1 4 0v2h4v4h2a2 2 0 1 1 0 4Z"/>',
  play: '<path d="m8 5 11 7-11 7V5Z"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  external: '<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/>',
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>',
  trash: '<path d="M4 7h16M9 7V4h6v3M7 7l1 14h8l1-14M10 11v6M14 11v6"/>',
  close: '<path d="m6 6 12 12M18 6 6 18"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  shield: '<path d="M12 3 4 6v5c0 5 3.5 8.5 8 10 4.5-1.5 8-5 8-10V6l-8-3Z"/><path d="m9 12 2 2 4-4"/>',
  arrow: '<path d="M5 12h14M14 7l5 5-5 5"/>',
  box: '<path d="m12 3 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5M3 16l9 5 9-5"/>',
};

function icon(name: string, size = 20): string {
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] ?? icons.puzzle}</svg>`;
}

function toast(message: string, kind: "success" | "error" | "info" = "info"): void {
  const root = document.querySelector<HTMLElement>("#toast-root")!;
  const node = document.createElement("div");
  node.className = `toast ${kind}`;
  node.innerHTML = `<span class="toast-icon">${icon(kind === "success" ? "check" : kind === "error" ? "info" : "info", 18)}</span><span>${escape(message)}</span>`;
  root.append(node);
  window.setTimeout(() => node.classList.add("visible"), 10);
  window.setTimeout(() => {
    node.classList.remove("visible");
    window.setTimeout(() => node.remove(), 250);
  }, 3600);
}

app.innerHTML = `
  <aside class="sidebar">
    <div class="brand"><span class="brand-mark"><i></i><i></i><i></i><i></i></span><div><b>办公工具箱</b><small>OFFICE SPACE</small></div></div>
    <div class="nav-label">工作空间</div>
    <nav class="nav">
      <button data-action="nav" data-view="all" class="active">${icon("grid")}<span>全部工具</span><em id="nav-count">0</em></button>
      <button data-action="nav" data-view="running">${icon("activity")}<span>正在运行</span></button>
      <button data-action="nav" data-view="rules">${icon("sliders")}<span>规则中心</span></button>
    </nav>
    <div class="nav-label nav-label-second">支持</div>
    <nav class="nav"><button data-action="nav" data-view="about">${icon("info")}<span>关于与插件规范</span></button></nav>
    <div class="sidebar-spacer"></div>
    <div class="local-card"><span class="local-icon">${icon("shield", 19)}</span><b>本地，更安心</b><p>工具与数据留在你的电脑，按需启动每一项能力。</p><span class="local-status"><i></i>本地服务正常</span></div>
    <div class="sidebar-foot"><span class="avatar">办</span><div><b>我的办公空间</b><small>个人工作站</small></div><span>•••</span></div>
  </aside>
  <main class="main">
    <header class="topbar">
      <label class="global-search">${icon("search", 19)}<input id="global-search" placeholder="搜索工具、能力或分类…" autocomplete="off" /><kbd>Ctrl K</kbd></label>
      <div class="top-actions"><span class="desktop-mode"><i></i>${isDesktop ? "桌面模式" : "界面预览"}</span><button class="button subtle" data-action="plugin-directory">${icon("folder", 17)}插件目录</button><button class="button primary" data-action="import">${icon("plus", 18)}导入插件</button></div>
    </header>
    <div id="content" class="content"></div>
  </main>`;

function statusMeta(status: PluginStatus): { label: string; className: string } {
  if (status === "starting") return { label: "启动中", className: "starting" };
  if (status === "running") return { label: "运行中", className: "running" };
  if (status === "detached") return { label: "独立运行", className: "detached" };
  if (status === "failed") return { label: "启动失败", className: "failed" };
  return { label: "未运行", className: "stopped" };
}

function pluginCard(plugin: PluginView): string {
  const { manifest, rules, runtime } = plugin;
  const busy = busyId === manifest.id;
  const starting = runtime.status === "starting" || (busy && runtime.status === "stopped");
  const status = statusMeta(starting ? "starting" : runtime.status);
  let actionButton: string;
  if (runtime.status === "running") {
    actionButton = `<button class="button subtle" data-action="stop" data-id="${manifest.id}" ${busy ? "disabled" : ""}>${icon("stop", 15)}停止</button>`;
  } else if (starting) {
    actionButton = `<button class="button subtle" data-action="stop" data-id="${manifest.id}">${icon("stop", 15)}取消启动</button>`;
  } else if (runtime.status === "detached") {
    actionButton = `<button class="button subtle" data-action="open-dir" data-id="${manifest.id}">${icon("external", 15)}独立运行中</button>`;
  } else {
    actionButton = `<button class="button launch" data-action="launch" data-id="${manifest.id}" ${!rules.enabled || busy ? "disabled" : ""}>${icon("play", 15)}${runtime.status === "failed" ? "重试" : "启动"}</button>`;
  }
  return `<article class="plugin-card ${!rules.enabled ? "disabled" : ""}">
    <div class="card-top">
      <div class="plugin-icon ${manifest.icon}">${icon(manifest.icon, 28)}</div>
      <div class="card-badges"><span class="status-badge ${status.className}"><i></i>${status.label}</span><span class="source-badge">${plugin.source === "builtin" ? "内置" : plugin.origin?.type === "git" ? "Git 导入" : "本地导入"}</span></div>
    </div>
    <div class="plugin-title"><h3>${escape(manifest.name)}</h3><span>v${escape(manifest.version)}</span></div>
    <div class="plugin-category">${escape(manifest.category || "办公效率")}</div>
    <p class="plugin-description">${escape(manifest.description)}</p>
    ${starting ? `<div class="runtime-detail starting"><span class="spinner"></span><span>${escape(runtime.detail || "正在创建插件进程，请稍候")}</span></div>` : runtime.status === "failed" ? `<div class="runtime-detail failed"><span>${escape(runtime.detail || "插件启动失败")}</span><button data-action="log" data-id="${manifest.id}">详细日志</button></div>` : ""}
    <div class="capability-row"><span>${icon("box", 14)}独立进程</span>${manifest.permissions.slice(0, 2).map((item) => `<span>${escape(permissionLabel(item))}</span>`).join("")}</div>
    <div class="card-footer">
      <label class="mini-switch" title="${rules.enabled ? "停用插件" : "启用插件"}"><input type="checkbox" data-action="toggle" data-id="${manifest.id}" ${rules.enabled ? "checked" : ""} ${busy || starting ? "disabled" : ""}/><span></span><em>${rules.enabled ? "已启用" : "已停用"}</em></label>
      <div class="card-actions">
        ${plugin.source === "imported" ? `<button class="icon-button" data-action="update" data-id="${manifest.id}" title="从来源更新插件" ${busy || runtime.status === "detached" ? "disabled" : ""}>${icon("activity", 17)}</button>` : ""}
        <button class="icon-button" data-action="extract" data-id="${manifest.id}" title="提取编译产物">${icon("box", 17)}</button>
        <button class="icon-button" data-action="rules" data-id="${manifest.id}" title="插件规则">${icon("sliders", 18)}</button>
        <button class="icon-button" data-action="standalone" data-id="${manifest.id}" title="独立启动（不托管）" ${!rules.enabled || busy || runtime.status !== "stopped" ? "disabled" : ""}>${icon("external", 17)}</button>
        ${actionButton}
      </div>
    </div>
  </article>`;
}

function dashboardContent(): string {
  const running = plugins.filter((plugin) => ["starting", "running", "detached"].includes(plugin.runtime.status)).length;
  const autoStart = plugins.filter((plugin) => plugin.rules.autoStart && plugin.rules.enabled).length;
  const needle = query.trim().toLowerCase();
  let shown = plugins.filter((plugin) => [plugin.manifest.name, plugin.manifest.description, plugin.manifest.category].join(" ").toLowerCase().includes(needle));
  if (currentView === "running") shown = shown.filter((plugin) => ["starting", "running", "detached"].includes(plugin.runtime.status));
  return `<section class="page-heading"><div><p class="eyebrow">YOUR WORKSPACE</p><h1>${currentView === "running" ? "正在运行" : "让办公，更得心应手"}</h1><p>${currentView === "running" ? "查看和管理已启动的独立工具。" : "把常用工具放在一个空间，各自独立，又彼此有序。"}</p></div><div class="heading-art"><span></span><span></span>${icon("box", 28)}</div></section>
    <section class="stats-row">
      <div class="stat"><span class="stat-icon green">${icon("puzzle", 21)}</span><div><b>${plugins.length}</b><small>已安装工具</small></div><span class="stat-note">个</span></div>
      <div class="stat"><span class="stat-icon blue">${icon("activity", 21)}</span><div><b>${running}</b><small>正在运行</small></div><span class="stat-note"><i class="live-dot"></i>实时</span></div>
      <div class="stat"><span class="stat-icon amber">${icon("rocket", 21)}</span><div><b>${autoStart}</b><small>随工具箱启动</small></div><span class="stat-note">项</span></div>
    </section>
    <div class="section-heading"><div><h2>${currentView === "running" ? "运行中的工具" : "我的工具"}</h2><span>${shown.length} 个结果</span></div><button class="text-button" data-action="refresh">刷新状态 ${icon("activity", 15)}</button></div>
    ${loading ? `<div class="loading-state"><span class="spinner dark"></span><p>正在读取本地插件…</p></div>` : shown.length ? `<section class="plugin-grid">${shown.map(pluginCard).join("")}<button class="add-plugin-card" data-action="import"><span>${icon("plus", 25)}</span><b>添加新工具</b><p>导入本地目录或 Git 仓库中的插件</p></button></section>` : `<section class="empty-state"><span>${icon("search", 32)}</span><h3>${needle ? "没有找到匹配的工具" : currentView === "running" ? "暂时没有运行中的工具" : "还没有插件"}</h3><p>${needle ? "试试其他名称、分类或能力关键词。" : "从全部工具中启动一个插件，它会显示在这里。"}</p>${needle ? `<button class="button subtle" data-action="clear-search">清空搜索</button>` : `<button class="button primary" data-action="nav" data-view="all">浏览全部工具</button>`}</section>`}`;
}

function rulesContent(): string {
  return `<section class="page-heading compact"><div><p class="eyebrow">PLUGIN GOVERNANCE</p><h1>规则中心</h1><p>统一控制启停、启动方式与插件协作能力。</p></div></section>
    <div class="rule-notice">${icon("shield", 22)}<div><b>规则清晰，边界也清晰</b><p>启停与随工具箱启动由本体直接管理；网络、剪贴板、文件等能力规则通过环境变量下发，需插件按规范适配，并非操作系统级沙箱。</p></div></div>
    <section class="rules-list">${plugins.map((plugin) => `<article class="rule-row"><div class="plugin-icon small ${plugin.manifest.icon}">${icon(plugin.manifest.icon, 22)}</div><div class="rule-plugin"><b>${escape(plugin.manifest.name)}</b><span>${escape(plugin.manifest.id)} · ${plugin.rules.detach ? "独立启动" : "工具箱托管"}</span></div><div class="rule-tags"><span class="${plugin.rules.enabled ? "on" : ""}">${plugin.rules.enabled ? "已启用" : "已停用"}</span><span class="${plugin.rules.autoStart ? "on" : ""}">${plugin.rules.autoStart ? "自动启动" : "手动启动"}</span></div><button class="button subtle" data-action="rules" data-id="${plugin.manifest.id}">配置规则${icon("arrow", 15)}</button></article>`).join("")}</section>`;
}

function installKindLabel(): string {
  if (installKind === "installed") return "Windows 安装版";
  if (installKind === "portable") return "Windows 便携版";
  if (installKind === "macos") return "macOS 应用";
  return "正在识别安装方式";
}

function appUpdateCard(): string {
  const busy = ["checking", "downloading", "installing"].includes(updatePhase);
  const showProgress = updatePhase === "downloading" || updatePhase === "installing";
  const percent = updateTotal > 0 ? Math.min(100, Math.round(updateDownloaded / updateTotal * 100)) : 0;
  const progressText = updatePhase === "installing" ? "更新包下载完成，正在安装" : updateTotal > 0 ? `${percent}% · ${(updateDownloaded / 1024 / 1024).toFixed(1)} / ${(updateTotal / 1024 / 1024).toFixed(1)} MB` : `已下载 ${(updateDownloaded / 1024 / 1024).toFixed(1)} MB`;
  const canInstall = pendingAppUpdate && ["available", "error"].includes(updatePhase);
  return `<section class="spec-card app-update-card ${updatePhase === "error" ? "has-error" : ""}" id="app-update-card">
    <div class="app-update-heading"><div><span class="section-kicker">APPLICATION UPDATE</span><h2>办公工具箱 <span>v${escape(appVersion)}</span></h2><p>${escape(installKindLabel())} · 更新来自 GitHub Releases</p></div><span class="app-update-icon">${icon("activity", 24)}</span></div>
    <div class="app-update-status" role="status"><span class="update-state-dot ${updatePhase}"></span><div><b>${escape(updateMessage)}</b><p>${pendingAppUpdate ? `发现新版本 v${escape(pendingAppUpdate.version)}` : installKind === "portable" ? "可手动检查新版本，再从发布页下载便携版。" : "可手动检查新版本；安装前会验证更新包签名。"}</p></div></div>
    ${showProgress ? `<div class="app-update-progress"><progress max="100" value="${percent}" aria-label="更新下载进度"></progress><span>${escape(progressText)}</span></div>` : ""}
    <div class="app-update-actions"><button class="button primary" data-action="app-check-update" ${busy ? "disabled" : ""}>${updatePhase === "checking" ? '<span class="spinner"></span>' : icon("activity", 16)}${updatePhase === "checking" ? "正在检查" : "检查更新"}</button>${canInstall ? `<button class="button primary" data-action="app-install-update" ${installKind ? "" : "disabled"}>${icon(installKind === "portable" ? "external" : "arrow", 16)}${installKind === "portable" ? "前往下载" : "下载并安装"}</button>` : ""}<button class="button subtle" data-action="app-open-releases" ${busy ? "disabled" : ""}>${icon("external", 15)}发布页面</button></div>
    ${installKind === "portable" ? '<p class="app-update-note">便携版请下载新版便携 exe 并替换原文件；不要运行安装包，以免生成另一个安装副本。</p>' : '<p class="app-update-note">安装过程中主程序会退出并重启，托管插件也会随之停止。</p>'}
  </section>`;
}

function syncAppUpdateCard(): void {
  const card = document.querySelector<HTMLElement>("#app-update-card");
  if (card) card.outerHTML = appUpdateCard();
}

async function runAppUpdateCheck(): Promise<void> {
  if (["checking", "downloading", "installing"].includes(updatePhase)) return;
  updatePhase = "checking";
  updateMessage = "正在检查主程序更新…";
  updateDownloaded = 0;
  updateTotal = 0;
  syncAppUpdateCard();
  if (pendingAppUpdate) {
    const previous = pendingAppUpdate;
    pendingAppUpdate = null;
    await previous.close().catch(() => undefined);
  }
  try {
    const update = await checkAppUpdate();
    pendingAppUpdate = update;
    updatePhase = update ? "available" : "idle";
    updateMessage = update ? `新版本 v${update.version} 可用` : "当前已是最新版本";
  } catch (error) {
    updatePhase = "error";
    updateMessage = `检查更新失败：${String(error)}`;
  }
  syncAppUpdateCard();
}

async function runAppUpdateInstall(): Promise<void> {
  if (!pendingAppUpdate || ["checking", "downloading", "installing"].includes(updatePhase)) return;
  if (!installKind) {
    try { installKind = await getInstallKind(); }
    catch (error) { updatePhase = "error"; updateMessage = `无法识别安装方式：${String(error)}`; syncAppUpdateCard(); return; }
  }
  if (installKind === "portable") {
    await openReleasePage().catch((error) => toast(String(error), "error"));
    return;
  }
  updatePhase = "downloading";
  updateMessage = `正在下载 v${pendingAppUpdate.version}…`;
  updateDownloaded = 0;
  updateTotal = 0;
  syncAppUpdateCard();
  try {
    await installAppUpdate(pendingAppUpdate, ({ downloaded, total, finished }) => {
      updateDownloaded = downloaded;
      updateTotal = total;
      updatePhase = finished ? "installing" : "downloading";
      updateMessage = finished ? "下载完成，正在校验并安装…" : `正在下载 v${pendingAppUpdate?.version ?? ""}…`;
      syncAppUpdateCard();
    });
    updatePhase = "installing";
    updateMessage = "更新已安装，正在重启…";
    syncAppUpdateCard();
  } catch (error) {
    updatePhase = "error";
    updateMessage = `更新失败：${String(error)}`;
    syncAppUpdateCard();
  }
}

function aboutContent(): string {
  return `<section class="page-heading compact"><div><p class="eyebrow">OPEN & INDEPENDENT</p><h1>一个开放的小型办公生态</h1><p>工具箱不接管插件的窗口与数据，只提供统一的入口和运行规则。</p></div></section>
    ${appUpdateCard()}
    <section class="architecture-card"><div class="architecture-copy"><span class="section-kicker">运行架构</span><h2>相互独立，才能稳定协作</h2><p>每个工具都是独立进程。一个插件崩溃不会拖垮工具箱；插件也可以脱离本体，使用原来的方式开发、构建和运行。</p><div class="architecture-points"><span>${icon("check", 16)}标准 JSON 清单</span><span>${icon("check", 16)}独立数据目录</span><span>${icon("check", 16)}托管 / 独立双模式</span></div></div><div class="architecture-diagram"><div class="host-node">${icon("grid", 23)}<b>办公工具箱</b><small>规则 · 启停 · 日志</small></div><div class="diagram-lines"><i></i><i></i></div><div class="child-nodes"><span>${icon("rocket", 20)}速启</span><span>${icon("clipboard", 20)}共享剪贴板</span><span>${icon("puzzle", 20)}你的插件</span></div></div></section>
    <section class="spec-card management-card"><div><span class="section-kicker">PLUGIN MANAGEMENT</span><h2>从导入到更新，统一管理</h2></div><div class="management-grid"><div>${icon("folder", 19)}<b>自定义插件目录</b><p>点击顶部「插件目录」选择存放位置。每次启动自动扫描，后续导入也写入此目录；切换时不迁移旧文件。</p></div><div>${icon("shield", 19)}<b>导入兼容性检查</b><p>支持本地目录和 Git 仓库。导入前检查清单规范、当前平台及启动方式，不兼容时说明原因并停止导入。</p></div><div>${icon("activity", 19)}<b>来源更新</b><p>本地插件从原项目重新同步，Git 插件克隆默认分支的最新内容。校验通过后替换，失败时保留旧版本。</p></div><div>${icon("box", 19)}<b>删除与产物提取</b><p>删除时可选是否同时删除原始本地项目；编译产物可提取到指定目录，并保留相对路径。</p></div></div></section>
    <section class="spec-card"><div><span class="section-kicker">PLUGIN MANIFEST V1</span><h2>三步接入你的插件</h2></div><ol class="spec-steps"><li><span>01</span><div><b>添加插件清单</b><p>在插件根目录创建 <code>office-plugin.json</code>，声明 id、名称、版本和启动命令；可选 <code>platforms</code> 声明 windows／macos。</p></div></li><li><span>02</span><div><b>声明启动方式</b><p><code>executable</code> 指向已构建程序；<code>dev</code> 声明开发命令；<code>auto</code> 优先程序、回退开发命令。</p></div></li><li><span>03</span><div><b>读取规则并报告就绪</b><p>读取 <code>OFFICE_PLUGIN_RULES_FILE</code> 和 <code>OFFICE_PLUGIN_ALLOW_*</code>；初始化完成后输出 <code>OFFICE_TOOLBOX_READY:插件id</code>。</p></div></li></ol><div class="env-example"><code>OFFICE_PLUGIN_ID=suqi</code><code>OFFICE_PLUGIN_ALLOW_NETWORK=true</code><code>OFFICE_PLUGIN_ALLOW_CLIPBOARD=true</code><code>OFFICE_PLUGIN_DATA_DIR=…</code></div></section>`;
}

const viewTitles: Record<ViewKey, [string, string]> = {
  all: ["全部工具", "本地插件工作台"],
  running: ["正在运行", "独立进程状态"],
  rules: ["规则中心", "插件运行与能力规则"],
  about: ["关于", "插件接入规范"],
};

function render(): void {
  document.querySelectorAll<HTMLElement>(".nav [data-view]").forEach((button) => button.classList.toggle("active", button.dataset.view === currentView));
  const count = document.querySelector<HTMLElement>("#nav-count");
  if (count) count.textContent = String(plugins.length);
  const content = document.querySelector<HTMLElement>("#content")!;
  content.innerHTML = currentView === "rules" ? rulesContent() : currentView === "about" ? aboutContent() : dashboardContent();
  document.title = `${viewTitles[currentView][0]} · 办公工具箱`;
}

let refreshing = false;
async function refresh(): Promise<void> {
  if (refreshing) return;
  refreshing = true;
  try {
    plugins = await listPlugins();
  } catch (error) {
    toast(String(error), "error");
  } finally {
    refreshing = false;
    loading = false;
    render();
  }
}

function closeModal(): void {
  modalRoot.innerHTML = "";
}

function showModal(content: string, className = ""): void {
  modalRoot.innerHTML = `<div class="modal-backdrop ${className}"><section class="modal" role="dialog" aria-modal="true">${content}</section></div>`;
}

function showImport(mode: "local" | "git" = "local"): void {
  const local = mode === "local";
  showModal(`<button class="modal-close" data-action="close-modal" aria-label="关闭">${icon("close")}</button><span class="modal-symbol">${icon("plus", 26)}</span><h2>导入插件</h2><div class="import-source-tabs"><button class="${local ? "active" : ""}" data-action="import-mode" data-mode="local">${icon("folder", 16)}本地项目</button><button class="${local ? "" : "active"}" data-action="import-mode" data-mode="git">${icon("box", 16)}Git 项目</button></div>
    ${local ? `<p class="modal-intro">选择包含 <code>office-plugin.json</code> 的文件夹。插件会复制到工具箱，原目录保持不变；清单中已有的编译程序也会一并复制。</p><div class="modal-actions"><button class="button subtle" data-action="close-modal">取消</button><button class="button primary" data-action="confirm-import">选择插件文件夹${icon("arrow", 16)}</button></div>` : `<form id="git-import-form"><p class="modal-intro">输入 Git 仓库地址，工具箱会克隆默认分支及其子模块。仓库根目录必须包含 <code>office-plugin.json</code>，本机需已安装 Git。</p><label class="field-label" for="git-repository">仓库地址</label><input class="text-input" id="git-repository" name="repository" type="text" placeholder="https://github.com/owner/plugin.git" autocomplete="off" required autofocus/><div class="modal-actions"><button type="button" class="button subtle" data-action="close-modal">取消</button><button class="button primary" type="submit">克隆并导入${icon("arrow", 16)}</button></div></form>`}
    <div class="warning-box import-warning">${icon("shield", 20)}<p>导入前会检查清单规范、当前平台和启动方式，不兼容的插件不会写入插件目录。插件是可执行代码，请只导入信任的来源。</p></div>`, "import-modal");
}

async function performImport(importAction: () => Promise<PluginView>): Promise<void> {
  closeModal();
  loading = true;
  render();
  try {
    const plugin = await importAction();
    await refresh();
    toast(`已导入「${plugin.manifest.name}」。`, "success");
  } catch (error) {
    loading = false;
    render();
    toast(String(error), "error");
  }
}

function showExtract(id: string): void {
  const plugin = plugins.find((item) => item.manifest.id === id);
  if (!plugin) return;
  showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><span class="modal-symbol">${icon("box", 25)}</span><h2>提取「${escape(plugin.manifest.name)}」编译产物</h2><p class="modal-intro">将清单 <code>entry.executable</code> 中已存在的程序复制到所选目录，并保留相对路径。此操作只提取，不会执行编译。</p><div class="artifact-list">${plugin.manifest.entry.executable.map((file) => `<code>${escape(file)}</code>`).join("")}</div><div class="modal-actions"><button class="button subtle" data-action="close-modal">取消</button><button class="button primary" data-action="confirm-extract" data-id="${id}">选择目录并提取${icon("arrow", 16)}</button></div>`);
}

async function showPluginDirectory(): Promise<void> {
  try {
    const directory = await getPluginDirectory();
    showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><span class="modal-symbol">${icon("folder", 25)}</span><h2>自定义插件目录</h2><p class="modal-intro">每次启动都会扫描此目录，新导入的插件也复制到这里。切换目录不会迁移或删除旧目录中的文件。</p><div class="standalone-info"><b>${directory.isDefault ? "当前使用默认目录" : "当前使用自定义目录"}</b><code>${escape(directory.path)}</code></div><div class="warning-box import-warning">${icon("info", 20)}<p>切换前会停止正在托管运行的导入插件；独立运行的插件需要先手动退出。规则配置会保留。</p></div><div class="modal-actions"><button class="button subtle" data-action="close-modal">取消</button><button class="button primary" data-action="confirm-plugin-directory">选择新目录${icon("arrow", 16)}</button></div>`);
  } catch (error) { toast(String(error), "error"); }
}

function showUpdate(id: string): void {
  const plugin = plugins.find((item) => item.manifest.id === id);
  if (!plugin?.origin) {
    toast("此插件没有可用的更新来源。", "error");
    return;
  }
  const isGit = plugin.origin.type === "git";
  const source = isGit ? plugin.origin.repository : plugin.origin.localPath;
  showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><span class="modal-symbol">${icon("activity", 25)}</span><h2>更新「${escape(plugin.manifest.name)}」</h2><p class="modal-intro">${isGit ? "从 Git 默认分支获取最新内容" : "从原始本地项目重新同步"}，并检查清单、平台兼容性、启动方式及插件 id。更新会停止托管进程，失败时保留旧版本。</p>${source ? `<div class="standalone-info"><b>更新来源</b><code>${escape(source)}</code></div>` : `<div class="error-box"><b>更新来源不完整</b><p>请移除后重新导入此插件。</p></div>`}<div class="modal-actions"><button class="button subtle" data-action="close-modal">取消</button><button class="button primary" data-action="confirm-update" data-id="${id}" ${source ? "" : "disabled"}>确认更新${icon("arrow", 16)}</button></div>`);
}

function ruleToggle(name: keyof PluginRules, title: string, description: string, checked: boolean): string {
  return `<label class="form-toggle"><div><b>${title}</b><p>${description}</p></div><input type="checkbox" name="${name}" ${checked ? "checked" : ""}/><span class="toggle-track"></span></label>`;
}

function showRules(id: string): void {
  const plugin = plugins.find((item) => item.manifest.id === id);
  if (!plugin) return;
  const { rules, manifest } = plugin;
  showModal(`<button class="modal-close" data-action="close-modal" aria-label="关闭">${icon("close")}</button><div class="rules-modal-head"><div class="plugin-icon ${manifest.icon}">${icon(manifest.icon, 26)}</div><div><h2>${escape(manifest.name)}规则</h2><p>${escape(manifest.id)} · v${escape(manifest.version)}</p></div></div><form id="rules-form" data-id="${manifest.id}">
    <div class="form-section"><h3>运行方式</h3>${ruleToggle("enabled", "启用插件", "停用后不能从工具箱启动，托管进程会停止。", rules.enabled)}${ruleToggle("autoStart", "随工具箱启动", "办公工具箱启动后自动拉起此插件。", rules.autoStart)}${ruleToggle("detach", "始终以独立模式启动", "启动后不由工具箱托管；插件退出本体后仍可继续运行。", rules.detach)}</div>
    <div class="form-section"><h3>插件能力 <span>需插件适配</span></h3>${ruleToggle("allowNetwork", "允许网络能力", "向插件声明允许使用网络与设备同步。", rules.allowNetwork)}${ruleToggle("allowClipboard", "允许剪贴板能力", "向插件声明允许读取或写入系统剪贴板。", rules.allowClipboard)}${ruleToggle("allowFileSystem", "允许文件系统能力", "向插件声明允许访问其功能所需文件。", rules.allowFileSystem)}${ruleToggle("allowGlobalShortcut", "允许全局快捷键", "向插件声明允许注册系统级快捷键。", rules.allowGlobalShortcut)}${ruleToggle("allowPluginAutostart", "允许插件设置开机启动", "默认禁止插件绕过工具箱自行设置系统自启。", rules.allowPluginAutostart)}</div>
    <div class="standalone-info"><b>${icon("external", 16)}独立运行</b><p>${escape(manifest.standalone.instructions)}</p><code>${escape(plugin.root)}</code></div>
    <div class="modal-actions split"><div>${plugin.source === "imported" ? `<button type="button" class="button text-button" data-action="update" data-id="${manifest.id}">${icon("activity", 16)}更新插件</button><button type="button" class="button danger-text" data-action="remove" data-id="${manifest.id}">${icon("trash", 16)}移除插件</button>` : ""}<button type="button" class="button text-button" data-action="extract" data-id="${manifest.id}">${icon("box", 16)}提取产物</button><button type="button" class="button text-button" data-action="log" data-id="${manifest.id}">${icon("terminal", 16)}运行日志</button></div><div><button type="button" class="button subtle" data-action="close-modal">取消</button><button class="button primary" type="submit">保存规则</button></div></div>
  </form>`, "rules-modal");
}

async function showLog(id: string, errorMessage?: string): Promise<void> {
  const plugin = plugins.find((item) => item.manifest.id === id);
  showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><h2>${escape(plugin?.manifest.name ?? id)}${errorMessage ? "启动失败" : "运行日志"}</h2><p class="modal-intro">仅保留最近 64 KB 输出，便于排查编译与启动问题。</p>${errorMessage ? `<div class="error-box"><b>详细错误</b><p>${escape(errorMessage)}</p></div>` : ""}<pre class="log-view">正在读取日志…</pre><div class="modal-actions"><button class="button subtle" data-action="rules" data-id="${id}">返回规则</button><button class="button primary" data-action="close-modal">完成</button></div>`, "rules-modal");
  const view = modalRoot.querySelector<HTMLElement>(".log-view");
  try {
    const log = await readPluginLog(id);
    if (view) view.textContent = log;
  } catch (error) {
    if (view) view.textContent = `读取日志失败：${String(error)}`;
  }
}

async function runPlugin(id: string, detached?: boolean): Promise<void> {
  busyId = id;
  const plugin = plugins.find((item) => item.manifest.id === id);
  if (plugin && !detached) {
    plugin.runtime = { status: "starting", pid: null, detail: "正在创建插件进程，首次开发启动可能需要编译" };
  }
  render();
  try {
    await launchPlugin(id, detached);
    await refresh();
    toast(detached ? "插件已独立启动，可脱离工具箱运行。" : "插件已就绪。", "success");
  } catch (error) {
    const message = String(error);
    await refresh();
    if (message.includes("启动已取消")) {
      toast("已取消插件启动。", "info");
    } else {
      toast("插件启动失败，请查看详细错误与日志。", "error");
      await showLog(id, message);
    }
  } finally {
    busyId = "";
    render();
  }
}

function permissionLabel(permission: string): string {
  return ({ clipboard: "剪贴板", network: "网络", globalShortcut: "快捷键", fileSystem: "文件", autostart: "自启动", process: "进程" } as Record<string, string>)[permission] ?? permission;
}

document.addEventListener("click", (event) => {
  const target = (event.target as HTMLElement).closest<HTMLElement>("[data-action]");
  if (!target) return;
  const action = target.dataset.action!;
  const id = target.dataset.id ?? "";
  if (action === "nav") {
    currentView = (target.dataset.view ?? "all") as ViewKey;
    render();
  } else if (action === "plugin-directory") void showPluginDirectory();
  else if (action === "app-check-update") void runAppUpdateCheck();
  else if (action === "app-install-update") void runAppUpdateInstall();
  else if (action === "app-open-releases") void openReleasePage().catch((error) => toast(String(error), "error"));
  else if (action === "confirm-plugin-directory") {
    void (async () => {
      const path = await choosePluginStorageDirectory();
      if (!path) return;
      try {
        const directory = await setPluginDirectory(path);
        closeModal();
        await refresh();
        toast(`插件目录已切换为：${directory.path}`, "success");
      } catch (error) { toast(String(error), "error"); }
    })();
  }
  else if (action === "import") showImport();
  else if (action === "import-mode") showImport(target.dataset.mode === "git" ? "git" : "local");
  else if (action === "confirm-import") {
    void (async () => {
      const path = await choosePluginDirectory();
      if (path) await performImport(() => importPlugin(path));
    })();
  } else if (action === "close-modal") closeModal();
  else if (action === "launch") void runPlugin(id);
  else if (action === "standalone") void runPlugin(id, true);
  else if (action === "stop") {
    void (async () => {
      busyId = id;
      render();
      try {
        await stopPlugin(id);
        await refresh();
        toast("插件已停止。", "success");
      } catch (error) { toast(String(error), "error"); }
      finally { busyId = ""; render(); }
    })();
  }
  else if (action === "rules") showRules(id);
  else if (action === "update") showUpdate(id);
  else if (action === "extract") showExtract(id);
  else if (action === "log") void showLog(id);
  else if (action === "open-dir") {
    void openPluginDirectory(id).then(() => { if (!isDesktop) toast("预览模式：插件目录已复制到剪贴板。", "success"); }).catch((error) => toast(String(error), "error"));
  }
  else if (action === "refresh") { loading = true; render(); void refresh(); }
  else if (action === "clear-search") {
    query = "";
    const input = document.querySelector<HTMLInputElement>("#global-search")!;
    input.value = "";
    render();
    input.focus();
  }
  else if (action === "confirm-update") {
    closeModal();
    busyId = id;
    render();
    void (async () => {
      try {
        const plugin = await updatePlugin(id);
        await refresh();
        toast(`「${plugin.manifest.name}」已更新到 v${plugin.manifest.version}。`, "success");
      } catch (error) {
        await refresh();
        toast(String(error), "error");
      } finally {
        busyId = "";
        render();
      }
    })();
  }
  else if (action === "confirm-extract") {
    void (async () => {
      const destination = await chooseExtractDirectory();
      if (!destination) return;
      try {
        const result = await extractPlugin(id, destination);
        showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><span class="modal-symbol">${icon("check", 25)}</span><h2>编译产物已提取</h2><p class="modal-intro">共提取 ${result.files.length} 个文件到：</p><div class="standalone-info"><code>${escape(result.outputDir)}</code></div><div class="artifact-list">${result.files.map((file) => `<code>${escape(file)}</code>`).join("")}</div><div class="modal-actions"><button class="button primary" data-action="close-modal">完成</button></div>`);
      } catch (error) { toast(String(error), "error"); }
    })();
  }
  else if (action === "remove") {
    const plugin = plugins.find((item) => item.manifest.id === id);
    const originalPath = plugin?.origin?.localPath;
    showModal(`<button class="modal-close" data-action="close-modal">${icon("close")}</button><span class="modal-symbol danger">${icon("trash", 25)}</span><h2>移除「${escape(plugin?.manifest.name ?? id)}」？</h2><p class="modal-intro">工具箱中的插件副本会被删除。此操作无法撤销。</p>${originalPath ? `<label class="delete-source-option"><input id="delete-source" type="checkbox"/><span><b>同时删除原始本地项目</b><small>原目录中的所有文件都会被永久删除，请先确认不再需要。</small><code>${escape(originalPath)}</code></span></label>` : ""}<div class="modal-actions"><button class="button subtle" data-action="close-modal">取消</button><button class="button danger" data-action="confirm-remove" data-id="${id}">确认移除</button></div>`);
  }
  else if (action === "confirm-remove") {
    const deleteSource = Boolean(modalRoot.querySelector<HTMLInputElement>("#delete-source")?.checked);
    void (async () => {
      try {
        await removePlugin(id, deleteSource);
        closeModal();
        await refresh();
        toast(deleteSource ? "插件及原始本地项目已删除。" : "插件已移除，原始本地项目保持不变。", "success");
      } catch (error) {
        await refresh();
        toast(String(error), "error");
      }
    })();
  }
});

document.addEventListener("change", (event) => {
  const input = (event.target as HTMLElement).closest<HTMLInputElement>('input[data-action="toggle"]');
  if (!input) return;
  const plugin = plugins.find((item) => item.manifest.id === input.dataset.id);
  if (!plugin) return;
  const rules = { ...plugin.rules, enabled: input.checked };
  void saveRules(plugin.manifest.id, rules).then(refresh).then(() => toast(input.checked ? "插件已启用。" : "插件已停用。", "success")).catch((error) => { input.checked = !input.checked; toast(String(error), "error"); });
});

document.addEventListener("submit", (event) => {
  const form = event.target as HTMLFormElement;
  if (form.id === "git-import-form") {
    event.preventDefault();
    const url = form.querySelector<HTMLInputElement>('[name="repository"]')?.value.trim() ?? "";
    if (!url) {
      toast("请输入 Git 仓库地址。", "error");
      return;
    }
    void performImport(() => importGitPlugin(url));
    return;
  }
  if (form.id !== "rules-form") return;
  event.preventDefault();
  const plugin = plugins.find((item) => item.manifest.id === form.dataset.id);
  if (!plugin) return;
  const value = (name: keyof PluginRules) => Boolean(form.querySelector<HTMLInputElement>(`[name="${name}"]`)?.checked);
  const rules: PluginRules = {
    enabled: value("enabled"),
    autoStart: value("autoStart"),
    detach: value("detach"),
    allowNetwork: value("allowNetwork"),
    allowClipboard: value("allowClipboard"),
    allowFileSystem: value("allowFileSystem"),
    allowGlobalShortcut: value("allowGlobalShortcut"),
    allowPluginAutostart: value("allowPluginAutostart"),
  };
  void saveRules(plugin.manifest.id, rules).then(async () => { closeModal(); await refresh(); toast("插件规则已保存。", "success"); }).catch((error) => toast(String(error), "error"));
});

const searchInput = document.querySelector<HTMLInputElement>("#global-search")!;
searchInput.addEventListener("input", () => {
  query = searchInput.value;
  if (currentView === "rules" || currentView === "about") currentView = "all";
  render();
});

document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
  }
  if (event.key === "Escape" && modalRoot.innerHTML) closeModal();
});

void refresh();
void Promise.all([getAppVersion(), getInstallKind()]).then(([version, kind]) => {
  appVersion = version;
  installKind = kind;
  syncAppUpdateCard();
}).catch((error) => {
  updatePhase = "error";
  updateMessage = `读取主程序信息失败：${String(error)}`;
  syncAppUpdateCard();
});
if (isDesktop) {
  window.setInterval(() => {
    if (!document.hidden) void refresh();
  }, 1000);
}
