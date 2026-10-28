import { invoke } from "@tauri-apps/api/core";

/** 背景图选区：归一化坐标（0-1），相对于旋转后的源图 */
export interface BgRegion { x: number; y: number; w: number; h: number }

/** 面板背景图设置（与后端 BackgroundConfig 对应） */
export interface BackgroundConfig {
  enabled: boolean; // 是否启用面板背景图
  mode: string; // "stretch" 拉伸铺满 | "tile" 原尺寸平铺
  opacity: number; // 背景图不透明度 0.05 - 1.0
  scaleW: number; // 图片宽度占面板百分比 5 - 300
  scaleH: number; // 图片高度占面板百分比 5 - 300
  applySettings: boolean; // 背景同时应用到设置窗口
  rotation: number; // 旋转角度（0/90/180/270，已烘焙进缓存图）
  region: BgRegion | null; // 归一化选区（已烘焙进缓存图），null = 整图
  hasImage: boolean; // 是否已选择图片
}

export interface AppConfig {
  hotkey: string;
  autostart: boolean;
  silentStart: boolean;
  alwaysOnTop: boolean;
  rememberSize: boolean;
  windowWidth: number;
  windowHeight: number;
  theme: string; // "dark" | "light"
  fontFamily: string;
  fontSize: number;
  highlightColor: string; // #rrggbb；空 = 跟随主题强调色
  windowEffect: string; // "default" | "acrylic" | "vibrancy" | "mica"
  viewMode: string; // 主面板显示方式："card" 卡片 | "list" 列表
  encryptUnlockMode: string; // 加密解锁策略："always" 每次打开面板都需密码 | "session" 仅重启应用才需密码
  hotkeyUnfocusedBehavior: string; // 面板已显示但未聚焦时按快捷键："front" 置前显示 | "hide" 隐藏面板
  runTargetEnabled: boolean; // 搜索框「直接运行」（仿 Win+R）：默认开启；关闭后仅搜索启动项
  clearSearchOnReopen: boolean; // 重新打开面板时清空搜索框：默认开启，回到全部条目重新开始
  configDir: string | null;
  dbDir: string | null;
  background: BackgroundConfig;
}

export const isMac = navigator.platform.toUpperCase().includes("MAC");
/** 默认全局快捷键：与后端 DEFAULT_HOTKEY 保持一致（macOS 上 Cmd+` 与系统「切换窗口」冲突，改用 Alt+Space） */
export const DEFAULT_HOTKEY = isMac ? "Alt+Space" : "Win+`";

/** 背景设置的默认值（与后端 BackgroundConfig::default 一致） */
export function defaultBackground(): BackgroundConfig {
  return {
    enabled: false,
    mode: "stretch",
    opacity: 0.6,
    scaleW: 100,
    scaleH: 100,
    applySettings: false,
    rotation: 0,
    region: null,
    hasImage: false,
  };
}

export async function loadConfig(): Promise<AppConfig> {
  // 页面加载可能早于 Rust 端 .manage() 完成，短间隔重试等后端就绪
  let lastErr: unknown;
  for (let i = 0; i < 20; i++) {
    try {
      return await invoke<AppConfig>("get_config");
    } catch (e) {
      lastErr = e;
      await new Promise((r) => setTimeout(r, 250));
    }
  }
  throw lastErr;
}

export async function saveConfig(config: AppConfig): Promise<void> {
  await invoke("save_config", { config });
}

/** 把配置里的快捷键格式化成展示文本（mac 用符号） */
export function formatHotkey(hotkey: string): string {
  if (!isMac) return hotkey;
  return hotkey
    .replace(/Ctrl\+/gi, "\u2303")
    .replace(/Shift\+/gi, "\u21E7")
    .replace(/Alt\+/gi, "\u2325")
    .replace(/Win\+|Super\+|Meta\+|Cmd\+/gi, "\u2318");
}

/** 应用外观（主题、高亮色、字号、字体）到文档根 */
export function applyAppearance(cfg: AppConfig): void {
  const root = document.documentElement;
  root.dataset.theme = cfg.theme === "light" ? "light" : "dark";
  root.style.setProperty("--app-font-size", `${cfg.fontSize || 14}px`);
  root.style.fontFamily = cfg.fontFamily || "system-ui";
  // 搜索命中高亮颜色：仅接受 #rrggbb；空/非法 = 跟随主题强调色
  const hl = (cfg.highlightColor || "").trim();
  if (/^#[0-9a-fA-F]{6}$/.test(hl)) {
    root.style.setProperty("--hl-color", hl);
  } else {
    root.style.setProperty("--hl-color", "var(--blue)");
  }
}

/**
 * 窗口材质效果（亚克力/云母/毛玻璃）：把效果名写到 <html data-effect>，
 * 面板背景由 styles.css 里对应的半透明规则接管，透出系统材质。
 */
export function applyWindowEffect(effect: string) {
  document.documentElement.dataset.effect = effect || "default";
}

/**
 * 把背景图按设置应用到一个背景层元素上（主面板与设置页预览共用）。
 * b64 为烘焙后的 PNG base64；mode 决定拉伸（不重复）还是平铺（重复）。
 */
export function styleBgLayer(el: HTMLElement, bg: BackgroundConfig, b64: string) {
  el.style.backgroundImage = `url("data:image/png;base64,${b64}")`;
  el.style.opacity = String(Math.max(0.05, Math.min(1, bg.opacity)));
  const w = Math.max(5, Math.min(300, bg.scaleW || 100));
  const h = Math.max(5, Math.min(300, bg.scaleH || 100));
  el.style.backgroundSize = `${w}% ${h}%`;
  el.style.backgroundPosition = "center";
  el.style.backgroundRepeat = bg.mode === "tile" ? "repeat" : "no-repeat";
}

let bgLayerEl: HTMLElement | null = null;

/**
 * 应用面板背景：按需创建 .bg-layer 固定层，读取后端烘焙缓存并设置样式。
 * include=false 表示当前窗口不应用背景（如设置页未勾选“应用到设置窗口”）。
 */
export async function applyWindowBackground(
  bg: BackgroundConfig | undefined,
  include: boolean,
  b64Override?: string
) {
  const on = !!bg && bg.enabled && bg.hasImage && include;
  document.documentElement.dataset.bg = on ? "on" : "off";
  if (!bgLayerEl) {
    bgLayerEl = document.createElement("div");
    bgLayerEl.className = "bg-layer";
    document.body.prepend(bgLayerEl);
  }
  if (!on) {
    bgLayerEl.style.backgroundImage = "none";
    return;
  }
  const b64 = b64Override ?? (await invoke<string | null>("get_background_cache"));
  if (!b64) {
    document.documentElement.dataset.bg = "off";
    bgLayerEl.style.backgroundImage = "none";
    return;
  }
  styleBgLayer(bgLayerEl, bg!, b64);
}
