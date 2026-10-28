import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as chooseFile } from "@tauri-apps/plugin-dialog";
import { emit, listen } from "@tauri-apps/api/event";
import { AppConfig, applyAppearance, applyWindowBackground, applyWindowEffect, isMac, loadConfig, saveConfig } from "./config";
import { autoCheckEnabled, checkUpdate } from "./updater";
import "./styles.css";

type SearchCache = { kw: string; kwPy: string; kwIn: string; nm: string; nmPy: string; nmIn: string };
type Item = { id: string; name: string; keyword: string; description: string; target: string; lnkPath: string | null; groupId: string | null; isEncrypted: boolean; isCmd: boolean; iconPath: string | null; sortOrder: number; createdAt: number; search?: SearchCache };
type Group = { id: string; name: string; sortOrder: number };
type DesktopEntry = { name: string; sourcePath: string; target: string; isShortcut: boolean; entryType: "shortcut" | "file" | "folder" | "system"; scanIconPath: string | null; folderName: string | null; relPath: string[] };
type SortBy = "relevance" | "name" | "created" | "type";
type ScanSource = { kind: "desktop" } | { kind: "start-menu" } | { kind: "folder"; dir: string };
type ViewMode = "card" | "list";
type ImportSkip = { name: string; target: string; groupName: string | null; reason: string | null };
type ImportResult = { added: number; skippedDuplicates: number; skippedInvalid: number; duplicates: ImportSkip[]; invalids: ImportSkip[]; importedShortcuts: string[] };
type DeleteShortcutsResult = { deleted: number; failures: ImportSkip[] };
type ExportResult = { exported: number; skipped: ImportSkip[]; failures: ImportSkip[]; dir: string };
type ItemIcon = { id: string; iconPath: string | null };

const encryptedId = "__encrypted__";
let items: Item[] = [];
let groups: Group[] = [];
let passwordSet = false;
let unlocked = false;
let activeGroup = "all";
let sortBy: SortBy = "relevance";
let viewMode: ViewMode = "card";
let appConfig: AppConfig | null = null;
let query = "";
let selected = 0;
let composingSearch = false;
let scanning: DesktopEntry[] = [];
let opening = false;
let searchSelection: [number, number] | null = null;
let pinned = false;
let selectMode = false;
const selectedIds = new Set<string>();
// 平台相关文案：macOS 无 CMD、「开始菜单」对应「应用程序」，快捷方式即 .app 应用包
const cmdLabel = isMac ? "命令" : "CMD 命令";
const shortcutLabel = isMac ? "应用程序" : "快捷方式";
const startMenuLabel = isMac ? "应用程序" : "系统开始菜单";

const app = document.querySelector<HTMLElement>("#app")!;
const escape = (text: string) => text.replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!);
const message = (value: unknown) => alert(typeof value === "string" ? value : "操作失败，请稍后重试");
const fixedGroups = () => [{ id: "all", name: "全部" }, { id: "none", name: "未分组" }, ...groups.sort((a, b) => a.sortOrder - b.sortOrder), { id: encryptedId, name: "加密" }];
/** 条目所属分组名称：自定义分组取组名，固定标签取标签名，无归属显示「未分组」（卡片分组标签用） */
const groupLabel = (id: string | null) => id === encryptedId ? "加密" : groups.find((group) => group.id === id)?.name ?? "未分组";
const displayItems = () => {
  const needle = query.trim().toLowerCase();
  const groupItems = items.filter((item) => {
    // 加密分组内容仅在解锁后参与显示与搜索：未解锁时既不显示，也不参与「全部」搜索
    if (item.isEncrypted) return unlocked && (activeGroup === encryptedId || activeGroup === "all");
    if (activeGroup === "none") return !item.groupId;
    return activeGroup === "all" || item.groupId === activeGroup;
  });
  // 匹配度：无搜索词时统一记 0 分，退化为按名称排序
  const scored = groupItems.map((item) => ({ item, score: needle ? fuzzyScore(item, needle) : 0 })).filter((entry) => !needle || entry.score > -1);
  const byName = (a: { item: Item }, b: { item: Item }) => a.item.name.localeCompare(b.item.name, "zh-CN");
  return scored.sort((a, b) => sortBy === "created" ? b.item.createdAt - a.item.createdAt
    : sortBy === "type" ? typeRank(a.item) - typeRank(b.item) || byName(a, b)
    : sortBy === "relevance" ? b.score - a.score || byName(a, b)
    : byName(a, b)).map((entry) => entry.item);
};
/** 类型排序用分组：0 命令、1 程序/快捷方式、2 文件、3 文件夹 */
function typeRank(item: Item) {
  if (item.isCmd) return 0;
  const target = item.target.toLowerCase();
  if (/\.(exe|com|bat|cmd|ps1|msi|lnk|url|app)$/.test(target)) return 1;
  return (target.split(/[\\/]/).pop() ?? "").includes(".") ? 2 : 3;
}
/**
 * 搜索匹配打分（越大越靠前）：关键字（含拼音）优先于名称（含拼音）。
 * 关键字命中统一加上大偏移，保证任何关键字命中都排在名称命中之前；同组内按匹配质量分档：
 * 3 以关键字开头 > 2 拼音以关键字开头 > 1 中间包含 > 0 子序列命中；未命中返回 -1（调用方据此过滤）。
 */
