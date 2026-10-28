import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { AppConfig, BackgroundConfig, applyAppearance, applyWindowBackground, applyWindowEffect, defaultBackground, formatHotkey, isMac, loadConfig, saveConfig, styleBgLayer, DEFAULT_HOTKEY } from "./config";
import { relaunch } from "@tauri-apps/plugin-process";
import { autoCheckEnabled, checkUpdate, resetUpdateCache, setAutoCheckEnabled, UpdateInfo } from "./updater";
import "./styles.css";

let config: AppConfig;
let recordingHotkey = false;

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const markDirty = () => { $("#save-msg").textContent = "有未保存的修改"; };
// 默认数据目录：Windows 为程序（exe）目录，macOS 为 ~/Library/Application Support
const defaultDirLabel = isMac ? "应用支持目录" : "程序目录";

// ---------- 应用内弹窗 ----------
// WebView 中原生 confirm / alert 不会阻塞等待用户点击（对话框刚显示代码就继续执行），故统一改用 DOM 弹窗
const escapeHtml = (text: string) => text.replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!);

/** 创建遮罩弹窗；点击 [data-close] 元素或遮罩空白处关闭 */
function overlay(html: string) {
  const node = document.createElement("div");
  node.className = "overlay";
  node.innerHTML = `<div class="modal">${html}</div>`;
  document.body.append(node);
  // 仅当鼠标「按下」与「松开」都落在遮罩空白处时才关闭，避免在弹窗内按下、拖到遮罩上松开时误关闭
  let pressedOnBackdrop = false;
  node.addEventListener("mousedown", (event) => { pressedOnBackdrop = event.target === node; });
  node.addEventListener("mouseup", (event) => { if (event.target !== node) pressedOnBackdrop = false; });
  node.addEventListener("click", (event) => {
    const target = event.target as HTMLElement;
    // 用 closest 兜底：点到按钮内的文字等子节点也能命中 data-close
    if (target.closest("[data-close]")) { node.remove(); return; }
    if (target === node && pressedOnBackdrop) node.remove();
    pressedOnBackdrop = false;
  });
  return node;
}

/** 确认框：点确定返回 true；取消 / 点遮罩 / Esc 关闭返回 false */
function confirmDialog(text: string, okLabel = "确定", danger = false) {
  return new Promise<boolean>((resolve) => {
    const node = overlay(`<h2>确认操作</h2><p class="hint">${escapeHtml(text).replace(/\n/g, "<br>")}</p><div class="modal-actions"><button type="button" data-close>取消</button><button class="${danger ? "danger" : "primary"}" id="confirm-ok">${escapeHtml(okLabel)}</button></div>`);
    // 遮罩 / Esc 关闭时节点被移除，监听节点被移除兜底返回 false
    const observer = new MutationObserver(() => { if (!node.isConnected) { observer.disconnect(); resolve(false); } });
    observer.observe(document.body, { childList: true });
    const finish = (value: boolean) => { observer.disconnect(); node.remove(); resolve(value); };
    // 每个按钮直接绑定关闭，不依赖事件委托，保证点击必定生效
    node.querySelectorAll<HTMLButtonElement>("[data-close]").forEach((btn) => { btn.onclick = () => finish(false); });
    node.querySelector<HTMLButtonElement>("#confirm-ok")!.onclick = () => finish(true);
  });
}

/** 提示框：替代原生 alert，点确定 / 点遮罩 / Esc 关闭 */
function alertDialog(text: string) {
  return new Promise<void>((resolve) => {
    const node = overlay(`<h2>提示</h2><p class="hint">${escapeHtml(text).replace(/\n/g, "<br>")}</p><div class="modal-actions"><button class="primary" data-close>知道了</button></div>`);
    const observer = new MutationObserver(() => { if (!node.isConnected) { observer.disconnect(); resolve(); } });
    observer.observe(document.body, { childList: true });
    node.querySelectorAll<HTMLButtonElement>("[data-close]").forEach((btn) => { btn.onclick = () => { observer.disconnect(); node.remove(); resolve(); }; });
  });
}

// Esc 关闭最上层的弹窗；没有弹窗时关闭设置窗口（后端 CloseRequested 拦截转为隐藏，不退出进程）
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  const overlays = document.querySelectorAll<HTMLElement>(".overlay");
  const top = overlays[overlays.length - 1];
  if (top) { top.remove(); return; }
  void getCurrentWindow().close();
});

// ---------- tab 切换 ----------
document.querySelectorAll<HTMLButtonElement>(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b === btn));
    document.querySelectorAll(".tab-page").forEach((p) => p.classList.toggle("active", p.id === `tab-${btn.dataset.tab}`));
  });
});

