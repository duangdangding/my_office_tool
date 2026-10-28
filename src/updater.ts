import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { isDesktop } from "./api";

export type InstallKind = "installed" | "portable" | "macos";

export interface UpdateProgress {
  downloaded: number;
  total: number;
  finished: boolean;
}

export async function getAppVersion(): Promise<string> {
  return isDesktop ? getVersion() : "0.1.0";
}

export async function getInstallKind(): Promise<InstallKind> {
  return isDesktop ? invoke<InstallKind>("update_install_kind") : "portable";
}

export async function checkAppUpdate(): Promise<Update | null> {
  // 浏览器预览不访问网络，也不模拟一个并不存在的更新。
  return isDesktop ? check({ timeout: 30_000 }) : null;
}

export async function installAppUpdate(update: Update, onProgress: (progress: UpdateProgress) => void): Promise<void> {
  let downloaded = 0;
  let total = 0;
  const handleEvent = (event: DownloadEvent) => {
    if (event.event === "Started") {
      downloaded = 0;
      total = event.data.contentLength ?? 0;
    } else if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
    }
    onProgress({ downloaded, total, finished: event.event === "Finished" });
  };
  await update.downloadAndInstall(handleEvent, { timeout: 120_000 });
  // Windows 安装程序接管后会退出当前进程；macOS 安装完成后在这里重启。
  await relaunch();
}

export async function openReleasePage(): Promise<void> {
  if (isDesktop) {
    await invoke("open_release_page");
  } else {
    window.open("https://github.com/duangdangding/my_office_tool/releases/latest", "_blank", "noopener,noreferrer");
  }
}