const KEYWORD_BONUS = 10000;
/** 预计算小写搜索字段：加载时算一次，避免每次按键对全部条目重复 toLowerCase */
const buildSearchCache = (item: Item): SearchCache => {
  const extra = item as Item & { fullPinyin?: string; initials?: string; keywordFullPinyin?: string; keywordInitials?: string };
  return {
    kw: item.keyword.toLowerCase(),
    kwPy: extra.keywordFullPinyin?.toLowerCase() ?? "",
    kwIn: extra.keywordInitials?.toLowerCase() ?? "",
    nm: item.name.toLowerCase(),
    nmPy: extra.fullPinyin?.toLowerCase() ?? "",
    nmIn: extra.initials?.toLowerCase() ?? "",
  };
};
function fuzzyScore(item: Item, needle: string) {
  const cache = item.search;
  const keywordScore = bestFieldScore([
    { value: cache?.kw ?? item.keyword.toLowerCase(), tier: 3 },
    { value: cache?.kwPy ?? "", tier: 2 },
    { value: cache?.kwIn ?? "", tier: 2 },
  ], needle);
  if (keywordScore > -1) return KEYWORD_BONUS + keywordScore;
  return bestFieldScore([
    { value: cache?.nm ?? item.name.toLowerCase(), tier: 3 },
    { value: cache?.nmPy ?? "", tier: 2 },
    { value: cache?.nmIn ?? "", tier: 2 },
  ], needle);
}
/** 取多个字段中的最高匹配分：完全包含时按「是否开头 + 分档」计分，否则退回子序列匹配 */
function bestFieldScore(fields: { value: string; tier: number }[], needle: string) {
  let best = -1;
  for (const { value, tier } of fields) {
    if (!value) continue;
    const at = value.indexOf(needle);
    if (at >= 0) best = Math.max(best, (at === 0 ? tier : 1) * 1000 + (200 - at));
    else { const sub = subsequenceScore(value, needle); if (sub > -1) best = Math.max(best, sub); }
  }
  return best;
}
function subsequenceScore(value: string, needle: string) {
  const exact = value.indexOf(needle); if (exact >= 0) return 200 - exact;
  let last = -1; let score = 0;
  for (const char of needle) { const next = value.indexOf(char, last + 1); if (next < 0) return -1; score += next === last + 1 ? 5 : 1; last = next; }
  return score;
}
/** 名称中的命中区间：优先连续子串，其次逐字符子序列（连续命中合并为一段） */
function nameMatchRanges(name: string, needle: string): [number, number][] | null {
  if (!needle) return null;
  const lower = name.toLowerCase();
  const exact = lower.indexOf(needle);
  if (exact >= 0) return [[exact, exact + needle.length]];
  const ranges: [number, number][] = [];
  let cursor = 0;
  for (const char of needle) {
    const next = lower.indexOf(char, cursor);
    if (next < 0) return null;
    const last = ranges[ranges.length - 1];
    if (last && last[1] === next) last[1] = next + 1;
    else ranges.push([next, next + 1]);
    cursor = next + 1;
  }
  return ranges;
}
/** 渲染名称：把命中片段包进 <mark> 高亮；未命中（如拼音匹配）返回原文 */
function renderName(name: string, needle: string): string {
  const ranges = nameMatchRanges(name, needle);
  if (!ranges) return escape(name);
  let html = "";
  let cursor = 0;
  for (const [start, end] of ranges) {
    if (start > cursor) html += escape(name.slice(cursor, start));
    html += `<mark>${escape(name.slice(start, end))}</mark>`;
    cursor = Math.max(cursor, end);
  }
  if (cursor < name.length) html += escape(name.slice(cursor));
  return html;
}
async function load() {
  // 解锁状态以后端会话为准：面板重开/页面重载后仍保持已解锁
  // 三个读取相互独立，并行发起，缩短面板加载时间
  const [data, index, icons] = await Promise.all([
    invoke<{ items: Item[]; groups: Group[]; passwordSet: boolean; unlocked: boolean }>("list_data", { includeEncrypted: unlocked }),
    invoke<{ id: string; fullPinyin: string; initials: string }[]>("searchable_items", { includeEncrypted: unlocked }),
    invoke<ItemIcon[]>("item_icons"),
  ]);
  unlocked = data.unlocked;
  const words = new Map(index.map((entry) => [entry.id, entry]));
  items = data.items.map((item) => Object.assign(item, words.get(item.id)));
  const iconMap = new Map(icons.map((icon) => [icon.id, icon.iconPath]));
  items.forEach((item) => { if (!item.iconPath) item.iconPath = iconMap.get(item.id) ?? null; });
  items.forEach((item) => { item.search = buildSearchCache(item); });
  groups = data.groups; passwordSet = data.passwordSet;
  if (!fixedGroups().some((group) => group.id === activeGroup)) activeGroup = "all";
  render(true);
}
function itemIcon(item: Item) {
  if (item.iconPath) return `<img src="${escape(convertFileSrc(item.iconPath))}" alt="" />`;
  if (item.isCmd) return "⌘";
  if (/^https?:\/\//.test(item.target)) return "◎";
  if (/\.(exe|bat|cmd|app)$/i.test(item.target)) return "▣";
  if (/\\[^\\]+$/.test(item.target)) return "▰";
  return "▤";
}
/** 搜索词可当作「运行」输入时返回原文（仿 Win+R：UNC 路径 / URL / 盘符路径 / shell: / 命令名，可带参数），否则返回 null */
function runCandidate(): string | null {
  const q = query.trim();
  if (!q || selectMode) return null;
  // 设置里关掉了「直接运行」：搜索框仅匹配启动项（appConfig 未加载时按默认开启处理）
  if (appConfig && !appConfig.runTargetEnabled) return null;
  // 加密分组未解锁时不提供运行入口，避免锁定界面残留可操作项
  if (activeGroup === encryptedId && !unlocked) return null;
  if (isMac) return /^([/~]|\w+:\/\/)/.test(q) || /^[a-zA-Z0-9_.+-]+(\s|$)/.test(q) ? q : null;
  return /^(\\\\|[a-zA-Z]:[\\/]|%[^%]+%|\w+:\/\/|shell:)/i.test(q) || /^[a-zA-Z0-9_.+-]+(\s|$)/.test(q) ? q : null;
}
/** 「直接运行」卡片：固定列在搜索结果最后，Enter / 双击执行搜索框内容 */
function runCardHtml(target: string | null, isSelected: boolean) {
  if (!target) return "";
  const label = /^\\\\/.test(target) ? "打开网络位置" : /^\w+:\/\//.test(target) ? "打开链接" : /^(shell:|[a-zA-Z]:[\\/]|%[^%]+%)/i.test(target) || /^[~/]/.test(target) ? "打开路径" : "运行命令";
  return `<article class="card run ${isSelected ? "selected" : ""}" data-run="1"><div class="icon">⏎</div><div class="card-text"><b>${label}</b><span>${escape(target)}</span></div></article>`;
}
/** 方向键上下移动的“列数”：列表视图单列，卡片视图按容器宽度估算 */
function arrowColumns() {
  if (viewMode === "list") return 1;
  return Math.max(1, Math.floor((document.querySelector("#grid")?.clientWidth ?? 250) / 170));
}
/** 切换全局显示方式（卡片/列表），并持久化到配置 */
function setViewMode(mode: ViewMode) {
  if (mode !== "card" && mode !== "list") return;
  if (mode === viewMode) return;
  viewMode = mode;
  if (appConfig) {
    appConfig.viewMode = mode;
    void saveConfig(appConfig);
  }
  render();
}
/** 重绘面板；keepScroll 为 true 时保留列表滚动位置（点击 / 方向键切换选中项、多选勾选等不改变内容的重绘不跳回顶部） */
function render(keepScroll = false) {
  hideTip();
  const scrollTop = keepScroll ? document.querySelector<HTMLElement>("#grid")?.scrollTop ?? 0 : 0;
  const shown = displayItems(); const run = runCandidate(); selected = Math.max(0, Math.min(selected, Math.max(0, shown.length + (run ? 1 : 0) - 1)));
  app.innerHTML = `<section class="shell">
    <header><div class="brand">速启 <small>Quick Launch</small></div><label class="search"><span>⌕</span><input id="search" autofocus placeholder="搜索关键字、名称或拼音" value="${escape(query)}" />${query ? `<button type="button" class="search-clear" data-action="clear-search" title="清空搜索" tabindex="-1">✕</button>` : ""}</label><button data-action="add" class="primary">＋ 添加</button><button data-action="scan">扫描导入</button><button data-action="export" title="把启动项还原为文件夹中的快捷方式 / 批处理 / 网址文件">还原</button><button data-action="groups">管理</button><button data-action="settings" title="设置">⚙</button><button data-action="pin" title="钉住面板">${pinned ? "📌" : "📍"}</button></header>
    <nav id="tabs">${fixedGroups().map((group) => `<button class="tab ${group.id === activeGroup ? "active" : ""} ${group.id === encryptedId ? "lock" : ""}" data-group="${group.id}" draggable="${groups.some((normal) => normal.id === group.id)}">${group.id === encryptedId ? (unlocked ? "🔓" : "🔒") : ""}${escape(group.name)}</button>`).join("")}<button class="add-group" data-action="new-group" title="新建分组">＋</button></nav>
    <section class="toolbar"><span>${opening ? `<i class="loading-dot"></i>正在打开…` : activeGroup === encryptedId && !unlocked ? "加密分组已锁定" : `共 ${shown.length} 项`}</span><label>排序 <select id="sort"><option value="relevance">匹配度</option><option value="name">名称</option><option value="created">添加时间</option><option value="type">类型</option></select></label><div class="view-toggle"><button data-view="card" class="${viewMode === "card" ? "active" : ""}" title="卡片视图">▦</button><button data-view="list" class="${viewMode === "list" ? "active" : ""}" title="列表视图">☰</button></div><button data-action="select-mode" class="${selectMode ? "active" : ""}" title="多选条目">${selectMode ? "退出多选" : "多选"}</button>${activeGroup === encryptedId ? `${unlocked ? `<button data-action="lock" title="立即锁定加密分组，再次查看需输入密码">加密</button>` : ""}<button data-action="password">${unlocked ? "重置密码" : passwordSet ? "解锁" : "设置密码"}</button>` : ""}</section>
    ${selectMode ? `<section class="bulk-bar"><span>已选 ${selectedIds.size} 项</span><div class="bulk-actions"><button data-action="select-all">全选</button><button data-action="select-invert">反选</button><button data-action="select-none">清空</button><button data-action="move-items" ${selectedIds.size ? "" : "disabled"}>移动到分组</button><button class="danger" data-action="delete-items" ${selectedIds.size ? "" : "disabled"}>删除</button></div></section>` : ""}
    <section id="grid" class="grid ${viewMode === "list" ? "list" : ""}">${activeGroup === encryptedId && !unlocked ? `<div class="locked"><b>🔒 加密分组</b><p>解锁后才会显示其中的启动项，并参与「全部」搜索。</p><button class="primary" data-action="password">${passwordSet ? "输入密码解锁" : "设置访问密码"}</button></div>` : shown.length || run ? shown.map((item, index) => { const needle = query.trim().toLowerCase(); const picked = selectedIds.has(item.id); return `<article class="card ${!selectMode && index === selected ? "selected" : ""} ${picked ? "picked" : ""}" data-id="${item.id}"><div class="icon${selectMode ? " pick" : ""}">${itemIcon(item)}${selectMode && picked ? `<i class="pick-mark">✓</i>` : ""}</div><div class="card-text"><b>${renderName(item.name, needle)}</b>${item.keyword ? `<em class="keyword" title="关键字">${renderName(item.keyword, needle)}</em>` : ""}${activeGroup === "all" ? `<i class="group-tag" title="所属分组">${escape(groupLabel(item.groupId))}</i>` : ""}<span>${escape(item.isCmd ? cmdLabel : item.target)}</span></div>${selectMode ? "" : `<button class="more" data-menu="${item.id}" title="更多">⋯</button>`}</article>`; }).join("") + runCardHtml(run, !selectMode && selected === shown.length) : `<div class="empty"><i class="empty-icon">🗂️</i><b>这里还没有启动项</b><p>点击“添加”，或使用扫描导入批量添加。</p></div>`}</section>
    <footer>${run ? `Enter 运行「${escape(run)}」 · 方向键切换` : "右键空白处即可添加 · Enter 打开 · 方向键切换"}</footer>
  </section>`;
  const input = document.querySelector<HTMLInputElement>("#search")!;
  input.addEventListener("compositionstart", () => { composingSearch = true; });
  input.addEventListener("compositionend", () => { composingSearch = false; query = input.value; selected = 0; searchSelection = [input.selectionStart ?? query.length, input.selectionEnd ?? query.length]; render(); });
  input.addEventListener("input", () => { if (!composingSearch) { query = input.value; selected = 0; searchSelection = [input.selectionStart ?? query.length, input.selectionEnd ?? query.length]; render(); } });
  input.addEventListener("keydown", (event) => {
    if (composingSearch) return;
    const shownItems = displayItems();
    // 多选模式下 Enter 不启动条目，避免与勾选操作冲突
    if (!selectMode && event.key === "Enter") {
      const target = shownItems[selected];
      if (target) { event.preventDefault(); void openItem(target.id); }
      else if (selected === shownItems.length && runCandidate()) { event.preventDefault(); void runCommand(); }
    }
    const total = shownItems.length + (runCandidate() ? 1 : 0);
    if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key) && total) {
      event.preventDefault();
      const width = arrowColumns();
      const delta = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" ? -width : width;
      selected = (selected + delta + total) % total;
      render(true);
    }
  });
  input.focus();
  if (searchSelection) input.setSelectionRange(...searchSelection);
  document.querySelector<HTMLSelectElement>("#sort")!.value = sortBy;
  document.querySelector<HTMLSelectElement>("#sort")!.onchange = (event) => { sortBy = (event.target as HTMLSelectElement).value as SortBy; render(); };
  document.querySelectorAll<HTMLElement>("[data-view]").forEach((button) => button.onclick = () => setViewMode(button.dataset.view as ViewMode));
  // 重建 DOM 会把列表滚动位置清零，放在最后恢复，避免聚焦搜索框等操作再次影响滚动
  if (scrollTop) document.querySelector<HTMLElement>("#grid")!.scrollTop = scrollTop;
  bindInteractions(shown);
}
/**
 * 条目悬停提示：卡片正文受宽度限制会截断名称，鼠标移入时用气泡展示完整名称与说明。
 * 全局单例，移开 / 列表滚动 / 重绘时销毁。
 */