// ---------- 快捷键录制 ----------
const hotkeyBtn = $<HTMLButtonElement>("#hotkey");
function renderHotkey() { hotkeyBtn.textContent = formatHotkey(config.hotkey) || "点击录制"; }
hotkeyBtn.addEventListener("click", () => {
  recordingHotkey = true;
  hotkeyBtn.textContent = "请按下快捷键…";
});
hotkeyBtn.addEventListener("keydown", (e) => {
  if (!recordingHotkey) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { recordingHotkey = false; renderHotkey(); return; }
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");
  if (e.metaKey) parts.push(isMac ? "Cmd" : "Win");
  let key = e.key;
  if (key === "Control" || key === "Shift" || key === "Alt" || key === "Meta") return; // 等待更多键
  if (key === " ") key = "Space";
  if (key === "`" || key === "~") key = "`";
  if (key.length === 1) key = key.toUpperCase();
  parts.push(key);
  // 至少需要一个修饰键，避免抢占普通按键
  if (parts.length < 2) return;
  config.hotkey = parts.join("+");
  recordingHotkey = false;
  renderHotkey();
  markDirty();
});

$("#hotkey-reset").addEventListener("click", () => { config.hotkey = DEFAULT_HOTKEY; renderHotkey(); markDirty(); });

// ---------- 常规 ----------
function bindCheck(id: string, key: keyof AppConfig) {
  const el = $(`#${id}`) as HTMLInputElement;
  el.checked = Boolean(config[key]);
  el.addEventListener("change", () => { (config as unknown as Record<string, unknown>)[key] = el.checked; markDirty(); });
}

// ---------- 加密分组解锁 ----------
const encryptUnlockSel = $<HTMLSelectElement>("#encrypt-unlock-mode");
encryptUnlockSel.addEventListener("change", () => { config.encryptUnlockMode = encryptUnlockSel.value; markDirty(); });

// ---------- 面板未聚焦时的热键行为 ----------
const hotkeyUnfocusedSel = $<HTMLSelectElement>("#hotkey-unfocused-behavior");
hotkeyUnfocusedSel.addEventListener("change", () => { config.hotkeyUnfocusedBehavior = hotkeyUnfocusedSel.value; markDirty(); });

// ---------- 外观 ----------
const themeSel = $<HTMLSelectElement>("#theme");
const colorInput = $<HTMLInputElement>("#highlight-color");
const followThemeInput = $<HTMLInputElement>("#highlight-follow");
const fontSizeInput = $<HTMLInputElement>("#font-size");
const effectSel = $<HTMLSelectElement>("#window-effect");

themeSel.addEventListener("change", () => { config.theme = themeSel.value; markDirty(); });
// 高亮颜色：空 = 跟随主题强调色（勾选框勾上、取色器禁用但保留上次颜色）
function syncHighlightUI() {
  const follow = followThemeInput.checked;
  colorInput.disabled = follow;
  config.highlightColor = follow ? "" : colorInput.value;
}
followThemeInput.addEventListener("change", () => { syncHighlightUI(); markDirty(); });
colorInput.addEventListener("input", () => { followThemeInput.checked = false; syncHighlightUI(); markDirty(); });
fontSizeInput.addEventListener("change", () => { config.fontSize = Number(fontSizeInput.value) || 14; markDirty(); });
effectSel.addEventListener("change", async () => {
  config.windowEffect = effectSel.value;
  // 实时预览
  try { await invoke("set_window_effect", { effect: config.windowEffect }); } catch { /* 保存时还会再应用 */ }
  await invoke("show_panel");
  markDirty();
});

// ---------- 配置文件目录 ----------
async function refreshConfigDir() { $("#config-dir").textContent = await invoke<string>("get_config_dir"); }

$("#btn-change-config-dir").addEventListener("click", async () => {
  const path = await openDialog({ directory: true, title: "选择配置文件目录" });
  if (!path || typeof path !== "string") return;
  const action = (await confirmDialog("是否将现有配置文件移动到新目录？\n确定=移动文件，取消=仅修改路径", "移动文件")) ? "move" : "keep";
  await invoke("set_data_dirs", { configDir: path, dbDir: null, action });
  await refreshConfigDir();
  await alertDialog("配置目录已更改，重启应用后生效");
});

$("#btn-reset-config-dir").addEventListener("click", async () => {
  if (!await confirmDialog(`确定恢复默认配置目录（${defaultDirLabel}）？`, "恢复默认", true)) return;
  await invoke("set_data_dirs", { configDir: null, dbDir: null, action: "reset" });
  await refreshConfigDir();
  await alertDialog("配置目录已恢复默认，重启应用后生效");
});

// ---------- 数据库 ----------
async function refreshDbDir() { $("#db-dir").textContent = await invoke<string>("get_db_dir"); }

$("#btn-change-db-dir").addEventListener("click", async () => {
  const path = await openDialog({ directory: true, title: "选择数据库目录" });
  if (!path || typeof path !== "string") return;
  const action = (await confirmDialog("是否将现有数据库文件移动到新目录？\n确定=移动文件，取消=仅修改路径", "移动文件")) ? "move" : "keep";
  await invoke("set_data_dirs", { configDir: null, dbDir: path, action });
  await refreshDbDir();
  await refreshDbInfo();
  await alertDialog("数据库目录已更改，重启应用后生效");
});

$("#btn-reset-db-dir").addEventListener("click", async () => {
  if (!await confirmDialog(`确定恢复默认数据库目录（${defaultDirLabel}）？`, "恢复默认", true)) return;
  await invoke("set_data_dirs", { configDir: null, dbDir: null, action: "reset" });
  await refreshDbDir();
  await refreshDbInfo();
  await alertDialog("数据库目录已恢复默认，重启应用后生效");
});

