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

export type PluginExecutable = string | {
  path: string;
  platforms?: Array<"windows" | "macos">;
  architectures?: Array<"x86_64" | "aarch64">;
};

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
  platforms?: Array<"windows" | "macos">;
  entry: {
    mode: "auto" | "executable" | "dev";
    executable: PluginExecutable[];
    dev?: { program: string; args: string[]; cwd: string };
  };
  standalone: { instructions: string };
}

export interface PluginOrigin {
  type: "local" | "git" | "archive";
  localPath?: string;
  repository?: string;
}

export interface PluginDirectoryView {
  path: string;
  isDefault: boolean;
}

export type AppTheme = "system" | "light" | "dark" | "paper" | "forest" | "ocean" | "sunset";

export interface AppSettings {
  launchAtStartup: boolean;
  silentStart: boolean;
  theme: AppTheme;
  pluginDir: PluginDirectoryView;
}

export interface PluginView {
  manifest: PluginManifest;
  root: string;
  source: PluginSource;
  origin: PluginOrigin | null;
  rules: PluginRules;
  runtime: {
    status: PluginStatus;
    pid: number | null;
    detail: string | null;
  };
}

export type PluginImportOutcome = {
  status: "imported";
  plugin: PluginView;
} | {
  status: "needsManifest";
  sessionId: string;
  source: string;
  draft: PluginManifest;
  executableCandidates: string[];
  platform: "windows" | "macos";
  architecture: "x86_64" | "aarch64";
};

export interface ExtractPluginResult {
  outputDir: string;
  files: string[];
}