let tipNode: HTMLElement | null = null;
function hideTip() { tipNode?.remove(); tipNode = null; }
function showTip(card: HTMLElement, item: Item) {
  hideTip();
  const node = document.createElement("div");
  node.className = "tip";
  node.innerHTML = `<b>${escape(item.name)}</b>${item.description ? `<span>${escape(item.description)}</span>` : ""}<small>${escape(item.target)}</small>`;
  document.body.append(node);
  tipNode = node;
  const box = card.getBoundingClientRect();
  const size = node.getBoundingClientRect();
  // 优先显示在卡片下方，空间不足时翻到上方；左右做视口边界钳制
  const below = box.bottom + 6;
  const top = below + size.height > window.innerHeight - 8 ? box.top - size.height - 6 : below;
  node.style.left = `${Math.min(Math.max(8, box.left), Math.max(8, window.innerWidth - size.width - 8))}px`;
  node.style.top = `${Math.max(8, top)}px`;
}
function bindInteractions(shown: Item[]) {
  document.querySelectorAll<HTMLElement>("[data-action]").forEach((button) => button.onclick = () => void action(button.dataset.action!));
  document.querySelectorAll<HTMLElement>("[data-group]").forEach((tab) => { tab.onclick = () => { activeGroup = tab.dataset.group!; selected = 0; selectedIds.clear(); render(); }; tab.ondragstart = (event) => event.dataTransfer?.setData("group", tab.dataset.group!); tab.ondragover = (event) => event.preventDefault(); tab.ondrop = (event) => void reorderGroups(event, tab.dataset.group!); });
  document.querySelectorAll<HTMLElement>(".card").forEach((card) => {
    // 「直接运行」卡片：单击选中，双击执行
    if (card.dataset.run) {
      card.onclick = () => { selected = shown.length; render(true); };
      card.ondblclick = () => void runCommand();
      return;
    }
    card.onclick = (event) => {
      if ((event.target as HTMLElement).closest(".more")) return;
      // 多选模式下点击卡片切换勾选，不移动键盘光标
      if (selectMode) { const id = card.dataset.id!; if (selectedIds.has(id)) selectedIds.delete(id); else selectedIds.add(id); render(true); return; }
      // 已选中的条目再次点击名称区域：直接进入内联改名
      if (card.dataset.id === shown[selected]?.id && (event.target as HTMLElement).closest(".card-text b")) { renameItemInline(card.dataset.id!); return; }
      selected = shown.findIndex((item) => item.id === card.dataset.id); render(true);
    };
    card.ondblclick = (event) => { if (selectMode || (event.target as HTMLElement).closest(".card-text b")) return; void openItem(card.dataset.id!); };
    card.onmouseenter = () => { const item = shown.find((target) => target.id === card.dataset.id); if (item) showTip(card, item); };
    card.onmouseleave = hideTip;
  });
  document.querySelectorAll<HTMLElement>("[data-menu]").forEach((button) => button.onclick = (event) => { event.stopPropagation(); showItemMenu(button.dataset.menu!, button); });
  document.querySelector("#grid")?.addEventListener("contextmenu", (event) => { if ((event.target as HTMLElement).closest(".card")) return; event.preventDefault(); openItemDialog(); });
  // 滚动时卡片位置变化，悬停提示会错位，直接隐藏
  document.querySelector("#grid")?.addEventListener("scroll", hideTip);
}
async function action(type: string) {
  if (type === "add") openItemDialog();
  if (type === "clear-search") { query = ""; searchSelection = null; selected = 0; render(); }
  if (type === "scan") await openScan();
  if (type === "export") openExportDialog();
  if (type === "groups") openGroupManager();
  if (type === "new-group") openGroupDialog();
  if (type === "password") openPasswordDialog();
  if (type === "lock") {
    // 立即上锁：先同步前端状态，避免 load 时仍带着已解锁标记取回加密条目
    unlocked = false;
    selectedIds.clear();
    await invoke("lock_encrypted");
    await load();
  }
  if (type === "select-mode") { selectMode = !selectMode; selectedIds.clear(); render(true); }
  if (type === "select-all") { displayItems().forEach((item) => selectedIds.add(item.id)); render(true); }
  if (type === "select-invert") { displayItems().forEach((item) => selectedIds.has(item.id) ? selectedIds.delete(item.id) : selectedIds.add(item.id)); render(true); }
  if (type === "select-none") { selectedIds.clear(); render(true); }
  if (type === "delete-items") await deleteSelectedItems();
  if (type === "move-items") openMoveDialog([...selectedIds]);
  if (type === "settings") {
    await invoke("open_settings_window");
  }
  if (type === "pin") {
    pinned = !pinned;
    await invoke("set_panel_pinned", { pinned });
    render();
  }
}
async function openItem(id: string) {
  if (opening) return;
  opening = true; render(true);
  // 启动成功后隐藏面板，回到之前的窗口（双击 / Enter / 菜单「打开」共用此入口）
  try { await invoke("open_item", { id, unlocked }); void invoke("hide_panel"); }
  catch (error) { message(error); }
  finally { opening = false; render(true); }
}
/** 直接运行搜索框内容（仿 Win+R）：成功后隐藏面板，失败保留面板并提示原因 */
async function runCommand() {
  const target = runCandidate();
  if (!target || opening) return;
  opening = true; render(true);
  try { await invoke("run_target", { raw: target }); void invoke("hide_panel"); }
  catch (error) { message(error); }
  finally { opening = false; render(true); }
}
function modal(content: string) {
  const node = document.createElement("div");
  node.className = "overlay";
  node.innerHTML = `<div class="modal">${content}</div>`;
  document.body.append(node);
  // 仅当鼠标「按下」与「松开」都落在遮罩空白处时才关闭：
  // 在弹窗内按下、拖到遮罩上松开会触发 target 为遮罩的 click，此前会误关闭
  let pressedOnBackdrop = false;
  node.addEventListener("mousedown", (event) => { pressedOnBackdrop = event.target === node; });
  node.addEventListener("mouseup", (event) => { if (event.target !== node) pressedOnBackdrop = false; });
  node.addEventListener("click", (event) => {
    const target = event.target as HTMLElement;
    if (target.dataset.close !== undefined) { node.remove(); return; }
    if (target === node && pressedOnBackdrop) node.remove();
    pressedOnBackdrop = false;
  });
  return node;
}
/**
 * 应用内确认弹窗：WebView 中的 window.confirm 不会等待用户点击就返回（导致未确认即执行删除），
 * 故统一改用弹窗确认；点击取消 / 遮罩 / Esc 关闭都返回 false。
 */