async function refreshDbInfo() {
  const info = await invoke<{ path: string; size: number; itemCount: number; groupCount: number; passwordSet: boolean }>("get_db_info");
  $("#db-path").textContent = info.path;
  $("#db-size").textContent = `${(info.size / 1024).toFixed(1)} KB`;
  $("#db-items").textContent = String(info.itemCount);
  $("#db-groups").textContent = String(info.groupCount);
  $("#db-password").textContent = info.passwordSet ? "是" : "否";
}

// ---------- 面板背景 ----------
const bgEnabledEl = $<HTMLInputElement>("#bg-enabled");
const bgPickBtn = $<HTMLButtonElement>("#bg-pick");
const bgClearBtn = $<HTMLButtonElement>("#bg-clear");
const bgApplySettingsEl = $<HTMLInputElement>("#bg-apply-settings");
const bgEditorEl = $<HTMLDivElement>("#bg-editor");
const bgPreviewLayerEl = $<HTMLDivElement>("#bg-preview-layer");
const bgModeEl = $<HTMLSelectElement>("#bg-mode");
const bgOpacityEl = $<HTMLInputElement>("#bg-opacity");
const bgOpacityValEl = $<HTMLSpanElement>("#bg-opacity-val");
const bgScaleWEl = $<HTMLInputElement>("#bg-scale-w");
const bgScaleWValEl = $<HTMLSpanElement>("#bg-scale-w-val");
const bgScaleHEl = $<HTMLInputElement>("#bg-scale-h");
const bgScaleHValEl = $<HTMLSpanElement>("#bg-scale-h-val");
const bgRegionToggleBtn = $<HTMLButtonElement>("#bg-region-toggle");
const bgRegionResetBtn = $<HTMLButtonElement>("#bg-region-reset");
const bgRegionWrapEl = $<HTMLDivElement>("#bg-region-wrap");
const bgRegionBoxEl = $<HTMLDivElement>("#bg-region-box");
const bgRegionImgEl = $<HTMLImageElement>("#bg-region-img");
const bgRegionRectEl = $<HTMLDivElement>("#bg-region-rect");
const bgApplyBtn = $<HTMLButtonElement>("#bg-apply");

let bgDraft: BackgroundConfig = defaultBackground(); // 编辑中的选项（点「应用背景」才落盘）
let bgSrcImg: HTMLImageElement | null = null; // 源图（原始方向）
let bgPreviewB64: string | null = null; // 预览烘焙缓存（旋转/选区/换图才重烘）

/** 烘焙背景图：旋转 → 按选区裁剪 → 限制最大边长，输出 canvas */
function bakeBackground(maxDim: number): HTMLCanvasElement | null {
  const img = bgSrcImg;
  if (!img) return null;
  const rot = ((bgDraft.rotation % 360) + 360) % 360;
  const swap = rot === 90 || rot === 270;
  const rw = swap ? img.naturalHeight : img.naturalWidth;
  const rh = swap ? img.naturalWidth : img.naturalHeight;
  // 先旋转到正向画布
  const rotC = document.createElement("canvas");
  rotC.width = rw;
  rotC.height = rh;
  const rctx = rotC.getContext("2d")!;
  rctx.translate(rw / 2, rh / 2);
  rctx.rotate((rot * Math.PI) / 180);
  rctx.drawImage(img, -img.naturalWidth / 2, -img.naturalHeight / 2);
  // 选区是归一化坐标，基于旋转后的图
  const rg = bgDraft.region;
  const sx = rg ? rg.x * rw : 0;
  const sy = rg ? rg.y * rh : 0;
  const sw = rg ? rg.w * rw : rw;
  const sh = rg ? rg.h * rh : rh;
  const scale = Math.min(1, maxDim / Math.max(sw, sh));
  const out = document.createElement("canvas");
  out.width = Math.max(1, Math.round(sw * scale));
  out.height = Math.max(1, Math.round(sh * scale));
  out.getContext("2d")!.drawImage(rotC, sx, sy, sw, sh, 0, 0, out.width, out.height);
  return out;
}

function rebakeBgPreview() {
  const c = bakeBackground(1600);
  bgPreviewB64 = c ? c.toDataURL("image/png").split(",")[1] : null;
  applyBgPreviewStyle();
}

function applyBgPreviewStyle() {
  if (bgPreviewB64) styleBgLayer(bgPreviewLayerEl, bgDraft, bgPreviewB64);
  applyBgDraftToSelfWindow();
}

/** 草稿实时应用到设置窗口自身（勾选「同时应用到设置窗口」时整窗可见） */
function applyBgDraftToSelfWindow() {
  applyWindowBackground(bgDraft, bgDraft.applySettings, bgPreviewB64 ?? undefined);
}

/** 选区编辑器里的图片：旋转后的整图（不含选区裁剪） */
function refreshRotatedSource() {
  if (!bgSrcImg) return;
  const keep = bgDraft.region;
  bgDraft.region = null;
  const c = bakeBackground(800);
  bgDraft.region = keep;
  if (c) bgRegionImgEl.src = c.toDataURL("image/png");
}

