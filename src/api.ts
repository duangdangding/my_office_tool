import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import suqiManifest from "../plugins/suqi/office-plugin.json";
import lscopyManifest from "../plugins/lscopy/office-plugin.json";
import type { PluginManifest, PluginRules, PluginView } from "./types";

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
  rules: defaultRules(),
  runtime: { status: index === 0 ? "running" : "stopped", pid: index === 0 ? 28416 : null, detail: null },
}));

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export async function listPlugins(): Promise<PluginView[]> {
  return isDesktop ? invoke<PluginView[]>("list_plugins") : clone(mockPlugins);
}

export async function choosePluginDirectory(): Promise<string | null> {
  if (!isDesktop) return null;
  const selected = await open({ directory: true, multiple: false, title: "选择包含 office-plugin.json 的插件目录" });
  return typeof selected === "string" ? selected : null;
}

export async function importPlugin(path: string): Promise<PluginView> {
  if (!isDesktop) throw new Error("浏览器预览不执行本地导入，请在桌面应用中使用此功能。");
  return invoke<PluginView>("import_plugin", { path });
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

export async function removePlugin(id: string): Promise<void> {
  if (isDesktop) return invoke("remove_plugin", { id });
  const index = mockPlugins.findIndex((item) => item.manifest.id === id);
  if (index >= 0 && mockPlugins[index].source === "imported") mockPlugins.splice(index, 1);
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