function confirmDialog(text: string, okLabel = "删除") {
  return new Promise<boolean>((resolve) => {
    const node = modal(`<h2>确认操作</h2><p class="hint">${escape(text)}</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="danger" id="confirm-ok">${escape(okLabel)}</button></div>`);
    // 遮罩 / Esc 关闭由 modal 内部处理，这里监听节点被移除来兜底返回 false
    const observer = new MutationObserver(() => { if (!node.isConnected) { observer.disconnect(); resolve(false); } });
    observer.observe(document.body, { childList: true });
    node.querySelector<HTMLButtonElement>("#confirm-ok")!.onclick = () => { observer.disconnect(); node.remove(); resolve(true); };
  });
}
function selectedGroup() { return activeGroup === "all" || activeGroup === "none" ? null : activeGroup; }
function openItemDialog(item?: Item) {
  const isCmd = item?.isCmd ?? false; const node = modal(`<h2>${item ? "编辑启动项" : "添加启动项"}</h2><form id="item-form"><label>名称<input name="name" required value="${escape(item?.name ?? "")}" placeholder="例如：工作资料" /></label><label>关键字<input name="keyword" value="${escape(item?.keyword ?? "")}" placeholder="用于搜索，留空则使用名称" /></label><label>说明（可选）<input name="description" value="${escape(item?.description ?? "")}" placeholder="备注信息，鼠标悬停条目时显示" /></label><label>类型<select name="kind"><option value="target">文件、目录、快捷方式或 URL</option><option value="cmd">${cmdLabel}</option></select></label><label>启动目标<div class="with-button"><input name="target" required value="${escape(item?.target ?? "")}" placeholder="绝对路径、https:// 或命令" /><button type="button" id="pick">选择…</button></div></label><label>分组<select name="group"><option value="">未分组</option>${groups.sort((a,b) => a.sortOrder-b.sortOrder).map((group) => `<option value="${group.id}">${escape(group.name)}</option>`).join("")}<option value="${encryptedId}">🔒 加密</option></select></label><label>自定义图标（可选）<div class="with-button"><input name="icon" value="${escape(item?.iconPath ?? "")}" placeholder="图标文件路径" /><button type="button" id="pick-icon">选择…</button></div></label><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary">保存</button></div></form>`); const form = node.querySelector<HTMLFormElement>("#item-form")!; const kind = form.elements.namedItem("kind") as HTMLSelectElement; kind.value = isCmd ? "cmd" : "target"; (form.elements.namedItem("group") as HTMLSelectElement).value = item?.groupId ?? selectedGroup() ?? "";
  node.querySelector<HTMLButtonElement>("#pick")!.onclick = async () => { const path = await chooseFile({ directory: false, multiple: false }); if (typeof path === "string") (form.elements.namedItem("target") as HTMLInputElement).value = path; };
  node.querySelector<HTMLButtonElement>("#pick-icon")!.onclick = async () => { const path = await chooseFile({ multiple: false, filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "ico", "svg"] }] }); if (typeof path === "string") (form.elements.namedItem("icon") as HTMLInputElement).value = path; };
  form.onsubmit = async (event) => { event.preventDefault(); const values = new FormData(form); try { await invoke("save_item", { item: { id: item?.id ?? "", name: values.get("name"), keyword: values.get("keyword") ?? "", description: values.get("description") ?? "", target: values.get("target"), groupId: values.get("group") || null, isEncrypted: false, isCmd: kind.value === "cmd", iconPath: values.get("icon") || null, lnkPath: item?.lnkPath ?? null, sortOrder: item?.sortOrder ?? 0, createdAt: item?.createdAt ?? 0 }, isCmd: kind.value === "cmd" }); node.remove(); await load(); } catch (error) { message(error); } };
}
/** 启动项 ⋯ 菜单：webview 无法渲染到窗口之外，只能收进视口内；优先与按钮右对齐、向下展开，贴近左/右/下边缘时自动翻转或回收，保证四个边缘的条目菜单都完整可见；滚动 / 调整窗口时关闭以免菜单与按钮脱节 */
function showItemMenu(id: string, anchor: HTMLElement) { document.querySelector(".menu")?.remove(); const menu = document.createElement("div"); menu.className = "menu"; menu.innerHTML = `<button data-edit>编辑</button><button data-open>打开</button><button data-move>移动到分组</button><button class="danger" data-delete>删除</button>`; document.body.append(menu); const box = anchor.getBoundingClientRect(); const margin = 6; const mw = menu.offsetWidth, mh = menu.offsetHeight; let left = box.right - mw; if (left < margin) left = box.left; left = Math.min(Math.max(margin, left), window.innerWidth - mw - margin); let top = box.bottom + 4; if (top + mh > window.innerHeight - margin) top = box.top - mh - 4; top = Math.max(margin, top); menu.style.left = `${left}px`; menu.style.top = `${top}px`; const closeMenu = () => menu.remove(); document.querySelector("#grid")?.addEventListener("scroll", closeMenu, { once: true }); window.addEventListener("resize", closeMenu, { once: true }); menu.querySelector("[data-edit]")!.addEventListener("click", () => { menu.remove(); openItemDialog(items.find((item) => item.id === id)); }); menu.querySelector("[data-open]")!.addEventListener("click", () => { menu.remove(); void openItem(id); }); menu.querySelector("[data-move]")!.addEventListener("click", () => { menu.remove(); const item = items.find((target) => target.id === id); if (item) openMoveDialog([id], item.groupId); }); menu.querySelector("[data-delete]")!.addEventListener("click", async () => { menu.remove(); if (!await confirmDialog("确认删除这个启动项？")) return; try { await invoke("delete_item", { id }); await load(); } catch (error) { message(error); } }); }
/** 点击已选中条目的名称：在卡片内直接把名称换成输入框，失焦 / 回车保存，Esc 取消 */
function renameItemInline(id: string) {
  const item = items.find((target) => target.id === id);
  const card = document.querySelector<HTMLElement>(`.card[data-id="${id}"]`);
  const nameEl = card?.querySelector<HTMLElement>(".card-text b");
  if (!item || !card || !nameEl || card.querySelector("input.rename-input")) return;
  const input = document.createElement("input");
  input.className = "rename-input";
  input.value = item.name;
  nameEl.replaceWith(input);
  input.focus();
  input.select();
  let done = false;
  const finish = async (save: boolean) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    if (save && name && name !== item.name) {
      try { await invoke("save_item", { item: { ...item, name }, isCmd: item.isCmd }); await load(); return; }
      catch (error) { message(error); }
    }
    render();
  };
  input.onkeydown = (event) => {
    event.stopPropagation();
    if (event.key === "Enter") { event.preventDefault(); void finish(true); }
    if (event.key === "Escape") void finish(false);
  };
  input.onblur = () => void finish(true);
  input.onclick = (event) => event.stopPropagation();
  input.ondblclick = (event) => event.stopPropagation();
}
/** 批量删除多选模式下勾选的启动项 */
async function deleteSelectedItems() {
  const ids = [...selectedIds];
  if (!ids.length) return;
  if (!await confirmDialog(`确认删除选中的 ${ids.length} 个启动项？`)) return;
  try { await invoke("delete_items", { ids }); selectedIds.clear(); await load(); }
  catch (error) { message(error); }
}
/** 移动到其他分组（可选「未分组」或已有普通分组）：支持批量多选或单个条目，currentGroupId 用于预选当前分组 */
function openMoveDialog(ids: string[], currentGroupId: string | null = null) {
  if (!ids.length) return;
  const options = [{ id: "", name: "未分组" }, ...groups.sort((a, b) => a.sortOrder - b.sortOrder), { id: encryptedId, name: "🔒 加密" }];
  const node = modal(`<h2>移动到分组</h2><form id="move-form"><label>目标分组<select name="group">${options.map((option) => `<option value="${option.id}">${escape(option.name)}</option>`).join("")}</select></label><p class="hint">共 ${ids.length} 项将移动到所选分组。</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary">移动</button></div></form>`);
  // 预选当前分组；不在选项中的值回落到「未分组」
  node.querySelector<HTMLSelectElement>("#move-form select[name='group']")!.value = options.some((option) => option.id === currentGroupId) ? currentGroupId! : "";
  node.querySelector<HTMLFormElement>("#move-form")!.onsubmit = async (event) => {
    event.preventDefault();
    const groupId = String(new FormData(event.currentTarget as HTMLFormElement).get("group") ?? "");
    try { await invoke("move_items", { ids, groupId: groupId || null }); node.remove(); selectedIds.clear(); await load(); }
    catch (error) { message(error); }
  };
}
/** 删除分组：由用户选择仅删除分组，还是连同组内项目一起删除 */
function openDeleteGroupDialog(groupId: string, manager: HTMLElement) {
  const group = groups.find((item) => item.id === groupId);
  const count = items.filter((item) => item.groupId === groupId).length;
  const node = modal(`<h2>删除分组「${escape(group?.name ?? "")}」</h2><form id="delete-group-form"><p class="hint">该分组下有 ${count} 个启动项，请选择处理方式：</p><div class="del-options"><label><input type="radio" name="mode" value="keep" checked />仅删除分组，组内项目转为未分组</label><label><input type="radio" name="mode" value="delete" />删除分组及组内全部项目</label></div><div class="modal-actions"><button type="button" data-close>取消</button><button class="danger">确认删除</button></div></form>`);
  node.querySelector<HTMLFormElement>("#delete-group-form")!.onsubmit = async (event) => {
    event.preventDefault();
    const mode = String(new FormData(event.currentTarget as HTMLFormElement).get("mode"));
    if (mode === "delete" && !await confirmDialog(`将同时删除「${group?.name ?? ""}」中的 ${count} 个启动项？`, "确认删除")) return;
    try { await invoke("delete_group", { id: groupId, deleteItems: mode === "delete" }); node.remove(); manager.remove(); await load(); }
    catch (error) { message(error); }
  };
}
function openGroupDialog(group?: Group) { const node = modal(`<h2>${group ? "重命名分组" : "新建分组"}</h2><form id="group-form"><label>分组名称<input name="name" required value="${escape(group?.name ?? "")}" /></label><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary">保存</button></div></form>`); node.querySelector<HTMLFormElement>("#group-form")!.onsubmit = async (event) => { event.preventDefault(); const name = new FormData(event.currentTarget as HTMLFormElement).get("name"); try { if (group) await invoke("rename_group", { id: group.id, name }); else await invoke("create_group", { name }); node.remove(); await load(); } catch (error) { message(error); } }; }
function openGroupManager() { const node = modal(`<h2>分组管理</h2><div class="group-list">${groups.sort((a,b) => a.sortOrder-b.sortOrder).map((group) => `<div><span>${escape(group.name)}</span><button data-rename="${group.id}">重命名</button><button class="danger" data-delete-group="${group.id}">删除</button></div>`).join("") || "<p class=muted>尚未创建普通分组</p>"}</div><p class="hint">“全部”“未分组”“加密”为固定逻辑标签，不能删除。</p><div class="modal-actions"><button data-close>关闭</button><button class="primary" id="manager-add">新建分组</button></div>`); node.querySelector("#manager-add")!.addEventListener("click", () => { node.remove(); openGroupDialog(); }); node.querySelectorAll<HTMLElement>("[data-rename]").forEach((button) => button.onclick = () => { node.remove(); openGroupDialog(groups.find((group) => group.id === button.dataset.rename)); }); node.querySelectorAll<HTMLElement>("[data-delete-group]").forEach((button) => button.onclick = () => openDeleteGroupDialog(button.dataset.deleteGroup!, node)); }
function openPasswordDialog() { const resetting = passwordSet && unlocked; const node = modal(`<h2>${resetting ? "重置加密密码" : passwordSet ? "解锁加密分组" : "设置加密密码"}</h2><form id="password-form">${resetting ? `<label>当前密码<input name="current" type="password" required /></label>` : ""}<label>${passwordSet && !resetting ? "密码" : "新密码"}<input name="password" type="password" minlength="4" required /></label><p class="hint">密码仅以随机 salt 的 SHA-256 哈希保存，无法找回。</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary">确认</button></div></form>${passwordSet && !resetting ? `<p class="hint" style="margin-top:8px"><button type="button" id="forgot-password" class="link">忘记密码？清空加密分组并重新设置</button></p>` : ""}`); node.querySelector<HTMLFormElement>("#password-form")!.onsubmit = async (event) => { event.preventDefault(); const values = new FormData(event.currentTarget as HTMLFormElement); const password = String(values.get("password")); try { if (passwordSet && !resetting) { if (!await invoke<boolean>("unlock_encrypted", { password })) throw "密码不正确"; unlocked = true; } else { await invoke("reset_password", { currentPassword: values.get("current") || null, newPassword: password }); unlocked = true; } node.remove(); activeGroup = encryptedId; await load(); } catch (error) { message(error); } }; node.querySelector<HTMLButtonElement>("#forgot-password")?.addEventListener("click", () => openClearEncryptedConfirm(node)); }