/** 把 draft.region 画成选区框（百分比定位，随图片缩放自适应） */
function renderRegionRect() {
  const rg = bgDraft.region;
  if (!rg || rg.w <= 0 || rg.h <= 0) {
    bgRegionRectEl.style.display = "none";
    return;
  }
  bgRegionRectEl.style.display = "block";
  bgRegionRectEl.style.left = `${rg.x * 100}%`;
  bgRegionRectEl.style.top = `${rg.y * 100}%`;
  bgRegionRectEl.style.width = `${rg.w * 100}%`;
  bgRegionRectEl.style.height = `${rg.h * 100}%`;
}

/** 控件 ↔ bgDraft 同步（控件侧） */
function syncBgControls() {
  bgEnabledEl.checked = bgDraft.enabled;
  bgApplySettingsEl.checked = bgDraft.applySettings;
  bgModeEl.value = bgDraft.mode === "tile" ? "tile" : "stretch";
  const pct = Math.round(Math.max(0.05, Math.min(1, bgDraft.opacity)) * 100);
  bgOpacityEl.value = String(pct);
  bgOpacityValEl.textContent = `${pct}%`;
  const clampScale = (v: number) => Math.round(Math.max(5, Math.min(300, v || 100)));
  bgScaleWEl.value = String(clampScale(bgDraft.scaleW));
  bgScaleWValEl.textContent = `${bgScaleWEl.value}%`;
  bgScaleHEl.value = String(clampScale(bgDraft.scaleH));
  bgScaleHValEl.textContent = `${bgScaleHEl.value}%`;
  document.querySelectorAll<HTMLButtonElement>(".bg-rot").forEach((b) =>
    b.classList.toggle("active", Number(b.dataset.rot) === bgDraft.rotation)
  );
  bgClearBtn.disabled = !bgDraft.hasImage;
  bgEditorEl.hidden = !bgDraft.hasImage;
  bgRegionResetBtn.disabled = !bgDraft.region;
  renderRegionRect();
}

async function loadBgSource(): Promise<boolean> {
  const data = await invoke<{ b64: string; mime: string } | null>("read_background_source");
  if (!data) {
    bgSrcImg = null;
    return false;
  }
  const img = new Image();
  img.src = `data:${data.mime};base64,${data.b64}`;
  await img.decode();
  bgSrcImg = img;
  return true;
}

bgEnabledEl.addEventListener("change", async () => {
  bgDraft.enabled = bgEnabledEl.checked;
  applyBgDraftToSelfWindow();
  // 启用/关闭即时落盘并广播 config-changed：主面板实时跟随，无需点「应用背景」
  try {
    const bg: BackgroundConfig = { ...(config.background || defaultBackground()), enabled: bgEnabledEl.checked };
    await invoke("set_background_config", { bg });
    config.background = bg;
    await invoke("show_panel");
  } catch (e) {
    await alertDialog(`切换背景开关失败: ${e}`);
  }
});

// 应用范围开关：设置窗口立即整窗预览
bgApplySettingsEl.addEventListener("change", () => {
  bgDraft.applySettings = bgApplySettingsEl.checked;
  applyBgDraftToSelfWindow();
});

bgPickBtn.addEventListener("click", async () => {
  const path = await openDialog({
    title: "选择背景图片",
    filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] }],
  });
  if (typeof path !== "string") return;
  try {
    await invoke("import_background_source", { path });
    await loadBgSource();
    // 换图后回到正向整图，保留填充方式/透明度偏好
    bgDraft = { ...bgDraft, enabled: true, hasImage: true, rotation: 0, region: null };
    refreshRotatedSource();
    syncBgControls();
    rebakeBgPreview();
  } catch (e) {
    await alertDialog(`选择背景图失败: ${e}`);
  }
});

bgClearBtn.addEventListener("click", async () => {
  if (!await confirmDialog("确定清除面板背景图？", "清除", true)) return;
  try {
    await invoke("clear_background_image");
    bgDraft = defaultBackground();
    bgSrcImg = null;
    bgPreviewB64 = null;
    bgPreviewLayerEl.style.backgroundImage = "none";
    bgRegionWrapEl.hidden = true;
    config.background = bgDraft;
    syncBgControls();
    applyBgDraftToSelfWindow();
  } catch (e) {
    await alertDialog(`清除背景失败: ${e}`);
  }
});

bgModeEl.addEventListener("change", () => {
  bgDraft.mode = bgModeEl.value;
  applyBgPreviewStyle();
});

bgOpacityEl.addEventListener("input", () => {
  bgDraft.opacity = Number(bgOpacityEl.value) / 100;
  bgOpacityValEl.textContent = `${bgOpacityEl.value}%`;
  applyBgPreviewStyle();
});

