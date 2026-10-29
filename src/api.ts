import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import suqiManifest from "../plugins/suqi/office-plugin.json";
import lscopyManifest from "../plugins/lscopy/office-plugin.json";
import type { AppSettings, AppTheme, ExtractPluginResult, PluginDirectoryView, PluginImportOutcome, PluginManifest, PluginRules, PluginView } from "./types";

export const isDesktop = "__TAURI_INTERNALS__" in window;

const defaultRules = (): PluginRules => ({
  enabled: true,
  autoStart: false,
  detach: false,
  allowNetwork: true,
  allowClipboard: true,
  allowFileSystem: true,
  allowGlobalShortcut: true,
  allowPluginAutostart: false,
});

const mockPlugins: PluginView[] = [suqiManifest, lscopyManifest].map((manifest, index) => ({
  manifest: manifest as PluginManifest,
  root: `F:\\workspace\\my\\vscode\\my_office_tool\\plugins\\${manifest.id}`,
  source: "builtin",
  origin: null,
  rules: defaultRules(),
  runtime: { status: index === 0 ? "running" : "stopped", pid: index === 0 ? 28416 : null, detail: null },
}));

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export async function listPlugins(): Promise<PluginView[]> {
  return isDesktop ? invoke<PluginView[]>("list_plugins") : clone(mockPlugins);
}

export async function getPluginDirectory(): Promise<PluginDirectoryView> {
  if (isDesktop) return invoke<PluginDirectoryView>("get_plugin_directory");
  return { path: "F:\\workspace\\my\\vscode\\my_office_tool\\plugins", isDefault: false };
}

const browserSettings: AppSettings = {
  launchAtStartup: false,
  silentStart: false,
  theme: "light",
  pluginDir: { path: "F:\\workspace\\my\\vscode\\my_office_tool\\plugins", isDefault: false },
  proxy: "",
};

export async function getSettings(): Promise<AppSettings> {
  return isDesktop ? invoke<AppSettings>("get_settings") : clone(browserSettings);
}

export async function setLaunchAtStartup(enabled: boolean): Promise<AppSettings> {
  if (isDesktop) return invoke<AppSettings>("set_launch_at_startup", { enabled });
  browserSettings.launchAtStartup = enabled;
  return clone(browserSettings);
}

export async function setSilentStart(enabled: boolean): Promise<AppSettings> {
  if (isDesktop) return invoke<AppSettings>("set_silent_start", { enabled });
  browserSettings.silentStart = enabled;
  return clone(browserSettings);
}

export async function setTheme(theme: AppTheme): Promise<AppSettings> {
  if (isDesktop) return invoke<AppSettings>("set_theme", { theme });
  browserSettings.theme = theme;
  return clone(browserSettings);
}

export async function setProxy(value: string): Promise<AppSettings> {
  if (isDesktop) return invoke<AppSettings>("set_proxy", { value });
  browserSettings.proxy = value.trim();
  return clone(browserSettings);
}

export async function choosePluginStorageDirectory(): Promise<string | null> {
  if (!isDesktop) return null;
  const selected = await open({ directory: true, multiple: false, title: "选择插件存放目录" });
  return typeof selected === "string" ? selected : null;
}

export async function setPluginDirectory(path: string): Promise<PluginDirectoryView> {
  if (!isDesktop) throw new Error("浏览器预览不修改插件目录，请在桌面应用中使用此功能。");
  return invoke<PluginDirectoryView>("set_plugin_directory", { path });
}

export async function choosePluginDirectory(): Promise<string | null> {
  if (!isDesktop) return null;
  const selected = await open({ directory: true, multiple: false, title: "选择插件目录" });
  return typeof selected === "string" ? selected : null;
}

export async function chooseExtractDirectory(): Promise<string | null> {
  if (!isDesktop) return null;
  const selected = await open({ directory: true, multiple: false, title: "选择编译产物提取位置" });
  return typeof selected === "string" ? selected : null;
}

export async function importPlugin(path: string): Promise<PluginImportOutcome> {
  if (!isDesktop) throw new Error("浏览器预览不执行本地导入，请在桌面应用中使用此功能。");
  return invoke<PluginImportOutcome>("import_plugin", { path });
}

