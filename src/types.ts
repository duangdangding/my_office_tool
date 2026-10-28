export type PluginSource = "builtin" | "imported";
export type PluginStatus = "stopped" | "starting" | "running" | "detached" | "failed";

export interface PluginRules {
  enabled: boolean;
  autoStart: boolean;
  detach: boolean;
  allowNetwork: boolean;
  allowClipboard: boolean;
  allowFileSystem: boolean;
  allowGlobalShortcut: boolean;
  allowPluginAutostart: boolean;
}

export interface PluginManifest {
  schemaVersion: number;
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  category: string;
  icon: string;
  permissions: string[];
  entry: {
    mode: "auto" | "executable" | "dev";
    executable: string[];
    dev?: { program: string; args: string[]; cwd: string };
  };
  standalone: { instructions: string };
}

export interface PluginView {
  manifest: PluginManifest;
  root: string;
  source: PluginSource;
  rules: PluginRules;
  runtime: {
    status: PluginStatus;
    pid: number | null;
    detail: string | null;
  };
}