// 宽高百分比：纯 CSS background-size 调整，无需重烘焙
bgScaleWEl.addEventListener("input", () => {
  bgDraft.scaleW = Number(bgScaleWEl.value);
  bgScaleWValEl.textContent = `${bgScaleWEl.value}%`;
  applyBgPreviewStyle();
});
bgScaleHEl.addEventListener("input", () => {
  bgDraft.scaleH = Number(bgScaleHEl.value);
  bgScaleHValEl.textContent = `${bgScaleHEl.value}%`;
  applyBgPreviewStyle();
});

// 旋转：选区是相对旋转后图的，旋转变化后旧选区失效，重置为整图
document.querySelectorAll<HTMLButtonElement>(".bg-rot").forEach((btn) => {
  btn.addEventListener("click", () => {
    const rot = Number(btn.dataset.rot) || 0;
    if (rot === bgDraft.rotation) return;
    bgDraft.rotation = rot;
    bgDraft.region = null;
    refreshRotatedSource();
    syncBgControls();
    rebakeBgPreview();
  });
});

bgRegionToggleBtn.addEventListener("click", () => {
  bgRegionWrapEl.hidden = !bgRegionWrapEl.hidden;
  if (!bgRegionWrapEl.hidden) refreshRotatedSource();
});

bgRegionResetBtn.addEventListener("click", () => {
  bgDraft.region = null;
  syncBgControls();
  rebakeBgPreview();
});

// 选区框选：在（旋转后的）源图上按住拖动出矩形，归一化存储
let bgDragging = false;
let bgDragStart = { x: 0, y: 0 };

function bgRelPos(e: MouseEvent) {
  const r = bgRegionImgEl.getBoundingClientRect();
  return {
    x: Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)),
    y: Math.max(0, Math.min(1, (e.clientY - r.top) / r.height)),
  };
}

bgRegionBoxEl.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || !bgSrcImg) return;
  e.preventDefault();
  bgDragging = true;
  bgDragStart = bgRelPos(e);
  bgDraft.region = { x: bgDragStart.x, y: bgDragStart.y, w: 0, h: 0 };
  renderRegionRect();
});
window.addEventListener("mousemove", (e) => {
  if (!bgDragging) return;
  const p = bgRelPos(e);
  bgDraft.region = {
    x: Math.min(bgDragStart.x, p.x),
    y: Math.min(bgDragStart.y, p.y),
    w: Math.abs(p.x - bgDragStart.x),
    h: Math.abs(p.y - bgDragStart.y),
  };
  renderRegionRect();
});
window.addEventListener("mouseup", () => {
  if (!bgDragging) return;
  bgDragging = false;
  // 太小的框按误触处理：恢复整图
  if (bgDraft.region && (bgDraft.region.w < 0.03 || bgDraft.region.h < 0.03)) {
    bgDraft.region = null;
  }
  syncBgControls();
  rebakeBgPreview();
});

// 应用：烘焙成品图（≤1600px）存到配置目录，设置落盘并弹出主面板看实际效果
bgApplyBtn.addEventListener("click", async () => {
  const c = bakeBackground(1600);
  if (!c) {
    await alertDialog("请先选择图片");
    return;
  }
  bgApplyBtn.disabled = true;
  try {
    const b64 = c.toDataURL("image/png").split(",")[1];
    await invoke("save_background_cache", { b64 });
    const bg: BackgroundConfig = { ...bgDraft, hasImage: true };
    await invoke("set_background_config", { bg });
    config.background = bg;
    bgDraft = { ...bg, region: bg.region ? { ...bg.region } : null };
    syncBgControls();
    await invoke("show_panel");
  } catch (e) {
    await alertDialog(`应用背景失败: ${e}`);
  } finally {
    bgApplyBtn.disabled = false;
  }
});

// 初始化：回填已保存的背景设置；源图还在则重建预览
async function initBackground() {
  const saved = config.background || defaultBackground();
  bgDraft = { ...defaultBackground(), ...saved, region: saved.region ? { ...saved.region } : null };
  if (bgDraft.hasImage) {
    if (await loadBgSource()) {
      refreshRotatedSource();
      rebakeBgPreview();
    } else {
      // 源图丢失（如配置目录被换过）：按无图处理，等用户重新选择
      bgDraft.hasImage = false;
    }
  }
  syncBgControls();
  // 设置窗口自身的背景（按已保存设置的 applySettings 决定）
  applyBgDraftToSelfWindow();
}

// ---------- 应用内更新 ----------
const curVersionEl = $<HTMLElement>("#cur-version");
const latestVersionEl = $<HTMLElement>("#latest-version");
const updateStatusEl = $<HTMLElement>("#update-status");
const updateNotesEl = $<HTMLElement>("#update-notes");
const checkBtn = $<HTMLButtonElement>("#btn-check-update");
const doUpdateBtn = $<HTMLButtonElement>("#btn-do-update");
const doUpdateDirBtn = $<HTMLButtonElement>("#btn-do-update-dir");
const relaunchBtn = $<HTMLButtonElement>("#btn-relaunch");
const progressWrapEl = $<HTMLElement>("#update-progress");
const progressBarEl = $<HTMLElement>("#update-progress-bar");
const autoCheckEl = $<HTMLInputElement>("#auto-check-update");
const installKindEl = $<HTMLElement>("#install-kind");