/** 忘记密码：二次确认后清空加密分组全部条目并重置密码 */
function openClearEncryptedConfirm(parent: HTMLElement) {
  const count = items.filter((item) => item.isEncrypted).length;
  const node = modal(`<h2>忘记密码</h2><p class="hint">将删除加密分组内的 ${count} 个启动项并重置密码，删除后无法恢复。确定继续吗？</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="danger" id="confirm-clear">确认清空</button></div>`);
  node.querySelector<HTMLButtonElement>("#confirm-clear")!.onclick = async () => {
    try { await invoke("clear_encrypted"); parent.remove(); node.remove(); unlocked = false; activeGroup = encryptedId; await load(); } catch (error) { message(error); }
  };
}
const scanMeta = (entry: DesktopEntry) => entry.entryType === "shortcut" ? { label: shortcutLabel, icon: "↗" } : entry.entryType === "system" ? { label: "系统", icon: "🖥" } : entry.entryType === "folder" ? { label: "文件夹", icon: "▰" } : { label: "文件", icon: "▤" };
function scanIcon(entry: DesktopEntry) { return entry.scanIconPath ? `<img src="${escape(convertFileSrc(entry.scanIconPath))}" alt="" />` : scanMeta(entry).icon; }
function updateScanSelection(node: HTMLElement) {
  const checked = node.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry:checked").length;
  const total = node.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry").length;
  const count = node.querySelector<HTMLElement>("#scan-count");
  if (count) count.textContent = `已选 ${checked} / ${total}`;
}
/** 渲染扫描结果（卡片 / 列表两种扁平布局共用，外层 class 控制形态） */
function renderScanEntries(entries: DesktopEntry[], chosen: Set<number>) {
  return entries.map((entry, index) => { const meta = scanMeta(entry); const folder = entry.folderName ? `<i class="scan-folder" title="所在文件夹">▰ ${escape(entry.folderName)}</i>` : ""; return `<label><input type="checkbox" class="scan-entry" value="${index}" data-type="${entry.entryType}" ${chosen.has(index) ? "checked" : ""}/><i class="scan-icon ${entry.entryType}">${scanIcon(entry)}</i><span class="scan-name">${escape(entry.name)}<em>${meta.label}</em>${folder}</span><small>${escape(entry.target)}</small></label>`; }).join("") || "<p class=muted>该目录没有可导入的文件或目录</p>";
}
/** 扫描结果树节点：children 为子目录，files 为当前目录下的文件 / 快捷方式索引 */
type ScanTreeNode = { name: string; folderIndex: number | null; children: ScanTreeNode[]; files: number[] };
/** 按相对路径把扫描结果组织成目录树 */
function buildScanTree(entries: DesktopEntry[]) {
  const root: ScanTreeNode = { name: "", folderIndex: null, children: [], files: [] };
  const dirs = new Map<string, ScanTreeNode>();
  const openDir = (segments: string[]) => {
    let current = root; let key = "";
    for (const segment of segments) {
      key = key ? `${key}/${segment}` : segment;
      let child = dirs.get(key);
      if (!child) { child = { name: segment, folderIndex: null, children: [], files: [] }; dirs.set(key, child); current.children.push(child); }
      current = child;
    }
    return current;
  };
  entries.forEach((entry, index) => {
    const segments = entry.relPath?.length ? entry.relPath : [entry.name];
    if (entry.entryType === "folder") openDir(segments).folderIndex = index;
    else openDir(segments.slice(0, -1)).files.push(index);
  });
  return root;
}
/** 渲染树状扫描结果：目录层级缩进，节点默认展开 */
function renderScanTree(entries: DesktopEntry[], chosen: Set<number>, rootLabel: string) {
  if (!entries.length) return "<p class=muted>该目录没有可导入的文件或目录</p>";
  const sortByName = (list: number[]) => [...list].sort((a, b) => entries[a].name.localeCompare(entries[b].name, "zh"));
  const renderNode = (treeNode: ScanTreeNode, isRoot = false): string => {
    const children = [...treeNode.children].sort((a, b) => a.name.localeCompare(b.name, "zh"));
    const folderIndex = treeNode.folderIndex;
    const entry = folderIndex === null ? null : entries[folderIndex];
    const box = entry
      ? `<input type="checkbox" class="scan-entry" value="${folderIndex}" data-type="${entry.entryType}" ${chosen.has(folderIndex!) ? "checked" : ""}/>`
      : `<span class="scan-toggle-spacer"></span>`;
    const hasChildren = children.length > 0 || treeNode.files.length > 0;
    const files = sortByName(treeNode.files).map((index) => {
      const file = entries[index];
      return `<label class="scan-file"><span class="scan-toggle-spacer"></span><input type="checkbox" class="scan-entry" value="${index}" data-type="${file.entryType}" ${chosen.has(index) ? "checked" : ""}/><i class="scan-icon ${file.entryType}">${scanIcon(file)}</i><span class="scan-name">${escape(file.name)}<em>${scanMeta(file).label}</em></span><small>${escape(file.target)}</small></label>`;
    }).join("");
    return `<div class="scan-node${hasChildren ? " has-children" : ""}">
      <div class="scan-row">
        ${hasChildren ? `<button type="button" class="scan-toggle" title="展开 / 折叠">▾</button>` : `<span class="scan-toggle-spacer"></span>`}
        ${box}
        <i class="scan-icon ${entry ? entry.entryType : "folder"}">${entry ? scanIcon(entry) : "▰"}</i>
        <span class="scan-name">${escape(isRoot ? rootLabel : treeNode.name)}${entry ? `<em>${scanMeta(entry).label}</em>` : ""}${hasChildren ? `<em>${children.length + treeNode.files.length} 项</em>` : ""}</span>
        <small>${escape(entry ? entry.target : "")}</small>
      </div>
      <div class="scan-children">${children.map((child) => renderNode(child)).join("")}${files}</div>
    </div>`;
  };
  return renderNode(buildScanTree(entries), true);
}
/** 导入结果弹窗：汇总导入数量，并列出重复项（含已有条目的所在分组）与失败项（含失败原因）；
 *  若成功导入的条目含 .lnk 快捷方式，额外提供「删除源快捷方式」选项（只删快捷方式文件本身，不动其指向的程序 / 文件） */