export async function importGitPlugin(url: string): Promise<PluginImportOutcome> {
  if (!isDesktop) throw new Error("浏览器预览不执行 Git 导入，请在桌面应用中使用此功能。");
  return invoke<PluginImportOutcome>("import_git_plugin", { url });
}

export async function importArchivePlugin(url: string): Promise<PluginImportOutcome> {
  if (!isDesktop) throw new Error("浏览器预览不执行压缩包导入，请在桌面应用中使用此功能。");
  return invoke<PluginImportOutcome>("import_archive_plugin", { url });
}

export async function completePluginImport(sessionId: string, manifest: PluginManifest): Promise<PluginView> {
  if (!isDesktop) throw new Error("浏览器预览不执行本地导入，请在桌面应用中使用此功能。");
  return invoke<PluginView>("complete_plugin_import", { sessionId, manifest });
}

export async function cancelPluginImport(sessionId: string): Promise<void> {
  if (!isDesktop) return;
  await invoke("cancel_plugin_import", { sessionId });
}

export async function updatePlugin(id: string): Promise<PluginView> {
  if (isDesktop) return invoke<PluginView>("update_plugin", { id });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (!plugin) throw new Error("插件不存在");
  return clone(plugin);
}

export async function saveRules(id: string, rules: PluginRules): Promise<PluginView> {
  if (isDesktop) return invoke<PluginView>("update_plugin_rules", { id, rules });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (!plugin) throw new Error("插件不存在");
  plugin.rules = clone(rules);
  if (!rules.enabled) plugin.runtime = { status: "stopped", pid: null, detail: null };
  return clone(plugin);
}

export async function launchPlugin(id: string, detached?: boolean): Promise<PluginView["runtime"]> {
  if (isDesktop) return invoke<PluginView["runtime"]>("launch_plugin", { id, detached });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (!plugin) throw new Error("插件不存在");
  if (!plugin.rules.enabled) throw new Error("插件已停用，请先在规则中启用。");
  await new Promise((resolve) => window.setTimeout(resolve, 700));
  plugin.runtime = { status: detached || plugin.rules.detach ? "detached" : "running", pid: 30000 + Math.floor(Math.random() * 1000), detail: null };
  return clone(plugin.runtime);
}

export async function stopPlugin(id: string): Promise<void> {
  if (isDesktop) return invoke("stop_plugin", { id });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (plugin) plugin.runtime = { status: "stopped", pid: null, detail: null };
}

export async function removePlugin(id: string, deleteSource = false): Promise<void> {
  if (isDesktop) return invoke("remove_plugin", { id, deleteSource });
  const index = mockPlugins.findIndex((item) => item.manifest.id === id);
  if (index >= 0 && mockPlugins[index].source === "imported") mockPlugins.splice(index, 1);
}

export async function extractPlugin(id: string, destination: string): Promise<ExtractPluginResult> {
  if (!isDesktop) throw new Error("浏览器预览不提取本地文件，请在桌面应用中使用此功能。");
  return invoke<ExtractPluginResult>("extract_plugin", { id, destination });
}

export async function openPluginDirectory(id: string): Promise<void> {
  if (isDesktop) return invoke("open_plugin_directory", { id });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (plugin) await navigator.clipboard?.writeText(plugin.root);
}

export async function readPluginLog(id: string): Promise<string> {
  if (isDesktop) return invoke<string>("read_plugin_log", { id });
  return `[预览模式] ${id} 的运行日志将在桌面应用中显示。\n插件以独立进程运行，标准输出与错误输出会保存在本机。`;
}

export async function readPluginReadme(id: string): Promise<string> {
  if (isDesktop) return invoke<string>("read_plugin_readme", { id });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  if (!plugin) throw new Error("插件不存在。");
  return `# ${plugin.manifest.name}\n\n${plugin.manifest.description}\n\n预览模式仅展示插件摘要，桌面应用会读取插件目录中的 README.md。`;
}

export async function openPluginPage(id: string): Promise<void> {
  if (isDesktop) return invoke("open_plugin_page", { id });
  const plugin = mockPlugins.find((item) => item.manifest.id === id);
  const url = plugin?.manifest.entry.web?.url;
  if (!url) throw new Error("此插件没有配置 Web 页面地址。");
  window.open(url, "_blank", "noopener,noreferrer");
}