let pendingUpdate: UpdateInfo | null = null;
// "installed" | "portable"，由 update_install_kind 探测（注册表卸载项）
let installKind = "installed";

function showUpdateResult(info: UpdateInfo | null) {
  doUpdateBtn.hidden = true;
  doUpdateDirBtn.hidden = true;
  relaunchBtn.hidden = true;
  updateNotesEl.hidden = true;
  if (info) {
    latestVersionEl.textContent = `v${info.version}`;
    pendingUpdate = info;
    doUpdateBtn.hidden = false;
    // 便携版额外提供「下载到其他目录」（得到另一个 exe，配置独立）
    doUpdateDirBtn.hidden = installKind !== "portable";
    if (info.notes.trim()) {
      updateNotesEl.hidden = false;
      updateNotesEl.textContent = info.notes;
    }
  } else {
    latestVersionEl.textContent = "已是最新";
    pendingUpdate = null;
  }
}

async function runCheck(manual: boolean) {
  checkBtn.disabled = true;
  updateStatusEl.textContent = "正在检查…";
  if (manual) resetUpdateCache();
  try {
    const info = await checkUpdate();
    showUpdateResult(info);
    updateStatusEl.textContent = info ? `发现新版本，可下载 v${info.version}` : "已是最新版本";
  } catch (e) {
    updateStatusEl.textContent = `检查更新失败：${e}`;
  } finally {
    checkBtn.disabled = false;
  }
}

checkBtn.addEventListener("click", () => runCheck(true));

// 更新下载进度条：0-100%，未知总大小时显示不确定状态
function setProgress(sent: number, total: number) {
  progressWrapEl.hidden = false;
  if (total > 0) {
    progressBarEl.style.width = `${Math.round((sent / total) * 100)}%`;
  } else {
    progressBarEl.style.width = "30%";
  }
}

function setBusy(on: boolean) {
  checkBtn.disabled = on;
  doUpdateBtn.disabled = on;
  doUpdateDirBtn.disabled = on;
}

// 从 Release 的 sha256sums.txt 中取指定文件的哈希
async function fetchSha256(url: string, fileName: string): Promise<string | null> {
  try {
    const text = await invoke<string>("http_get_text", { url });
    for (const line of text.split(/\r?\n/)) {
      const parts = line.trim().split(/\s+/);
      if (parts.length >= 2 && parts[1].trim() === fileName) return parts[0].trim();
    }
  } catch { /* 无校验文件 */ }
  return null;
}

// 便携版：下载便携 exe → 校验 → （就地）替换重启 /（其他目录）下载完在资源管理器中显示
async function portableUpdate(toOtherDir: boolean) {
  if (!pendingUpdate) return;
  setBusy(true);
  try {
    const dir = toOtherDir
      ? await openDialog({ directory: true, title: "选择存放新版 速启.exe 的目录" })
      : null;
    if (toOtherDir && !dir) return;
    const path = await invoke<string>("portable_update_begin", {
      targetDir: (dir as string) ?? null,
    });
    const url = `https://github.com/duangdangding/ls_quick_start/releases/download/v${pendingUpdate.version}/suqi_v${pendingUpdate.version}_x64_portable.exe`;
    updateStatusEl.textContent = "正在下载更新…";
    const unlisten = await listen<{ sent: number; total: number }>(
      "portable-update-progress",
      (e) => setProgress(e.payload.sent, e.payload.total),
    );
    try {
      await invoke("portable_update_download", { url, path });
    } finally {
      unlisten();
    }

    // SHA-256 校验：防止下载损坏或被篡改
    updateStatusEl.textContent = "正在校验文件…";
    const sha256Url = url.substring(0, url.lastIndexOf("/") + 1) + "sha256sums.txt";
    const fileName = url.substring(url.lastIndexOf("/") + 1);
    const expected = await fetchSha256(sha256Url, fileName);
    if (expected) {
      const ok = await invoke<boolean>("portable_update_verify", { path, expectedSha256: expected });
      if (!ok) { updateStatusEl.textContent = "SHA-256 校验不一致，文件可能损坏，请重试"; return; }
    }

    if (toOtherDir) {
      progressWrapEl.hidden = true;
      updateStatusEl.textContent = `已下载到所选目录，数据与原目录相互独立`;
      await invoke("reveal_path", { path });
      return;
    }

    // 就地更新：退出本程序，由辅助进程等待退出后替换 exe 并重启新版
    updateStatusEl.textContent = "下载完成，正在替换并重启…";
    await invoke("portable_update_apply", { newPath: path });
  } catch (e) {
    updateStatusEl.textContent = `更新失败：${e}`;
  } finally {
    setBusy(false);
  }
}