function showImportResult(result: ImportResult) {
  const row = (skip: ImportSkip, tag: string) => `<li><span class="skip-name">${escape(skip.name)}<em>${escape(tag)}</em></span><small>${escape(skip.target)}</small></li>`;
  const skipped = result.skippedDuplicates + result.skippedInvalid;
  const duplicates = result.duplicates.length
    ? `<h3>重复已跳过（${result.skippedDuplicates}）</h3><ul class="skip-list">${result.duplicates.map((skip) => row(skip, `已存在于「${skip.groupName ?? "未分组"}」`)).join("")}</ul>` : "";
  const invalids = result.invalids.length
    ? `<h3>导入失败（${result.skippedInvalid}）</h3><ul class="skip-list">${result.invalids.map((skip) => row(skip, skip.reason ?? "未知原因")).join("")}</ul>` : "";
  const shortcuts = result.importedShortcuts;
  const cleanup = shortcuts.length
    ? `<div id="shortcut-cleanup"><h3>清理源快捷方式</h3><p class="hint">本次导入包含 ${shortcuts.length} 个快捷方式，已从原位置记录到启动器。如不再需要，可删除原位置的快捷方式文件——只删除 .lnk 快捷方式本身，其指向的程序 / 文件不受影响。</p><div class="modal-actions"><button type="button" class="danger" id="delete-shortcuts">删除这 ${shortcuts.length} 个快捷方式</button></div></div>` : "";
  const node = modal(`<h2>导入完成</h2><p class="hint">已导入 ${result.added} 项${skipped ? `；跳过重复 ${result.skippedDuplicates} 项、失败 ${result.skippedInvalid} 项，明细如下。` : "。"}</p>${duplicates}${invalids}${cleanup}<div class="modal-actions"><button class="primary" data-close>知道了</button></div>`);
  const deleteButton = node.querySelector<HTMLButtonElement>("#delete-shortcuts");
  deleteButton?.addEventListener("click", async () => {
    deleteButton.disabled = true;
    deleteButton.innerHTML = `<i class="loading-dot"></i>正在删除…`;
    try {
      const deleted = await invoke<DeleteShortcutsResult>("delete_shortcut_files", { paths: shortcuts });
      const failures = deleted.failures.length
        ? `<h3>删除失败（${deleted.failures.length}）</h3><ul class="skip-list">${deleted.failures.map((skip) => row(skip, skip.reason ?? "未知原因")).join("")}</ul>` : "";
      node.querySelector<HTMLElement>("#shortcut-cleanup")!.innerHTML = `<p class="hint">已删除 ${deleted.deleted} 个快捷方式文件${failures ? "，以下删除失败：" : "。"}</p>${failures}`;
    } catch (error) {
      deleteButton.disabled = false;
      deleteButton.textContent = `删除这 ${shortcuts.length} 个快捷方式`;
      message(error);
    }
  });
}
async function openScan() {
  let scanView: "tree" | "card" | "list" = "tree";
  let chosen = new Set<number>();
  // macOS 默认扫描「应用程序」目录（/Applications + ~/Applications），Windows 默认扫描桌面
  const defaultSource: ScanSource = isMac ? { kind: "start-menu" } : { kind: "desktop" };
  const defaultSourceLabel = isMac ? startMenuLabel : "桌面";
  const hintText = isMac
    ? `默认扫描${startMenuLabel}目录（/Applications），也可以选择其他文件夹；仅在导入所选快捷方式时解析真实目标，重复项会自动忽略。`
    : `默认扫描桌面（含此电脑、回收站等系统图标），也可以选择其他文件夹或${startMenuLabel}；仅在导入所选快捷方式时解析真实目标，重复项会自动忽略。`;
  const node = modal(`<h2>扫描导入<button type="button" class="scan-max" id="scan-max" title="最大化 / 还原">⛶</button></h2><p class="hint">${hintText}</p>
    <div class="scan-bulk"><span id="scan-source">来源：${defaultSourceLabel}</span><div><label class="scan-check"><input type="checkbox" id="scan-recursive" />扫描子文件夹</label><label class="scan-check"><input type="checkbox" id="group-by-folder" />按文件夹名称分组</label><button type="button" id="pick-start-menu">${startMenuLabel}</button><button type="button" id="pick-folder">选择文件夹…</button></div></div>
    <div id="scan-body"></div>
    <div class="modal-actions"><button data-close>取消</button><button class="primary" id="import">导入已选</button></div>`);
  node.classList.add("scan-overlay");
  const scanModal = node.querySelector<HTMLElement>(".modal")!;
  // 最大化切换：窗口 95% 铺满，再次点击还原
  node.querySelector<HTMLButtonElement>("#scan-max")!.onclick = () => scanModal.classList.toggle("max");
  const sourceText = node.querySelector<HTMLElement>("#scan-source")!;
  const body = node.querySelector<HTMLElement>("#scan-body")!;
  const importButton = node.querySelector<HTMLButtonElement>("#import")!;
  const recursiveInput = node.querySelector<HTMLInputElement>("#scan-recursive")!;
  const groupByFolderInput = node.querySelector<HTMLInputElement>("#group-by-folder")!;
  const describe = (src: ScanSource) => src.kind === "folder" ? src.dir : src.kind === "start-menu" ? startMenuLabel : "桌面";
  let currentSource: ScanSource = defaultSource;
  const refresh = async (source: ScanSource) => {
    currentSource = source;
    importButton.disabled = true;
    body.innerHTML = `<p class="muted"><i class="loading-dot"></i>正在扫描…</p>`;
    const recursive = recursiveInput.checked;
    try {
      scanning = source.kind === "folder" ? await invoke<DesktopEntry[]>("scan_folder", { dir: source.dir, recursive })
        : await invoke<DesktopEntry[]>(source.kind === "start-menu" ? "scan_start_menu" : "scan_desktop", { recursive });
    }
    catch (error) { body.innerHTML = ""; importButton.disabled = false; message(error); return; }
    sourceText.textContent = `来源：${describe(source)}${recursive ? "（含子文件夹）" : ""}`;
    // 默认勾选全部快捷方式
    chosen = new Set(scanning.map((entry, index) => entry.isShortcut || entry.entryType === "system" ? index : -1).filter((index) => index >= 0));
    const countByType = (type: DesktopEntry["entryType"]) => scanning.filter((entry) => entry.entryType === type).length;
    const rootLabel = source.kind === "folder" ? (source.dir.split(/[\\/]/).filter(Boolean).pop() ?? source.dir) : describe(source);
    const syncChosen = () => { chosen = new Set([...body.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry:checked")].map((input) => Number(input.value))); };
    const bindItems = () => {
      const container = body.querySelector<HTMLElement>(".scan-items")!;
      if (scanView !== "tree") {
        container.querySelectorAll<HTMLInputElement>("input.scan-entry").forEach((input) => input.onchange = () => { syncChosen(); updateScanSelection(node); });
        return;
      }
      // 父子勾选不关联：勾文件夹不等于勾选其内容，选快捷方式也不会带上父目录
      // 父级整行（名称、图标、空白处）点击均可展开 / 折叠，勾选框与展开按钮除外
      container.querySelectorAll<HTMLElement>(".scan-node.has-children").forEach((treeNode) => {
        const row = treeNode.querySelector<HTMLElement>(":scope > .scan-row")!;
        const toggle = () => { treeNode.classList.toggle("collapsed"); row.querySelector<HTMLElement>(".scan-toggle")!.textContent = treeNode.classList.contains("collapsed") ? "▸" : "▾"; };
        row.querySelector<HTMLButtonElement>(".scan-toggle")?.addEventListener("click", (event) => { event.preventDefault(); toggle(); });
        row.addEventListener("click", (event) => { if (!(event.target as HTMLElement).closest("input, button")) toggle(); });
      });
      container.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry").forEach((input) => input.onchange = () => { syncChosen(); updateScanSelection(node); });
    };
    const renderList = () => {
      body.innerHTML = `<div class="scan-bulk"><span id="scan-count"></span><div class="scan-view-toggle"><button type="button" data-scan-view="tree" class="${scanView === "tree" ? "active" : ""}" title="树状布局">🌲</button><button type="button" data-scan-view="card" class="${scanView === "card" ? "active" : ""}" title="卡片布局">▦</button><button type="button" data-scan-view="list" class="${scanView === "list" ? "active" : ""}" title="列表布局">☰</button></div><div><button type="button" data-select="shortcut">全选快捷方式 (${countByType("shortcut")})</button><button type="button" data-select="file">全选文件 (${countByType("file")})</button><button type="button" data-select="folder">全选文件夹 (${countByType("folder")})</button>${countByType("system") ? `<button type="button" data-select="system">全选系统 (${countByType("system")})</button>` : ""}<button type="button" data-select="all">全选</button><button type="button" data-select="none">取消全选</button></div></div>
        <div class="scan-items ${scanView}">${scanView === "tree" ? renderScanTree(scanning, chosen, rootLabel) : renderScanEntries(scanning, chosen)}</div>`;
      const toggle = (type: string) => {
        body.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry").forEach((input) => {
          input.checked = type === "all" ? true : type === "none" ? false : input.dataset.type === type;
        });
        syncChosen();
        updateScanSelection(node);
      };
      body.querySelectorAll<HTMLButtonElement>("[data-select]").forEach((button) => button.onclick = () => toggle(button.dataset.select!));
      // 视图切换：重建列表，保留已勾选状态
      body.querySelectorAll<HTMLButtonElement>("[data-scan-view]").forEach((button) => button.onclick = () => {
        syncChosen();
        scanView = button.dataset.scanView as "tree" | "card" | "list";
        renderList();
      });
      bindItems();
      updateScanSelection(node);
    };
    renderList();
    importButton.disabled = false;
  };
  node.querySelector<HTMLButtonElement>("#pick-folder")!.onclick = async () => {
    const dir = await chooseFile({ directory: true, multiple: false });
    if (typeof dir === "string") await refresh({ kind: "folder", dir });
  };
  // 开始菜单 / 应用程序目录默认只扫当前层（子文件夹层级多、扫描慢），需要时手动勾选「扫描子文件夹」
  node.querySelector<HTMLButtonElement>("#pick-start-menu")!.onclick = async () => { await refresh({ kind: "start-menu" }); };
  recursiveInput.onchange = () => { void refresh(currentSource); };
  importButton.addEventListener("click", async () => {
    const selectedEntries = [...node.querySelectorAll<HTMLInputElement>(".scan-items input.scan-entry:checked")].map((input) => scanning[Number(input.value)]);
    if (!selectedEntries.length) { message("请至少选择一项"); return; }
    importButton.disabled = true; importButton.innerHTML = `<i class="loading-dot"></i>正在导入…`;
    node.querySelectorAll<HTMLButtonElement>("button").forEach((control) => { if (control !== importButton) control.disabled = true; });
    try {
      const result = await invoke<ImportResult>("import_scanned", { entries: selectedEntries, groupId: selectedGroup(), groupByFolder: groupByFolderInput.checked });
      node.remove(); await load();
      showImportResult(result);
    } catch (error) {
      importButton.disabled = false; importButton.textContent = "导入已选";
      node.querySelectorAll<HTMLButtonElement>("button").forEach((control) => control.disabled = false);
      message(error);
    }
  });
  await refresh(currentSource);
}
/** 导出形态提示（与后端分类规则一致，后端最终以文件系统状态为准） */
function exportKindLabel(item: Item) {
  if (item.isCmd) return "BAT";
  if (/^https?:\/\//.test(item.target)) return "URL";
  if (item.lnkPath || /\.(exe|com|bat|cmd|ps1|msi|app|lnk|url)$/i.test(item.target)) return "快捷方式";
  return "不导出";
}
/** 就地解锁加密分组（不切换标签页）：导出加密条目前的密码校验；取消 / 关闭返回 false */
function promptUnlock() {
  return new Promise<boolean>((resolve) => {
    const node = modal(`<h2>解锁加密分组</h2><form id="unlock-form"><label>密码<input name="password" type="password" required autofocus /></label><p class="hint">导出加密分组的启动项前需要先解锁。</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary">解锁</button></div></form>`);
    const observer = new MutationObserver(() => { if (!node.isConnected) { observer.disconnect(); resolve(false); } });
    observer.observe(document.body, { childList: true });
    node.querySelector<HTMLFormElement>("#unlock-form")!.onsubmit = async (event) => {
      event.preventDefault();
      const password = String(new FormData(event.currentTarget as HTMLFormElement).get("password"));
      try {
        if (!await invoke<boolean>("unlock_encrypted", { password })) throw "密码不正确";
        unlocked = true;
        observer.disconnect();
        node.remove();
        resolve(true);
      } catch (error) { message(error); }
    };
  });
}
/** 导出结果弹窗：成功数量 + 未导出 / 失败明细，可一键打开目标文件夹查看 */
function showExportResult(result: ExportResult) {
  const row = (skip: ImportSkip, tag: string) => `<li><span class="skip-name">${escape(skip.name)}<em>${escape(tag)}</em></span><small>${escape(skip.target)}</small></li>`;
  const skipped = result.skipped.length
    ? `<h3>未导出（${result.skipped.length}）</h3><ul class="skip-list">${result.skipped.map((skip) => row(skip, skip.reason ?? "不导出")).join("")}</ul>` : "";
  const failures = result.failures.length
    ? `<h3>导出失败（${result.failures.length}）</h3><ul class="skip-list">${result.failures.map((skip) => row(skip, skip.reason ?? "未知原因")).join("")}</ul>` : "";
  const detail = result.skipped.length || result.failures.length ? `；未导出 ${result.skipped.length} 项、失败 ${result.failures.length} 项，明细如下。` : "。";
  const node = modal(`<h2>导出完成</h2><p class="hint">已导出 ${result.exported} 项到「${escape(result.dir)}」${detail}</p>${skipped}${failures}<div class="modal-actions"><button data-close>完成</button><button class="primary" id="export-open-folder">打开文件夹查看</button></div>`);
  node.querySelector<HTMLButtonElement>("#export-open-folder")!.onclick = async () => {
    try { await invoke("reveal_path", { path: result.dir }); } catch (error) { message(error); }
  };
}
/** 还原启动项到文件夹：选择目标文件夹，按分组树勾选要导出的条目（默认全选），加密分组需先解锁 */
function openExportDialog() {
  let dir = "";
  const chosen = new Set<string>();
  const seen = new Set<string>();
  const node = modal(`<h2>还原到文件夹</h2><form id="export-form"><p class="hint">把启动项还原为文件夹中的快捷方式、${cmdLabel}（.bat）或网址文件（.url）；文件和文件夹类型的启动项不会导出。</p><label>导出到<div class="with-button"><input id="export-dir" readonly placeholder="点击「选择…」指定文件夹" /><button type="button" id="export-pick">选择…</button></div></label><label class="export-option"><input type="checkbox" id="export-group-folders" checked />按分组创建文件夹（「未分组」的条目直接放在根目录）</label><div class="scan-bulk"><span id="export-count"></span><div><button type="button" data-export-select="all">全选</button><button type="button" data-export-select="none">取消全选</button></div></div><div class="scan-items tree" id="export-tree"></div><div class="modal-actions"><button type="button" data-close>取消</button><button class="primary" id="export-run">导出</button></div></form>`);
  node.classList.add("export-overlay");
  const tree = node.querySelector<HTMLElement>("#export-tree")!;
  const dirInput = node.querySelector<HTMLInputElement>("#export-dir")!;
  const groupFoldersInput = node.querySelector<HTMLInputElement>("#export-group-folders")!;
  type Section = { id: string; name: string; locked: boolean; entries: Item[] };
  let currentSections: Section[] = [];
  let currentTotal = 0;
  const computeSections = (): Section[] => {
    const sections: Section[] = [];
    const ungrouped = items.filter((item) => !item.groupId && !item.isEncrypted);
    if (ungrouped.length) sections.push({ id: "", name: "未分组", locked: false, entries: ungrouped });
    for (const group of [...groups].sort((a, b) => a.sortOrder - b.sortOrder)) {
      const entries = items.filter((item) => item.groupId === group.id && !item.isEncrypted);
      if (entries.length) sections.push({ id: group.id, name: group.name, locked: false, entries });
    }
    const encryptedItems = items.filter((item) => item.isEncrypted || item.groupId === encryptedId);
    // 已解锁且有加密条目 → 正常展示勾选；未解锁但设有密码 → 显示锁定行（未解锁时加密条目不会返回前端）
    if (unlocked ? encryptedItems.length > 0 : passwordSet) sections.push({ id: encryptedId, name: "加密", locked: !unlocked, entries: encryptedItems });
    return sections;
  };
  const syncGroupBoxStates = () => {
    tree.querySelectorAll<HTMLInputElement>("input.export-group").forEach((box) => {
      const section = currentSections.find((item) => item.id === box.dataset.section);
      if (!section) return;
      const count = section.entries.filter((entry) => chosen.has(entry.id)).length;
      box.checked = count > 0 && count === section.entries.length;
      box.indeterminate = count > 0 && count < section.entries.length;
    });
    node.querySelector<HTMLElement>("#export-count")!.textContent = `已选 ${chosen.size} / ${currentTotal}`;
  };
  const renderTree = () => {
    currentSections = computeSections();
    currentTotal = currentSections.reduce((sum, section) => sum + section.entries.length, 0);
    // 新出现的条目默认勾选（首次渲染 = 全选；解锁后新载入的加密条目同样默认勾选）
    for (const section of currentSections) for (const entry of section.entries) if (!seen.has(entry.id)) { seen.add(entry.id); chosen.add(entry.id); }
    tree.innerHTML = currentSections.map((section) => {
      if (section.locked) return `<div class="scan-node"><div class="scan-row"><span class="scan-toggle-spacer"></span><input type="checkbox" disabled /><i class="scan-icon folder">🔒</i><span class="scan-name">${escape(section.name)}<em>已锁定</em></span><button type="button" class="export-unlock">解锁后导出</button></div></div>`;
      return `<div class="scan-node has-children"><div class="scan-row"><button type="button" class="scan-toggle" title="展开 / 折叠">▾</button><input type="checkbox" class="export-group" data-section="${escape(section.id)}" /><i class="scan-icon folder">▰</i><span class="scan-name">${escape(section.name)}<em>${section.entries.length} 项</em></span></div><div class="scan-children">${section.entries.map((entry) => `<label class="scan-file"><input type="checkbox" class="export-entry" data-section="${escape(section.id)}" value="${entry.id}" ${chosen.has(entry.id) ? "checked" : ""} /><i class="scan-icon">${itemIcon(entry)}</i><span class="scan-name">${escape(entry.name)}<em>${exportKindLabel(entry)}</em></span><small>${escape(entry.isCmd ? cmdLabel : entry.target)}</small></label>`).join("")}</div></div>`;
    }).join("") || "<p class=muted>还没有可导出的启动项</p>";
    tree.querySelectorAll<HTMLInputElement>("input.export-group").forEach((box) => box.onchange = () => {
      const section = currentSections.find((item) => item.id === box.dataset.section);
      if (!section) return;
      section.entries.forEach((entry) => { if (box.checked) chosen.add(entry.id); else chosen.delete(entry.id); });
      tree.querySelectorAll<HTMLInputElement>(`input.export-entry[data-section="${CSS.escape(box.dataset.section!)}"]`).forEach((input) => { input.checked = box.checked; });
      syncGroupBoxStates();
    });
    tree.querySelectorAll<HTMLInputElement>("input.export-entry").forEach((box) => box.onchange = () => {
      if (box.checked) chosen.add(box.value); else chosen.delete(box.value);
      syncGroupBoxStates();
    });
    // 分组展开 / 折叠：直接切换 class，不重建列表，保留勾选状态；点击父级整行（勾选框与按钮除外）也可切换
    tree.querySelectorAll<HTMLElement>(".scan-node.has-children").forEach((treeNode) => {
      const row = treeNode.querySelector<HTMLElement>(":scope > .scan-row")!;
      const toggleButton = row.querySelector<HTMLButtonElement>(":scope > .scan-toggle");
      const toggle = () => { treeNode.classList.toggle("collapsed"); if (toggleButton) toggleButton.textContent = treeNode.classList.contains("collapsed") ? "▸" : "▾"; };
      toggleButton?.addEventListener("click", (event) => { event.preventDefault(); toggle(); });
      row.addEventListener("click", (event) => { if (!(event.target as HTMLElement).closest("input, button")) toggle(); });
    });
    tree.querySelector<HTMLButtonElement>(".export-unlock")?.addEventListener("click", async () => {
      if (await promptUnlock()) { await load(); renderTree(); }
    });
    syncGroupBoxStates();
  };
  node.querySelectorAll<HTMLButtonElement>("[data-export-select]").forEach((button) => button.onclick = () => {
    const all = button.dataset.exportSelect === "all";
    chosen.clear();
    if (all) for (const section of currentSections) for (const entry of section.entries) chosen.add(entry.id);
    tree.querySelectorAll<HTMLInputElement>("input.export-entry").forEach((input) => { input.checked = all; });
    syncGroupBoxStates();
  });
  node.querySelector<HTMLButtonElement>("#export-pick")!.onclick = async () => {
    const picked = await chooseFile({ directory: true, multiple: false });
    if (typeof picked === "string") { dir = picked; dirInput.value = picked; }
  };
  const runButton = node.querySelector<HTMLButtonElement>("#export-run")!;
  node.querySelector<HTMLFormElement>("#export-form")!.onsubmit = async (event) => {
    event.preventDefault();
    if (!dir) { message("请先选择导出文件夹"); return; }
    if (!chosen.size) { message("请至少选择一个启动项"); return; }
    runButton.disabled = true;
    runButton.innerHTML = `<i class="loading-dot"></i>正在导出…`;
    try {
      const result = await invoke<ExportResult>("export_items", { request: { dir, groupFolders: groupFoldersInput.checked, itemIds: [...chosen] } });
      node.remove();
      showExportResult(result);
    } catch (error) {
      runButton.disabled = false;
      runButton.textContent = "导出";
      message(error);
    }
  };
  renderTree();
}
async function reorderGroups(event: DragEvent, targetId: string) { event.preventDefault(); const sourceId = event.dataTransfer?.getData("group"); if (!sourceId || sourceId === targetId) return; const reordered = groups.sort((a,b) => a.sortOrder-b.sortOrder); const from = reordered.findIndex((group) => group.id === sourceId); const to = reordered.findIndex((group) => group.id === targetId); if (from < 0 || to < 0) return; reordered.splice(to, 0, reordered.splice(from, 1)[0]); try { await invoke("save_group_order", { ids: reordered.map((group) => group.id) }); await load(); } catch (error) { message(error); } }
// Esc 逐层关闭：最上层弹窗 → 「⋯」菜单 → 隐藏面板（内联改名输入框已 stopPropagation，不会触发到这里）
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    const overlays = document.querySelectorAll<HTMLElement>(".overlay");
    const topOverlay = overlays[overlays.length - 1];
    if (topOverlay) { topOverlay.remove(); return; }
    const menu = document.querySelector<HTMLElement>(".menu");
    if (menu) { menu.remove(); return; }
    void invoke("hide_panel");
    return;
  }
  const target = event.target as HTMLElement;
  if (target.matches("input, textarea, select")) return;
  const shown = displayItems();
  if (event.key === "Enter" && shown[selected]) { event.preventDefault(); void openItem(shown[selected].id); }
  if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key) && shown.length) { event.preventDefault(); const width = arrowColumns(); const delta = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" ? -width : width; selected = (selected + delta + shown.length) % shown.length; render(true); }
});
void load();
// 面板重新显示时同步后端解锁状态：选择「每次打开面板都需输入密码」时，隐藏即上锁，重开需重新输入
// 「重新打开面板时清空搜索框」开启时（默认），清空上次的搜索词，从全部条目重新开始；关闭则保留
listen("panel-shown", () => {
  if (appConfig?.clearSearchOnReopen !== false) {
    query = "";
    searchSelection = null;
    selected = 0;
    // 立即用现有数据重绘：不等 load() 的 IPC 往返，避免面板显示后仍残留旧搜索词（感知延迟）
    render();
  }
  void load();
});
// 条目「⋯」菜单：点击菜单外任意位置即关闭（「⋯」按钮自身已 stopPropagation，不影响刚打开的菜单）
document.addEventListener("click", (event) => {
  if ((event.target as HTMLElement).closest(".menu")) return;
  document.querySelector(".menu")?.remove();
});
// 切到其他窗口再切回时，WebView 不会自动恢复焦点：主动把光标收回搜索框
// 弹窗打开 / 正在内联改名时不抢焦点
window.addEventListener("focus", () => {
  if (document.querySelector(".overlay") || document.querySelector("input.rename-input")) return;
  const input = document.querySelector<HTMLInputElement>("#search");
  if (!input) return;
  input.focus({ preventScroll: true });
  if (searchSelection) input.setSelectionRange(...searchSelection);
});
// 应用外观配置（主题、高亮色、字号、窗口效果、面板背景）并监听设置页变更实时生效
loadConfig().then(async (cfg) => {
  appConfig = cfg;
  viewMode = cfg.viewMode === "list" ? "list" : "card";
  applyAppearance(cfg);
  applyWindowEffect(cfg.windowEffect);
  await applyWindowBackground(cfg.background, true);
  pinned = cfg.alwaysOnTop;
  render();
});
listen<AppConfig>("config-changed", async (event) => {
  const cfg = event.payload;
  const runChanged = appConfig !== null && appConfig.runTargetEnabled !== cfg.runTargetEnabled;
  appConfig = cfg;
  appConfig = cfg;
  applyAppearance(cfg);
  applyWindowEffect(cfg.windowEffect);
  await applyWindowBackground(cfg.background, true);
  const mode: ViewMode = cfg.viewMode === "list" ? "list" : "card";
  const viewChanged = mode !== viewMode;
  viewMode = mode;
  // 钉住状态可能来自托盘菜单，显示方式 / 直接运行开关可能来自设置页，需实时反映到面板
  if (cfg.alwaysOnTop !== pinned || viewChanged || runChanged) {
    pinned = cfg.alwaysOnTop;
    render();
  }
});

// ---------- 应用内更新：启动自动检查，发现新版在面板顶部显示横幅 ----------
const updateBannerEl = document.createElement("div");
updateBannerEl.id = "update-banner";
updateBannerEl.className = "update-banner";
updateBannerEl.hidden = true;
updateBannerEl.innerHTML = `<span id="update-banner-text">发现新版本</span><button id="update-banner-btn" class="small" type="button">去更新</button>`;
document.body.prepend(updateBannerEl);
updateBannerEl.querySelector<HTMLButtonElement>("#update-banner-btn")!.onclick = async () => {
  await invoke("open_settings_window");
  await emit("open-update-tab");
};
if (autoCheckEnabled()) {
  checkUpdate().then((info) => {
    if (info) {
      updateBannerEl.querySelector<HTMLSpanElement>("#update-banner-text")!.textContent = `发现新版本 v${info.version}`;
      updateBannerEl.hidden = false;
    }
  });
}
