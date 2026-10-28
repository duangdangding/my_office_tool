import { check, Update } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";

// ---------- 应用内更新：主面板与设置页共用的检查逻辑 ----------
// 结果缓存一次：主面板自动检查过后，设置页直接复用，避免重复请求 GitHub
export interface UpdateInfo {
  version: string;
  notes: string;
  /** 插件构造的下载对象；仅插件通道成功时存在，兜底场景为 null（macOS 此时不可自动下载） */
  update: Update | null;
}

let cached: UpdateInfo | null | undefined; // undefined = 还没检查过

const AUTO_CHECK_KEY = "suqi:auto-check-update";

export function autoCheckEnabled(): boolean {
  return localStorage.getItem(AUTO_CHECK_KEY) !== "0";
}

export function setAutoCheckEnabled(on: boolean) {
  localStorage.setItem(AUTO_CHECK_KEY, on ? "1" : "0");
}

export async function checkUpdate(): Promise<UpdateInfo | null> {
  if (cached !== undefined) return cached;
  try {
    const update = await check();
    cached = update
      ? { version: update.version, notes: update.body ?? "", update }
      : await checkNewerRelease();
  } catch {
    // 断网、无 Release、未配置签名等场景：再尝试按已发布 Release 判断，失败才按「无更新」处理
    cached = await checkNewerRelease();
  }
  return cached;
}

// 手动点击「检查更新」时清掉缓存强制重新查
export function resetUpdateCache() {
  cached = undefined;
}

// ---------- 兜底：插件查不到更新时，比对 GitHub 上已发布的 Release ----------
// 发版后 CI 构建需要 5-10 分钟，期间「最新 Release」可能未及时切换或插件请求失败；
// 按「最新 tag」判断会把正在构建、尚不可下载的版本当成更新，所以只对已发布的 Release 取版本号。
const RELEASES_URL = "https://api.github.com/repos/duangdangding/ls_quick_start/releases?per_page=30";

function parseTag(tag: string): [number, number, number] | null {
  const m = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

function compareVer(a: [number, number, number], b: [number, number, number]): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

async function checkNewerRelease(): Promise<UpdateInfo | null> {
  try {
    const current = parseTag(`v${await getVersion()}`);
    if (!current) return null;
    const text = await invoke<string>("http_get_text", { url: RELEASES_URL });
    // 公开列表接口不含草稿 Release，取 tag_name 中版本号最高的一个
    const names = (JSON.parse(text) as { tag_name?: string }[]).map((r) => r.tag_name ?? "");
    let newest: [number, number, number] | null = null;
    for (const name of names) {
      const v = parseTag(name);
      if (v && (!newest || compareVer(v, newest) > 0)) newest = v;
    }
    if (!newest || compareVer(newest, current) <= 0) return null;
    return { version: newest.join("."), notes: "", update: null };
  } catch {
    return null;
  }
}