// Windows 安装版：下载 NSIS 安装包到系统「下载」目录 → 校验 → 用户确认后运行安装程序
async function installedUpdate() {
  if (!pendingUpdate) return;
  setBusy(true);
  try {
    const fileName = `suqi_${pendingUpdate.version}_x64-setup.exe`;
    const path = await invoke<string>("installer_update_path", { filename: fileName });
    const url = `https://github.com/duangdangding/ls_quick_start/releases/download/v${pendingUpdate.version}/${fileName}`;
    updateStatusEl.textContent = "正在下载安装包…";
    const unlisten = await listen<{ sent: number; total: number }>(
      "portable-update-progress",
      (e) => setProgress(e.payload.sent, e.payload.total),
    );
    try {
      await invoke("portable_update_download", { url, path });
    } finally {
      unlisten();
    }

    updateStatusEl.textContent = "正在校验文件…";
    const sha256Url = url.substring(0, url.lastIndexOf("/") + 1) + "sha256sums.txt";
    const expected = await fetchSha256(sha256Url, fileName);
    if (expected) {
      const ok = await invoke<boolean>("portable_update_verify", { path, expectedSha256: expected });
      if (!ok) { updateStatusEl.textContent = "SHA-256 校验不一致，文件可能损坏，请重试"; return; }
    }

    if (await confirmDialog("安装包已下载完成。\n\n点击「退出并安装」将退出速启并运行安装程序；\n点击「取消」稍后可到下载文件夹手动运行。", "退出并安装")) {
      await invoke("installer_update_run", { path });
    } else {
      updateStatusEl.textContent = `安装包已下载到「下载」文件夹：${fileName}，可稍后手动运行`;
      await invoke("reveal_path", { path });
    }
  } catch (e) {
    updateStatusEl.textContent = `更新失败：${e}`;
  } finally {
    setBusy(false);
  }
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

// 把插件的英文报错转成可读中文，并给出对应处理办法
function macUpdateErrorText(e: unknown): string {
  const msg = String(e);
  if (/Failed to move the new app into place/i.test(msg)) {
    return "安装失败：没有权限替换「应用程序」里的速启。若刚才弹出了管理员授权窗口，请点击「好」并输入密码；也可以先把速启拖到「应用程序」目录（不要直接在 DMG 安装盘或下载目录里运行）再重试。";
  }
  if (/signature|minisign|verify/i.test(msg)) {
    return `安装包校验失败：${msg}。请稍后重试；若持续失败，请到 Releases 页面手动下载安装。`;
  }
  if (/Download request failed with status/i.test(msg)) {
    return `下载失败：${msg}。多为网络问题，请检查网络或代理后重试。`;
  }
  if (/error sending request|timed out|connection|dns|Network/i.test(msg)) {
    return `网络错误：${msg}。GitHub 下载可能被中断，请检查网络或代理后重试。`;
  }
  return `更新失败：${msg}`;
}

// macOS 更新：下载 signed bundle → 校验 → 就地替换 .app
// 拆成 download + install 两段：下载阶段显示真实进度，校验/安装阶段插件不再发进度事件，改为明确文案
async function macUpdate() {
  if (!pendingUpdate) return;
  const update = pendingUpdate.update;
  if (!update) return;
  setBusy(true);
  progressWrapEl.hidden = false;
  progressBarEl.style.width = "0%";
  updateStatusEl.textContent = "正在连接更新服务器…";

  let total = 0;
  let received = 0;
  let lastPaint = 0;
  let stallTimer: number | undefined;
  // 长时间没有新数据时补一句说明，避免界面看起来像卡死
  const armStallTimer = () => {
    window.clearTimeout(stallTimer);
    stallTimer = window.setTimeout(() => {
      updateStatusEl.textContent = "网络响应较慢，仍在等待下载…如长时间无进展，请检查网络或代理后重试";
    }, 30000);
  };
  // 节流写 DOM：下载分片很密，每个分片都重排会让设置窗口发卡
  const paint = (text: string, force = false) => {
    const now = performance.now();
    if (!force && now - lastPaint < 100) return;
    lastPaint = now;
    if (total > 0) {
      progressBarEl.style.width = `${Math.min(100, Math.round((received / total) * 100))}%`;
      updateStatusEl.textContent = `${text} ${formatBytes(received)} / ${formatBytes(total)}`;
    } else {
      updateStatusEl.textContent = `${text} 已下载 ${formatBytes(received)}`;
    }
  };

  try {
    armStallTimer();
    // 第二参数给个足够宽松的超时：插件默认不限时，网络中断时可能一直挂着，超时后报错可重试
    await update.download((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
        paint("正在下载更新", true);
      } else if (event.event === "Progress") {
        received += event.data.chunkLength;
        armStallTimer();
        paint("正在下载更新");
      } else if (event.event === "Finished") {
        window.clearTimeout(stallTimer);
        progressBarEl.style.width = "100%";
        paint("正在下载更新", true);
      }
    }, { timeout: 10 * 60 * 1000 });
    // 之后插件要校验签名、解包并替换 .app，可能弹管理员授权窗口，这里必须给出提示
    updateStatusEl.textContent = "正在校验并安装更新，如弹出授权窗口请点击「好」并输入密码…";
    await update.install();
    window.clearTimeout(stallTimer);
    progressBarEl.style.width = "100%";
    updateStatusEl.textContent = "更新已安装，重启应用完成更新";
    relaunchBtn.hidden = false;
  } catch (e) {
    window.clearTimeout(stallTimer);
    progressWrapEl.hidden = true;
    updateStatusEl.textContent = macUpdateErrorText(e);
  } finally {
    setBusy(false);
  }
}

doUpdateBtn.addEventListener("click", async () => {
  if (!pendingUpdate) return;
  if (installKind === "portable") return portableUpdate(false);
  // macOS 下载依赖插件构造的 update 对象；兜底场景对象为空，引导手动下载
  if (isMac) {
    if (!pendingUpdate.update) { updateStatusEl.textContent = "自动下载通道暂不可用，请到 GitHub Releases 页面手动下载新版本"; return; }
    return macUpdate();
  }
  await installedUpdate();
});

doUpdateDirBtn.addEventListener("click", () => portableUpdate(true));

relaunchBtn.addEventListener("click", () => relaunch());

autoCheckEl.checked = autoCheckEnabled();
autoCheckEl.addEventListener("change", () => setAutoCheckEnabled(autoCheckEl.checked));

async function initAboutTab() {
  curVersionEl.textContent = `v${await getVersion()}`;
  // 安装方式提示：便携版说明就地替换逻辑（配置/记录保留在同目录）
  try {
    installKind = await invoke<string>("update_install_kind");
    if (installKind === "portable") {
      installKindEl.hidden = false;
      installKindEl.textContent =
        "当前为绿色便携版：更新会下载到本程序所在目录，替换旧文件后自动重启，配置和启动项都保留在原目录。";
    } else if (!isMac) {
      installKindEl.hidden = false;
      installKindEl.textContent =
        "当前为安装版：更新会下载安装包并运行安装程序，装回原目录，桌面快捷方式不变。";
    }
  } catch { /* 非 Windows 平台忽略 */ }
  // 自动检查（若主面板已查过会直接命中缓存，不重复请求）
  if (latestVersionEl.textContent === "尚未检查") runCheck(false);
}
listen("open-update-tab", () => {
  document.querySelector<HTMLButtonElement>('.tab[data-tab="about"]')?.click();
});

// ---------- 保存 ----------
$("#btn-save").addEventListener("click", async () => {
  try {
    // 若选了背景图但没点「应用背景」，保存时一并烘焙落盘，避免“保存后主面板无效果”
    if (bgDraft.hasImage && bgSrcImg) {
      const c = bakeBackground(1600);
      if (c) {
        const b64 = c.toDataURL("image/png").split(",")[1];
        await invoke("save_background_cache", { b64 });
        const bg: BackgroundConfig = { ...bgDraft, hasImage: true };
        await invoke("set_background_config", { bg });
        config.background = bg;
        bgDraft = { ...bg, region: bg.region ? { ...bg.region } : null };
      }
    }
    await saveConfig(config);
    $("#save-msg").textContent = "已保存";
    setTimeout(() => { $("#save-msg").textContent = ""; }, 2000);
    refreshConfigDir();
  } catch (e) {
    await alertDialog(String(e));
  }
});

// ---------- 初始化 ----------
(async () => {
  // 平台相关静态文案（Windows / macOS 展示不同示例与目录说明）
  $("#hotkey-hint").textContent = `按下组合键后松开即完成录制（例如 ${formatHotkey(DEFAULT_HOTKEY)}）`;
  $("#config-dir-hint").textContent = `默认在${defaultDirLabel}下，可自定义到其他位置`;
  $("#db-dir-hint").textContent = `数据库文件默认在${defaultDirLabel}下，可自定义到其他位置`;
  $("#about-desc").textContent = isMac ? "macOS 快捷启动器" : "Windows 快捷启动器";
  // macOS 无亚克力/云母材质，仅保留「默认」与「毛玻璃」
  if (isMac) document.querySelectorAll("#window-effect [data-win-only]").forEach((option) => option.remove());

  config = await loadConfig();

  bindCheck("autostart", "autostart");
  bindCheck("silent-start", "silentStart");
  bindCheck("always-on-top", "alwaysOnTop");
  bindCheck("remember-size", "rememberSize");
  bindCheck("run-target-enabled", "runTargetEnabled");
  bindCheck("clear-search-on-reopen", "clearSearchOnReopen");

  encryptUnlockSel.value = config.encryptUnlockMode === "always" ? "always" : "session";
  hotkeyUnfocusedSel.value = config.hotkeyUnfocusedBehavior === "hide" ? "hide" : "front";

  themeSel.value = config.theme;
  // 搜索高亮颜色：空 = 跟随主题强调色（勾选框勾上、取色器禁用但保留上次颜色）
  const hl = (config.highlightColor || "").trim();
  followThemeInput.checked = !hl;
  colorInput.value = /^#[0-9a-fA-F]{6}$/.test(hl) ? hl : "#7c3aed";
  colorInput.disabled = !hl;
  fontSizeInput.value = String(config.fontSize);
  effectSel.value = config.windowEffect;

  renderHotkey();
  refreshConfigDir();
  refreshDbDir();
  refreshDbInfo();
  applyAppearance(config);
  applyWindowEffect(config.windowEffect);
  initBackground().catch(() => { /* 背景源图读取失败不影响设置页其余功能 */ });

  // ---------- 关于 ----------
  initAboutTab();
})();
