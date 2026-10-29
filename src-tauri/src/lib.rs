use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, State,
};

const MANIFEST_FILE: &str = "office-plugin.json";
const RULES_FILE: &str = "rules.json";
const ORIGINS_FILE: &str = "plugin-origins.json";
const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginCommand {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "current_dir")]
    cwd: String,
    #[serde(default)]
    env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginWeb {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginExecutableVariant {
    path: String,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    architectures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum PluginExecutable {
    Path(String),
    Variant(PluginExecutableVariant),
}

impl PluginExecutable {
    fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Variant(variant) => &variant.path,
        }
    }

    fn platforms(&self) -> &[String] {
        match self {
            Self::Path(_) => &[],
            Self::Variant(variant) => &variant.platforms,
        }
    }

    fn architectures(&self) -> &[String] {
        match self {
            Self::Path(_) => &[],
            Self::Variant(variant) => &variant.architectures,
        }
    }

    fn matches_target(&self, platform: &str, architecture: &str) -> bool {
        (self.platforms().is_empty() || self.platforms().iter().any(|item| item == platform))
            && (self.architectures().is_empty() || self.architectures().iter().any(|item| item == architecture))
    }

    fn matches_current_target(&self) -> bool {
        self.matches_target(current_platform(), current_architecture())
    }
}

impl From<&str> for PluginExecutable {
    fn from(path: &str) -> Self {
        Self::Path(path.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginEntry {
    #[serde(default = "auto_mode")]
    mode: String,
    #[serde(default)]
    executable: Vec<PluginExecutable>,
    dev: Option<PluginCommand>,
    build: Option<PluginCommand>,
    #[serde(default)]
    web: Option<PluginWeb>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StandaloneInfo {
    instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    schema_version: u32,
    id: String,
    name: String,
    version: String,
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    category: String,
    #[serde(default = "default_icon")]
    icon: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    platforms: Vec<String>,
    entry: PluginEntry,
    standalone: StandaloneInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginRules {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    auto_start: bool,
    #[serde(default)]
    detach: bool,
    #[serde(default = "yes")]
    allow_network: bool,
    #[serde(default = "yes")]
    allow_clipboard: bool,
    #[serde(default = "yes")]
    allow_file_system: bool,
    #[serde(default = "yes")]
    allow_global_shortcut: bool,
    #[serde(default)]
    allow_plugin_autostart: bool,
}

impl Default for PluginRules {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_start: false,
            detach: false,
            allow_network: true,
            allow_clipboard: true,
            allow_file_system: true,
            allow_global_shortcut: true,
            allow_plugin_autostart: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginRuntime {
    status: String,
    pid: Option<u32>,
    detail: Option<String>,
}

impl PluginRuntime {
    fn stopped() -> Self {
        Self { status: "stopped".into(), pid: None, detail: None }
    }

    fn new(status: &str, pid: Option<u32>, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self { status: status.into(), pid, detail: (!detail.is_empty()).then_some(detail) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginOrigin {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginView {
    manifest: PluginManifest,
    root: String,
    source: String,
    origin: Option<PluginOrigin>,
    rules: PluginRules,
    runtime: PluginRuntime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum PluginImportOutcome {
    #[serde(rename = "imported")]
    Imported { plugin: PluginView },
    #[serde(rename = "needsManifest")]
    NeedsManifest {
        session_id: String,
        source: String,
        draft: PluginManifest,
        executable_candidates: Vec<String>,
        platform: String,
        architecture: String,
    },
}

struct PendingPluginImport {
    source: PathBuf,
    origin: PluginOrigin,
    cleanup: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractPluginResult {
    output_dir: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginDirectoryView {
    path: String,
    is_default: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum AppTheme {
    System,
    #[default]
    Light,
    Dark,
    Paper,
    Forest,
    Ocean,
    Sunset,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    plugin_dir: Option<PathBuf>,
    #[serde(default)]
    launch_at_startup: bool,
    #[serde(default)]
    silent_start: bool,
    #[serde(default)]
    theme: AppTheme,
    #[serde(default)]
    proxy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsView {
    launch_at_startup: bool,
    silent_start: bool,
    theme: AppTheme,
    plugin_dir: PluginDirectoryView,
    proxy: String,
}

struct AppState {
    data_dir: PathBuf,
    plugin_dir: Mutex<PathBuf>,
    settings: Mutex<StoredSettings>,
    rules: Mutex<HashMap<String, PluginRules>>,
    origins: Mutex<HashMap<String, PluginOrigin>>,
    runtimes: Mutex<HashMap<String, PluginRuntime>>,
    running: Mutex<HashMap<String, Child>>,
    detached: Mutex<HashMap<String, Child>>,
    pending_imports: Mutex<HashMap<String, PendingPluginImport>>,
}

fn yes() -> bool { true }
fn current_dir() -> String { ".".into() }
fn auto_mode() -> String { "auto".into() }
fn default_icon() -> String { "puzzle".into() }

fn default_data_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("OFFICE_TOOLBOX_HOME") {
        return PathBuf::from(custom);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OfficeToolbox")
}

fn load_rules(data_dir: &Path) -> HashMap<String, PluginRules> {
    fs::read_to_string(data_dir.join(RULES_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_rules(state: &AppState) -> Result<(), String> {
    fs::create_dir_all(&state.data_dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let rules = state.rules.lock().unwrap();
    let bytes = serde_json::to_vec_pretty(&*rules).map_err(|e| e.to_string())?;
    let target = state.data_dir.join(RULES_FILE);
    fs::write(target, bytes).map_err(|e| format!("保存插件规则失败：{e}"))
}

fn load_origins(data_dir: &Path) -> HashMap<String, PluginOrigin> {
    fs::read_to_string(data_dir.join(ORIGINS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_origins(state: &AppState) -> Result<(), String> {
    fs::create_dir_all(&state.data_dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let origins = state.origins.lock().unwrap();
    let bytes = serde_json::to_vec_pretty(&*origins).map_err(|e| e.to_string())?;
    fs::write(state.data_dir.join(ORIGINS_FILE), bytes).map_err(|e| format!("保存插件来源失败：{e}"))
}

fn load_settings(data_dir: &Path) -> StoredSettings {
    fs::read_to_string(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<StoredSettings>(&text).ok())
        .unwrap_or_default()
}

fn save_settings(state: &AppState) -> Result<(), String> {
    fs::create_dir_all(&state.data_dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let settings = state.settings.lock().unwrap();
    let bytes = serde_json::to_vec_pretty(&*settings).map_err(|e| e.to_string())?;
    fs::write(state.data_dir.join(SETTINGS_FILE), bytes).map_err(|e| format!("保存设置失败：{e}"))
}

#[cfg(test)]
fn load_plugin_dir(data_dir: &Path) -> PathBuf {
    load_settings(data_dir).plugin_dir.unwrap_or_else(|| data_dir.join("plugins"))
}

fn plugins_dir(state: &AppState) -> PathBuf {
    state.plugin_dir.lock().unwrap().clone()
}

fn plugin_directory_view(state: &AppState) -> PluginDirectoryView {
    let path = plugins_dir(state);
    PluginDirectoryView {
        is_default: path == state.data_dir.join("plugins"),
        path: path.to_string_lossy().to_string(),
    }
}

fn save_plugin_dir(state: &AppState, path: &Path) -> Result<(), String> {
    let default = state.data_dir.join("plugins");
    state.settings.lock().unwrap().plugin_dir = (path != default).then(|| path.to_path_buf());
    save_settings(state).map_err(|e| format!("保存插件目录设置失败：{e}"))
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err(format!("插件路径必须位于插件目录内：{relative}"));
    }
    Ok(root.join(path))
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err(format!("不支持的插件规范版本：{}", manifest.schema_version));
    }
    let valid_id = !manifest.id.is_empty()
        && manifest.id.len() <= 64
        && manifest.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'));
    if !valid_id {
        return Err("插件 id 只能包含小写字母、数字、连字符和下划线".into());
    }
    if manifest.name.trim().is_empty() || manifest.version.trim().is_empty() {
        return Err("插件名称和版本不能为空".into());
    }
    if !matches!(manifest.entry.mode.as_str(), "auto" | "executable" | "dev") {
        return Err(format!("未知启动模式：{}", manifest.entry.mode));
    }
    if manifest.entry.executable.is_empty() && manifest.entry.dev.is_none() {
        return Err("插件至少需要声明 executable 或 dev 启动方式".into());
    }
    if manifest.entry.mode == "executable" && manifest.entry.executable.is_empty() {
        return Err("executable 模式必须声明程序路径".into());
    }
    if manifest.entry.mode == "dev" && manifest.entry.dev.is_none() {
        return Err("dev 模式必须声明开发启动命令".into());
    }
    for platform in &manifest.platforms {
        if !matches!(platform.as_str(), "windows" | "macos") {
            return Err(format!("未知插件平台：{platform}；目前支持 windows、macos"));
        }
    }
    for candidate in &manifest.entry.executable {
        if candidate.path().trim().is_empty() {
            return Err("插件程序路径不能为空".into());
        }
        safe_join(Path::new(""), candidate.path())?;
        for platform in candidate.platforms() {
            if !matches!(platform.as_str(), "windows" | "macos") {
                return Err(format!("未知插件入口平台：{platform}；目前支持 windows、macos"));
            }
        }
        for architecture in candidate.architectures() {
            if !matches!(architecture.as_str(), "x86_64" | "aarch64") {
                return Err(format!("未知插件入口架构：{architecture}；目前支持 x86_64、aarch64"));
            }
        }
    }
    if let Some(command) = &manifest.entry.dev {
        if command.program.trim().is_empty() {
            return Err("开发启动命令不能为空".into());
        }
        safe_join(Path::new(""), &command.cwd)?;
    }
    if let Some(command) = &manifest.entry.build {
        if command.program.trim().is_empty() {
            return Err("构建命令不能为空".into());
        }
        safe_join(Path::new(""), &command.cwd)?;
    }
    if let Some(web) = &manifest.entry.web {
        if !(web.url.starts_with("http://") || web.url.starts_with("https://")) || web.url.chars().any(char::is_whitespace) {
            return Err("Web 服务地址必须是不含空白字符的 http:// 或 https:// 地址".into());
        }
    }
    Ok(())
}

fn read_manifest(root: &Path) -> Result<PluginManifest, String> {
    let path = root.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let manifest: PluginManifest = serde_json::from_str(&text).map_err(|e| format!("插件清单格式错误：{e}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn current_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    return "macos";
    #[cfg(not(target_os = "macos"))]
    return "windows";
}

fn current_architecture() -> &'static str {
    std::env::consts::ARCH
}

fn ensure_plugin_compatible(root: &Path, manifest: &PluginManifest) -> Result<(), String> {
    if !manifest.platforms.is_empty() && !manifest.platforms.iter().any(|platform| platform == current_platform()) {
        return Err(format!("此插件不支持当前系统（{}），声明支持：{}", current_platform(), manifest.platforms.join("、")));
    }
    let has_executable = manifest.entry.executable.iter().filter(|candidate| candidate.matches_current_target()).any(|candidate| {
        safe_join(root, candidate.path()).map(|path| path.is_file()).unwrap_or(false)
    });
    let has_dev = manifest.entry.dev.as_ref().is_some_and(|command| {
        safe_join(root, &command.cwd).map(|path| path.is_dir()).unwrap_or(false)
    });
    let usable = match manifest.entry.mode.as_str() {
        "executable" => has_executable,
        "dev" => has_dev,
        _ => has_executable || has_dev,
    };
    if !usable {
        return Err(format!(
            "插件与当前环境不兼容：没有适用于 {}/{} 的已编译程序，且 dev 启动工作目录无效。请提供当前平台和架构的编译产物或有效开发命令",
            current_platform(),
            current_architecture()
        ));
    }
    Ok(())
}

fn suggested_plugin_id(name: &str) -> String {
    let mut id = String::new();
    let mut separated = false;
    for character in name.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '-' | '_') {
            id.push(character);
            separated = false;
        } else if character.is_ascii_uppercase() {
            id.push(character.to_ascii_lowercase());
            separated = false;
        } else if !id.is_empty() && !separated {
            id.push('-');
            separated = true;
        }
    }
    if id.len() > 64 {
        id.truncate(64);
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() { "imported-plugin".into() } else { id }
}

fn probable_executable(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if ["unins", "uninstall", "setup", "installer", "updater"].iter().any(|keyword| name.contains(keyword)) {
        return false;
    }
    #[cfg(windows)]
    {
        path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|metadata| metadata.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        false
    }
}

fn scan_executables(root: &Path, directory: &Path, depth: usize, visited: &mut usize, candidates: &mut Vec<String>) {
    if depth > 8 || *visited >= 5000 || candidates.len() >= 50 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        if *visited >= 5000 || candidates.len() >= 50 {
            break;
        }
        *visited += 1;
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            let name = entry.file_name();
            if matches!(name.to_string_lossy().as_ref(), ".git" | "node_modules" | "deps" | "incremental" | ".fingerprint" | "__pycache__") {
                continue;
            }
            scan_executables(root, &path, depth + 1, visited, candidates);
        } else if kind.is_file() && probable_executable(&path) {
            if let Ok(relative) = path.strip_prefix(root) {
                candidates.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn executable_candidates(root: &Path) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut visited = 0;
    scan_executables(root, root, 0, &mut visited, &mut candidates);
    candidates.sort_by_key(|path| {
        let lower = path.to_ascii_lowercase();
        let build_rank = if lower.contains("/release/") { 0 } else if lower.contains("/debug/") { 1 } else { 2 };
        (build_rank, path.matches('/').count(), lower)
    });
    candidates
}

fn package_json(root: &Path) -> Option<serde_json::Value> {
    fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn node_program(root: &Path) -> &'static str {
    if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() { "bun" } else { "npm" }
}

fn suggested_build_command(root: &Path) -> Option<PluginCommand> {
    package_json(root)?;
    let program = node_program(root);
    let args = if program == "npm" {
        vec!["install".into(), "--no-audit".into(), "--no-fund".into()]
    } else {
        vec!["install".into()]
    };
    Some(PluginCommand { program: program.into(), args, cwd: current_dir(), env: HashMap::new() })
}

fn suggested_dev_command(root: &Path) -> Option<PluginCommand> {
    if let Some(package) = package_json(root) {
        if let Some(scripts) = package.get("scripts") {
            for name in ["dev", "start"] {
                if scripts.get(name).and_then(serde_json::Value::as_str).is_some() {
                    let program = node_program(root);
                    return Some(PluginCommand { program: program.into(), args: vec!["run".into(), name.into()], cwd: current_dir(), env: HashMap::new() });
                }
            }
        }
    }
    if root.join("Cargo.toml").is_file() {
        return Some(PluginCommand { program: "cargo".into(), args: vec!["run".into()], cwd: current_dir(), env: HashMap::new() });
    }
    for name in ["main.py", "app.py"] {
        if root.join(name).is_file() {
            return Some(PluginCommand { program: "python".into(), args: vec![name.into()], cwd: current_dir(), env: HashMap::new() });
        }
    }
    None
}

fn suggested_web(root: &Path, command: &mut PluginCommand) -> Option<PluginWeb> {
    let package = package_json(root)?;
    let script = package.get("scripts")?.get("dev")?.as_str()?;
    if !script.split_whitespace().any(|part| part == "next") {
        return None;
    }
    let args = script.split_whitespace().collect::<Vec<_>>();
    let port = args.iter().enumerate().find_map(|(index, argument)| {
        if matches!(*argument, "-p" | "--port") {
            args.get(index + 1).and_then(|value| value.parse::<u16>().ok())
        } else {
            argument.strip_prefix("--port=").and_then(|value| value.parse::<u16>().ok())
        }
    }).unwrap_or(3000);
    command.env.insert("HOSTNAME".into(), "127.0.0.1".into());
    command.env.insert("NEXT_TELEMETRY_DISABLED".into(), "1".into());
    if package.get("name").and_then(serde_json::Value::as_str) == Some("libretv") {
        command.env.insert("PASSWORD".into(), "libretv".into());
    }
    Some(PluginWeb { url: format!("http://127.0.0.1:{port}") })
}

fn manifest_draft(root: &Path, id: String) -> (PluginManifest, Vec<String>) {
    let candidates = executable_candidates(root);
    let build = suggested_build_command(root);
    let mut dev = suggested_dev_command(root);
    let web = dev.as_mut().and_then(|command| suggested_web(root, command));
    let mode = if candidates.is_empty() {
        "dev"
    } else if dev.is_some() {
        "auto"
    } else {
        "executable"
    };
    let executable = candidates.first().map(|path| {
        vec![PluginExecutable::Variant(PluginExecutableVariant {
            path: path.clone(),
            platforms: vec![current_platform().into()],
            architectures: vec![current_architecture().into()],
        })]
    }).unwrap_or_default();
    let name = root.file_name().unwrap_or_default().to_string_lossy().to_string();
    let manifest = PluginManifest {
        schema_version: 1,
        id,
        name: if name.is_empty() { "导入插件".into() } else { name },
        version: "0.1.0".into(),
        description: "通过导入向导生成清单的插件。".into(),
        author: String::new(),
        category: "办公效率".into(),
        icon: default_icon(),
        permissions: Vec::new(),
        platforms: vec![current_platform().into()],
        entry: PluginEntry { mode: mode.into(), executable, dev, build, web },
        standalone: StandaloneInfo { instructions: "由办公工具箱按照生成的 office-plugin.json 启动。".into() },
    };
    (manifest, candidates)
}

fn set_runtime(state: &AppState, id: &str, runtime: PluginRuntime) -> PluginRuntime {
    state.runtimes.lock().unwrap().insert(id.into(), runtime.clone());
    runtime
}

fn exited_runtime(
    state: &AppState,
    id: &str,
    result: Result<std::process::ExitStatus, String>,
    starting: bool,
    detached: bool,
) -> PluginRuntime {
    let runtime = match result {
        Ok(status) if status.success() && !starting => PluginRuntime::new(
            "stopped",
            None,
            if detached { "独立插件已退出" } else { "插件已退出" },
        ),
        Ok(status) => PluginRuntime::new(
            "failed",
            None,
            if starting {
                format!("插件在就绪前退出（{status}），请查看编译与启动日志")
            } else {
                format!("插件异常退出（{status}），请查看运行日志")
            },
        ),
        Err(error) => PluginRuntime::new("failed", None, error),
    };
    set_runtime(state, id, runtime)
}

fn runtime_for(state: &AppState, id: &str) -> PluginRuntime {
    let previous = state.runtimes.lock().unwrap().get(id).cloned().unwrap_or_else(PluginRuntime::stopped);
    let starting = previous.status == "starting";
    let mut managed_exit = None;
    {
        let mut running = state.running.lock().unwrap();
        if let Some(child) = running.get_mut(id) {
            let pid = child.id();
            match child.try_wait() {
                Ok(Some(status)) => managed_exit = Some(Ok(status)),
                Ok(None) => {
                    if previous.pid == Some(pid) && matches!(previous.status.as_str(), "starting" | "running") {
                        return previous;
                    }
                    return PluginRuntime::new("running", Some(pid), "插件进程运行中");
                }
                Err(error) => managed_exit = Some(Err(format!("读取插件进程状态失败：{error}"))),
            }
        }
        if managed_exit.is_some() {
            running.remove(id);
        }
    }
    if let Some(result) = managed_exit {
        return exited_runtime(state, id, result, starting, false);
    }

    let mut detached_exit = None;
    {
        let mut detached = state.detached.lock().unwrap();
        if let Some(child) = detached.get_mut(id) {
            let pid = child.id();
            match child.try_wait() {
                Ok(Some(status)) => detached_exit = Some(Ok(status)),
                Ok(None) => return PluginRuntime::new("detached", Some(pid), "独立进程由插件自行管理"),
                Err(error) => detached_exit = Some(Err(format!("读取独立插件进程状态失败：{error}"))),
            }
        }
        if detached_exit.is_some() {
            detached.remove(id);
        }
    }
    if let Some(result) = detached_exit {
        return exited_runtime(state, id, result, false, true);
    }
    previous
}

fn plugin_roots<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    // 开发时优先使用当前源码，避免 target/debug/plugins 中残留的旧副本覆盖源码插件。
    #[cfg(debug_assertions)]
    {
        let dev_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new(".")).join("plugins");
        roots.push((dev_root, "builtin".into()));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push((dir.join("plugins"), "builtin".into()));
        }
    }
    if let Ok(dir) = app.path().resource_dir() {
        roots.push((dir.join("plugins"), "builtin".into()));
    }
    roots.push((plugins_dir(state), "imported".into()));

    let mut seen = HashSet::new();
    roots.retain(|(root, _)| {
        let key = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        seen.insert(key)
    });
    roots
}

fn discover_plugins<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> Vec<PluginView> {
    let mut plugins = Vec::new();
    let mut ids = HashSet::new();
    for (root, source) in plugin_roots(app, state) {
        let Ok(entries) = fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || !path.join(MANIFEST_FILE).is_file() { continue }
            let Ok(manifest) = read_manifest(&path) else { continue };
            if !ids.insert(manifest.id.clone()) { continue }
            let rules = state.rules.lock().unwrap().get(&manifest.id).cloned().unwrap_or_default();
            let origin = state.origins.lock().unwrap().get(&manifest.id).cloned();
            let runtime = runtime_for(state, &manifest.id);
            plugins.push(PluginView {
                manifest,
                root: path.to_string_lossy().to_string(),
                source: source.clone(),
                origin,
                rules,
                runtime,
            });
        }
    }
    plugins.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    plugins
}

fn find_plugin<R: Runtime>(app: &AppHandle<R>, state: &AppState, id: &str) -> Result<PluginView, String> {
    discover_plugins(app, state).into_iter().find(|plugin| plugin.manifest.id == id).ok_or_else(|| "插件不存在或清单无效".into())
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

fn resolve_command(program: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        if matches!(program.to_ascii_lowercase().as_str(), "npm" | "npx") {
            let mut resolved_args = vec!["/D".into(), "/S".into(), "/C".into(), program.into()];
            resolved_args.extend_from_slice(args);
            return ("cmd.exe".into(), resolved_args);
        }
        if program.eq_ignore_ascii_case("bun") {
            if let Some(home) = dirs::home_dir() {
                let bun = home.join(".bun").join("bin").join("bun.exe");
                if bun.is_file() {
                    return (bun.to_string_lossy().to_string(), args.to_vec());
                }
            }
        }
    }
    (program.into(), args.to_vec())
}

fn prepare_launch(plugin: &PluginView) -> Result<(String, Vec<String>, PathBuf, bool), String> {
    let root = PathBuf::from(&plugin.root);
    let entry = &plugin.manifest.entry;
    // 开发宿主中的内置插件始终走 dev 命令，确保配置和源码更新生效，同时复用编译缓存。
    let prefer_dev = cfg!(debug_assertions) && plugin.source == "builtin" && entry.dev.is_some();
    if entry.mode != "dev" && !prefer_dev {
        for candidate in entry.executable.iter().filter(|candidate| candidate.matches_current_target()) {
            let executable = safe_join(&root, candidate.path())?;
            if executable.is_file() {
                return Ok((executable.to_string_lossy().to_string(), vec![], root, false));
            }
        }
    }
    if entry.mode != "executable" {
        if let Some(command) = &entry.dev {
            let cwd = safe_join(&root, &command.cwd)?;
            if !cwd.is_dir() {
                return Err(format!("插件工作目录不存在：{}", cwd.display()));
            }
            let (program, args) = resolve_command(&command.program, &command.args);
            return Ok((program, args, cwd, true));
        }
    }
    Err(format!(
        "未找到适用于 {}/{} 的已构建插件程序；请先构建插件，或提供有效的 dev 启动命令",
        current_platform(),
        current_architecture()
    ))
}

fn read_log(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else { return String::new() };
    let start = bytes.len().saturating_sub(64 * 1024);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn log_tail(path: &Path) -> String {
    let text = read_log(path);
    let tail: String = text.chars().rev().take(5000).collect::<String>().chars().rev().collect();
    if tail.trim().is_empty() { "暂无输出，请检查插件程序、依赖和权限。".into() } else { tail }
}

fn output_is_ready(output: &str, id: &str) -> bool {
    let marker = format!("OFFICE_TOOLBOX_READY:{id}");
    output.lines().any(|line| line.trim() == marker)
}

fn plugin_is_ready(path: &Path, id: &str) -> bool {
    output_is_ready(&read_log(path), id)
}

enum BuildAttemptError {
    Retryable(String),
    Fatal(String),
}

fn run_build_command_once(
    state: &AppState,
    id: &str,
    plugin: &PluginView,
    log: &fs::File,
    log_path: &Path,
    proxy: Option<&str>,
) -> Result<(), BuildAttemptError> {
    let Some(build) = &plugin.manifest.entry.build else { return Ok(()) };
    use std::io::Write;

    let root = PathBuf::from(&plugin.root);
    let cwd = safe_join(&root, &build.cwd).map_err(BuildAttemptError::Fatal)?;
    if !cwd.is_dir() {
        return Err(BuildAttemptError::Fatal(format!("构建工作目录不存在：{}", cwd.display())));
    }
    let (program, args) = resolve_command(&build.program, &build.args);
    let mut log = log.try_clone().map_err(|e| BuildAttemptError::Fatal(format!("写入构建日志失败：{e}")))?;
    let route = if proxy.is_some() { "代理" } else { "直连" };
    writeln!(log, "[办公工具箱] 正在通过{route}准备依赖：{program} {}", args.join(" ")).map_err(|e| BuildAttemptError::Fatal(format!("写入构建日志失败：{e}")))?;
    let error_log = log.try_clone().map_err(|e| BuildAttemptError::Fatal(format!("写入构建日志失败：{e}")))?;
    let mut command = Command::new(&program);
    command
        .args(&args)
        .current_dir(&cwd)
        .envs(&build.env)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log));
    apply_command_proxy(&mut command, proxy);
    hide_console(&mut command);
    let child = command.spawn().map_err(|error| BuildAttemptError::Retryable(format!("无法执行依赖准备命令（{program}）：{error}")))?;
    let pid = child.id();
    state.running.lock().unwrap().insert(id.into(), child);
    set_runtime(state, id, PluginRuntime::new("starting", Some(pid), "正在安装或检查插件依赖，首次运行可能较慢"));
    let started = std::time::Instant::now();
    loop {
        let check = {
            let mut running = state.running.lock().unwrap();
            running.get_mut(id).map(|child| child.try_wait())
        };
        let Some(check) = check else { return Err(BuildAttemptError::Fatal("插件启动已取消。".into())) };
        match check {
            Ok(Some(status)) if status.success() => {
                state.running.lock().unwrap().remove(id);
                return Ok(());
            }
            Ok(Some(status)) => {
                state.running.lock().unwrap().remove(id);
                let detail = format!("插件依赖准备失败（{status}）。\n\n最近日志：\n{}", log_tail(log_path));
                return Err(BuildAttemptError::Retryable(detail));
            }
            Err(error) => {
                state.running.lock().unwrap().remove(id);
                set_runtime(state, id, PluginRuntime::new("failed", None, "读取依赖准备进程失败"));
                return Err(BuildAttemptError::Fatal(format!("读取依赖准备进程失败：{error}")));
            }
            Ok(None) => {}
        }
        if started.elapsed() >= std::time::Duration::from_secs(30 * 60) {
            let _ = stop_tracked(state, id);
            let detail = format!("依赖准备超过 1800 秒，已停止启动。\n\n最近日志：\n{}", log_tail(log_path));
            set_runtime(state, id, PluginRuntime::new("failed", None, "依赖准备超时，请查看详细日志"));
            return Err(BuildAttemptError::Fatal(detail));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn run_build_command(state: &AppState, id: &str, plugin: &PluginView, log: &fs::File, log_path: &Path) -> Result<(), String> {
    if plugin.manifest.entry.build.is_none() {
        return Ok(());
    }
    use std::io::Write;

    let candidates = proxy_candidates(state);
    let attempted_proxy = candidates.len() > 1;
    for proxy in candidates {
        match run_build_command_once(state, id, plugin, log, log_path, proxy.as_deref()) {
            Ok(()) => return Ok(()),
            Err(BuildAttemptError::Fatal(error)) => return Err(error),
            Err(BuildAttemptError::Retryable(error)) if proxy.is_some() => {
                let mut retry_log = log.try_clone().map_err(|e| format!("写入构建日志失败：{e}"))?;
                writeln!(retry_log, "[办公工具箱] 代理依赖准备未成功，正在改用直连重试。\n{error}")
                    .map_err(|e| format!("写入构建日志失败：{e}"))?;
            }
            Err(BuildAttemptError::Retryable(error)) => {
                set_runtime(state, id, PluginRuntime::new("failed", None, "插件依赖准备失败，请查看详细日志"));
                return Err(if attempted_proxy {
                    format!("{error}\n已分别尝试代理和直连，均未成功。")
                } else {
                    error
                });
            }
        }
    }
    Err("插件依赖准备失败".into())
}

fn web_is_ready(url: &str) -> bool {
    match ureq::get(url).timeout(std::time::Duration::from_secs(1)).call() {
        Ok(_) | Err(ureq::Error::Status(_, _)) => true,
        Err(_) => false,
    }
}

fn open_web_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = Command::new("rundll32");
    #[cfg(windows)]
    command.args(["url.dll,FileProtocolHandler", url]);
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "macos")]
    command.arg(url);
    #[cfg(not(any(windows, target_os = "macos")))]
    return Err("当前系统不支持自动打开浏览器".into());
    #[cfg(any(windows, target_os = "macos"))]
    {
        hide_console(&mut command);
        command.spawn().map_err(|error| format!("打开浏览器失败：{error}"))?;
        Ok(())
    }
}

fn launch_by_id(app: &AppHandle, state: &AppState, id: &str, detached: Option<bool>) -> Result<PluginRuntime, String> {
    let plugin = find_plugin(app, state, id)?;
    if !plugin.rules.enabled {
        return Err("插件已停用，请先在规则中启用".into());
    }
    let current = runtime_for(state, id);
    if matches!(current.status.as_str(), "starting" | "running" | "detached") {
        return Ok(current);
    }
    let detach = detached.unwrap_or(plugin.rules.detach);
    let (program, args, cwd, dev_mode) = prepare_launch(&plugin).inspect_err(|error| {
        set_runtime(state, id, PluginRuntime::new("failed", None, error.clone()));
    })?;
    let plugin_data = state.data_dir.join("plugin-data").join(id);
    let log_dir = state.data_dir.join("logs");
    fs::create_dir_all(&plugin_data).map_err(|e| format!("创建插件数据目录失败：{e}"))?;
    fs::create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败：{e}"))?;
    let rules_path = plugin_data.join("rules.json");
    fs::write(&rules_path, serde_json::to_vec_pretty(&plugin.rules).map_err(|e| e.to_string())?).map_err(|e| format!("写入插件规则失败：{e}"))?;

    let log_path = log_dir.join(format!("{id}.log"));
    let mut log = fs::OpenOptions::new().create(true).write(true).truncate(true).open(&log_path).map_err(|e| format!("打开插件日志失败：{e}"))?;
    use std::io::Write;
    writeln!(log, "[办公工具箱] 启动命令：{program}；工作目录：{}", cwd.display()).map_err(|e| format!("写入插件日志失败：{e}"))?;
    writeln!(log, "[办公工具箱] 正在等待插件就绪，首次开发启动可能需要编译，请耐心等待。").map_err(|e| format!("写入插件日志失败：{e}"))?;
    if dev_mode {
        if let Err(error) = run_build_command(state, id, &plugin, &log, &log_path) {
            if runtime_for(state, id).status != "stopped" {
                set_runtime(state, id, PluginRuntime::new("failed", None, error.clone()));
            }
            return Err(error);
        }
    }
    let error_log = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(&program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .env("OFFICE_TOOLBOX", "1")
        .env("OFFICE_PLUGIN_ID", id)
        .env("OFFICE_PLUGIN_DATA_DIR", &plugin_data)
        .env("OFFICE_PLUGIN_RULES_FILE", &rules_path)
        .env("OFFICE_PLUGIN_ALLOW_NETWORK", plugin.rules.allow_network.to_string())
        .env("OFFICE_PLUGIN_ALLOW_CLIPBOARD", plugin.rules.allow_clipboard.to_string())
        .env("OFFICE_PLUGIN_ALLOW_FILE_SYSTEM", plugin.rules.allow_file_system.to_string())
        .env("OFFICE_PLUGIN_ALLOW_GLOBAL_SHORTCUT", plugin.rules.allow_global_shortcut.to_string())
        .env("OFFICE_PLUGIN_ALLOW_AUTOSTART", plugin.rules.allow_plugin_autostart.to_string());
    if dev_mode {
        if let Some(dev) = &plugin.manifest.entry.dev {
            command.envs(&dev.env);
        }
    }
    hide_console(&mut command);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let detail = format!("无法创建插件进程（{program}）：{error}。请检查程序路径和依赖环境。");
            set_runtime(state, id, PluginRuntime::new("failed", None, detail.clone()));
            return Err(detail);
        }
    };
    let pid = child.id();
    if detach {
        state.running.lock().unwrap().remove(id);
        state.detached.lock().unwrap().insert(id.into(), child);
        return Ok(set_runtime(state, id, PluginRuntime::new("detached", Some(pid), "独立进程由插件自行管理")));
    }

    state.detached.lock().unwrap().remove(id);
    state.running.lock().unwrap().insert(id.into(), child);
    set_runtime(state, id, PluginRuntime::new(
        "starting",
        Some(pid),
        if dev_mode { "正在编译并启动插件，首次运行可能较慢" } else { "正在启动插件并等待就绪" },
    ));

    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(if dev_mode { 30 * 60 } else { 30 });
    loop {
        let check = {
            let mut running = state.running.lock().unwrap();
            running.get_mut(id).map(|child| child.try_wait())
        };
        let Some(check) = check else {
            let current = runtime_for(state, id);
            if current.status == "failed" {
                return Err(current.detail.unwrap_or_else(|| "插件启动失败，请查看日志".into()));
            }
            return Err("插件启动已取消。".into());
        };
        match check {
            Ok(Some(status)) => {
                state.running.lock().unwrap().remove(id);
                let detail = format!("插件启动或编译失败（{status}）。\n\n最近日志：\n{}", log_tail(&log_path));
                set_runtime(state, id, PluginRuntime::new("failed", None, "插件启动或编译失败，请查看详细日志"));
                return Err(detail);
            }
            Err(error) => {
                state.running.lock().unwrap().remove(id);
                let detail = format!("读取启动中的插件进程失败：{error}");
                set_runtime(state, id, PluginRuntime::new("failed", None, detail.clone()));
                return Err(detail);
            }
            Ok(None) => {}
        }

        let web = plugin.manifest.entry.web.as_ref();
        let web_ready = web.is_some_and(|web| web_is_ready(&web.url));
        let ready = if web.is_some() { web_ready } else { plugin_is_ready(&log_path, id) };
        if ready {
            if !state.running.lock().unwrap().contains_key(id) {
                return Err("插件启动已取消。".into());
            }
            let mut detail = "插件已就绪".to_string();
            if let Some(web) = web {
                detail = match open_web_url(&web.url) {
                    Ok(()) => format!("Web 服务已就绪：{}", web.url),
                    Err(error) => format!("Web 服务已就绪，但{error}；请手动打开 {}", web.url),
                };
            }
            return Ok(set_runtime(state, id, PluginRuntime::new("running", Some(pid), detail)));
        }
        // 未适配就绪信号的单文件插件保持兼容；开发命令必须等待明确信号。
        if !dev_mode && web.is_none() && started.elapsed() >= std::time::Duration::from_secs(2) {
            return Ok(set_runtime(state, id, PluginRuntime::new("running", Some(pid), "进程运行中；插件未提供就绪信号")));
        }
        if started.elapsed() >= timeout {
            let _ = stop_tracked(state, id);
            let detail = format!("等待插件就绪超过 {} 秒，已停止启动。\n\n最近日志：\n{}", timeout.as_secs(), log_tail(&log_path));
            set_runtime(state, id, PluginRuntime::new("failed", None, "等待插件就绪超时，请查看详细日志"));
            return Err(detail);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn stop_tracked(state: &AppState, id: &str) -> Result<bool, String> {
    let child = state.running.lock().unwrap().remove(id);
    let Some(mut child) = child else { return Ok(false) };
    #[cfg(windows)]
    {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut taskkill);
        let _ = taskkill.status();
    }
    let _ = child.kill();
    let _ = child.wait();
    set_runtime(state, id, PluginRuntime::new("stopped", None, "插件已手动停止"));
    Ok(true)
}

fn copy_plugin_directory(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if matches!(name_text.as_ref(), ".git" | "node_modules" | "target" | "dist" | "build" | ".next" | "__pycache__") {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() { continue }
        let from = entry.path();
        let to = target.join(&name);
        if kind.is_dir() {
            copy_plugin_directory(&from, &to)?;
        } else if kind.is_file() {
            fs::copy(&from, &to).map_err(|e| format!("复制 {} 失败：{e}", from.display()))?;
        }
    }
    Ok(())
}

#[tauri::command]
fn list_plugins(app: AppHandle, state: State<'_, AppState>) -> Vec<PluginView> {
    discover_plugins(&app, &state)
}

#[tauri::command]
fn get_plugin_directory(state: State<'_, AppState>) -> PluginDirectoryView {
    plugin_directory_view(&state)
}

#[tauri::command]
fn set_plugin_directory(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<PluginDirectoryView, String> {
    let path = fs::canonicalize(path.trim()).map_err(|e| format!("插件目录不可访问：{e}"))?;
    if !path.is_dir() {
        return Err("请选择一个文件夹作为插件目录".into());
    }
    if path == plugins_dir(&state) {
        return Ok(plugin_directory_view(&state));
    }
    let imported = discover_plugins(&app, &state).into_iter().filter(|plugin| plugin.source == "imported").collect::<Vec<_>>();
    if imported.iter().any(|plugin| plugin.runtime.status == "detached") {
        return Err("请先退出独立运行的导入插件，再切换插件目录".into());
    }
    save_plugin_dir(&state, &path)?;
    for plugin in imported {
        stop_tracked(&state, &plugin.manifest.id)?;
    }
    *state.plugin_dir.lock().unwrap() = path;
    Ok(plugin_directory_view(&state))
}

#[cfg(windows)]
fn apply_system_autostart(enabled: bool) -> Result<(), String> {
    use winreg::{enums::*, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| format!("打开系统启动项失败：{e}"))?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("读取主程序路径失败：{e}"))?;
        key.set_value("OfficeToolbox", &format!("\"{}\"", exe.to_string_lossy()))
            .map_err(|e| format!("写入开机启动项失败：{e}"))
    } else {
        match key.delete_value("OfficeToolbox") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("移除开机启动项失败：{error}")),
        }
    }
}

#[cfg(target_os = "macos")]
fn apply_system_autostart(enabled: bool) -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "无法定位用户目录".to_string())?;
    let agents = home.join("Library").join("LaunchAgents");
    let plist = agents.join("com.office.toolbox.plist");
    if !enabled {
        if plist.exists() {
            let _ = Command::new("launchctl").arg("unload").arg(&plist).status();
            fs::remove_file(&plist).map_err(|e| format!("移除开机启动项失败：{e}"))?;
        }
        return Ok(());
    }
    fs::create_dir_all(&agents).map_err(|e| format!("创建启动项目录失败：{e}"))?;
    let exe = std::env::current_exe().map_err(|e| format!("读取主程序路径失败：{e}"))?;
    let escape_xml = |value: &str| value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let content = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.office.toolbox</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#, escape_xml(&exe.to_string_lossy()));
    fs::write(&plist, content).map_err(|e| format!("写入开机启动项失败：{e}"))?;
    let _ = Command::new("launchctl").arg("load").arg(&plist).status();
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn apply_system_autostart(_enabled: bool) -> Result<(), String> {
    Err("当前系统暂不支持配置开机启动".into())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> AppSettingsView {
    let settings = state.settings.lock().unwrap().clone();
    AppSettingsView {
        launch_at_startup: settings.launch_at_startup,
        silent_start: settings.silent_start,
        theme: settings.theme,
        plugin_dir: plugin_directory_view(&state),
        proxy: settings.proxy,
    }
}

#[tauri::command]
fn set_launch_at_startup(state: State<'_, AppState>, enabled: bool) -> Result<AppSettingsView, String> {
    apply_system_autostart(enabled)?;
    state.settings.lock().unwrap().launch_at_startup = enabled;
    save_settings(&state)?;
    Ok(get_settings(state))
}

#[tauri::command]
fn set_silent_start(state: State<'_, AppState>, enabled: bool) -> Result<AppSettingsView, String> {
    state.settings.lock().unwrap().silent_start = enabled;
    save_settings(&state)?;
    Ok(get_settings(state))
}

#[tauri::command]
fn set_theme(state: State<'_, AppState>, theme: AppTheme) -> Result<AppSettingsView, String> {
    state.settings.lock().unwrap().theme = theme;
    save_settings(&state)?;
    Ok(get_settings(state))
}

fn save_proxy(state: &AppState, value: &str) -> Result<(), String> {
    let proxy = normalize_proxy_url(value).unwrap_or_default();
    if proxy.chars().any(char::is_whitespace) {
        return Err("代理地址不能包含空白字符".into());
    }
    if !proxy.is_empty() {
        ureq::Proxy::new(&proxy).map_err(|e| format!("代理地址无效：{e}"))?;
    }
    state.settings.lock().unwrap().proxy = proxy;
    save_settings(state)
}

#[tauri::command]
fn set_proxy(state: State<'_, AppState>, value: String) -> Result<AppSettingsView, String> {
    save_proxy(&state, &value)?;
    Ok(get_settings(state))
}

fn copy_declared_executables(manifest: &PluginManifest, source: &Path, target: &Path) -> Result<(), String> {
    let source = fs::canonicalize(source).map_err(|e| format!("读取插件目录失败：{e}"))?;
    for candidate in &manifest.entry.executable {
        let candidate = candidate.path();
        let from = safe_join(&source, candidate)?;
        if !from.is_file() { continue }
        let canonical = fs::canonicalize(&from).map_err(|e| format!("读取编译产物失败：{e}"))?;
        if !canonical.starts_with(&source) {
            return Err(format!("编译产物不能链接到插件目录之外：{candidate}"));
        }
        let to = safe_join(target, candidate)?;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建编译产物目录失败：{e}"))?;
        }
        fs::copy(&canonical, &to).map_err(|e| format!("复制编译产物 {} 失败：{e}", canonical.display()))?;
    }
    Ok(())
}

fn write_generated_manifest(root: &Path, manifest: &PluginManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| format!("生成插件清单失败：{e}"))?;
    fs::write(root.join(MANIFEST_FILE), bytes).map_err(|e| format!("写入生成的插件清单失败：{e}"))
}

fn install_plugin<R: Runtime>(app: &AppHandle<R>, state: &AppState, source: &Path, origin: PluginOrigin) -> Result<PluginView, String> {
    install_plugin_with_manifest(app, state, source, origin, None)
}

fn install_plugin_with_manifest<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    source: &Path,
    origin: PluginOrigin,
    generated: Option<PluginManifest>,
) -> Result<PluginView, String> {
    let generate_manifest = !source.join(MANIFEST_FILE).is_file();
    let manifest = if generate_manifest {
        let manifest = generated.ok_or_else(|| "插件目录缺少 office-plugin.json，请先补全插件清单".to_string())?;
        validate_manifest(&manifest)?;
        manifest
    } else {
        read_manifest(source)?
    };
    ensure_plugin_compatible(source, &manifest)?;
    if find_plugin(app, state, &manifest.id).is_ok() {
        return Err(format!("插件 id「{}」已存在，请先移除旧插件", manifest.id));
    }
    let imported_root = plugins_dir(state);
    let target = imported_root.join(&manifest.id);
    if source.starts_with(&target) || target.starts_with(source) || target.exists() {
        return Err("目标插件目录已存在或导入路径无效".into());
    }
    fs::create_dir_all(&imported_root).map_err(|e| e.to_string())?;
    let result = copy_plugin_directory(source, &target)
        .and_then(|_| copy_declared_executables(&manifest, source, &target))
        .and_then(|_| if generate_manifest { write_generated_manifest(&target, &manifest) } else { Ok(()) });
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    state.origins.lock().unwrap().insert(manifest.id.clone(), origin);
    if let Err(error) = save_origins(state) {
        state.origins.lock().unwrap().remove(&manifest.id);
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    find_plugin(app, state, &manifest.id)
}

fn temporary_path(state: &AppState, prefix: &str) -> Result<PathBuf, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok(plugins_dir(state).join(".office-toolbox-tmp").join(format!("{prefix}-{}-{nonce}", std::process::id())))
}

fn prepare_plugin_import<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    source: PathBuf,
    origin: PluginOrigin,
    cleanup: Option<PathBuf>,
) -> Result<PluginImportOutcome, String> {
    if source.join(MANIFEST_FILE).is_file() {
        let result = install_plugin(app, state, &source, origin);
        if let Some(directory) = cleanup {
            let _ = fs::remove_dir_all(directory);
        }
        return result.map(|plugin| PluginImportOutcome::Imported { plugin });
    }

    let result = (|| {
        let existing: HashSet<String> = discover_plugins(app, state).into_iter().map(|plugin| plugin.manifest.id).collect();
        let directory_name = if origin.kind == "git" {
            origin.repository.as_deref()
                .and_then(|url| url.trim_end_matches('/').rsplit('/').next())
                .map(|name| name.strip_suffix(".git").unwrap_or(name).to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| source.file_name().unwrap_or_default().to_string_lossy().to_string())
        } else {
            source.file_name().unwrap_or_default().to_string_lossy().to_string()
        };
        let base_id = suggested_plugin_id(&directory_name);
        let mut id = base_id.clone();
        let mut suffix = 2;
        while existing.contains(&id) {
            id = format!("{}-{suffix}", &base_id[..base_id.len().min(56)]);
            suffix += 1;
        }
        let (mut draft, executable_candidates) = manifest_draft(&source, id);
        if !directory_name.is_empty() {
            draft.name = directory_name;
        }
        let session_path = temporary_path(state, "plugin-import-session")?;
        let session_id = session_path.file_name().ok_or_else(|| "创建导入会话失败".to_string())?.to_string_lossy().to_string();
        let outcome = PluginImportOutcome::NeedsManifest {
            session_id: session_id.clone(),
            source: source.to_string_lossy().to_string(),
            draft,
            executable_candidates,
            platform: current_platform().into(),
            architecture: current_architecture().into(),
        };
        state.pending_imports.lock().unwrap().insert(session_id, PendingPluginImport { source, origin, cleanup: cleanup.clone() });
        Ok(outcome)
    })();
    if result.is_err() {
        if let Some(directory) = cleanup {
            let _ = fs::remove_dir_all(directory);
        }
    }
    result
}

fn finish_plugin_import<R: Runtime>(app: &AppHandle<R>, state: &AppState, session_id: &str, manifest: PluginManifest) -> Result<PluginView, String> {
    let (source, origin, cleanup) = {
        let pending = state.pending_imports.lock().unwrap();
        let pending = pending.get(session_id).ok_or_else(|| "导入会话已失效，请重新导入".to_string())?;
        (pending.source.clone(), pending.origin.clone(), pending.cleanup.clone())
    };
    let result = install_plugin_with_manifest(app, state, &source, origin, Some(manifest));
    if result.is_ok() {
        state.pending_imports.lock().unwrap().remove(session_id);
        if let Some(directory) = cleanup {
            let _ = fs::remove_dir_all(directory);
        }
    }
    result
}

#[tauri::command(async)]
fn complete_plugin_import(app: AppHandle, state: State<'_, AppState>, session_id: String, manifest: PluginManifest) -> Result<PluginView, String> {
    finish_plugin_import(&app, &state, &session_id, manifest)
}

fn discard_pending_import(state: &AppState, session_id: &str) {
    if let Some(pending) = state.pending_imports.lock().unwrap().remove(session_id) {
        if let Some(directory) = pending.cleanup {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

#[tauri::command]
fn cancel_plugin_import(state: State<'_, AppState>, session_id: String) {
    discard_pending_import(&state, &session_id);
}

fn normalize_proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(if value.contains("://") { value.into() } else { format!("http://{value}") })
}

fn parse_windows_proxy_server(value: &str) -> Option<String> {
    if !value.contains('=') {
        return normalize_proxy_url(value);
    }
    let mut http = None;
    for item in value.split(';').filter_map(|item| item.split_once('=')) {
        let protocol = item.0.trim().to_ascii_lowercase();
        if protocol == "https" {
            if let Some(proxy) = normalize_proxy_url(item.1) {
                return Some(proxy);
            }
        } else if protocol == "http" {
            http = normalize_proxy_url(item.1);
        }
    }
    http
}

#[cfg(target_os = "macos")]
fn parse_macos_proxy(value: &str) -> Option<String> {
    let setting = |key: &str| value.lines().find_map(|line| {
        let (name, value) = line.trim().split_once(':')?;
        (name.trim() == key).then(|| value.trim().to_string())
    });
    for protocol in ["HTTPS", "HTTP"] {
        if setting(&format!("{protocol}Enable")).as_deref() == Some("1") {
            if let (Some(host), Some(port)) = (setting(&format!("{protocol}Proxy")), setting(&format!("{protocol}Port"))) {
                return normalize_proxy_url(&format!("{host}:{port}"));
            }
        }
    }
    None
}

#[cfg(windows)]
fn system_proxy() -> Option<String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let settings = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings").ok()?;
    let enabled = settings.get_value::<u32, _>("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    settings.get_value::<String, _>("ProxyServer").ok().and_then(|value| parse_windows_proxy_server(&value))
}

#[cfg(target_os = "macos")]
fn system_proxy() -> Option<String> {
    let output = Command::new("scutil").arg("--proxy").output().ok()?;
    output.status.success().then(|| parse_macos_proxy(&String::from_utf8_lossy(&output.stdout))).flatten()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_proxy() -> Option<String> {
    None
}

fn network_proxy() -> Option<String> {
    for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        if let Some(proxy) = std::env::var(name).ok().and_then(|value| normalize_proxy_url(&value)) {
            return Some(proxy);
        }
    }
    system_proxy()
}

fn proxy_candidates_from(configured: &str, detected: Option<String>) -> Vec<Option<String>> {
    let configured = configured.trim();
    let first = if configured.is_empty() { detected } else { Some(configured.to_string()) };
    first.into_iter().map(Some).chain(std::iter::once(None)).collect()
}

fn proxy_candidates(state: &AppState) -> Vec<Option<String>> {
    let configured = state.settings.lock().unwrap().proxy.clone();
    let detected = configured.trim().is_empty().then(network_proxy).flatten();
    proxy_candidates_from(&configured, detected)
}

fn apply_command_proxy(command: &mut Command, proxy: Option<&str>) {
    for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        if let Some(proxy) = proxy {
            command.env(name, proxy);
        } else {
            command.env_remove(name);
        }
    }
}

fn clone_git_repository_once(url: &str, checkout: &Path, proxy: Option<&str>) -> Result<(), String> {
    // 先由 Rust 创建并验证空目录，再使用相对目标，避免部分 Windows Git 环境展开中文或特殊磁盘的绝对路径。
    fs::create_dir_all(checkout).map_err(|e| format!("创建 Git 临时工作目录失败（{}）：{e}", checkout.display()))?;
    if fs::read_dir(checkout).map_err(|e| format!("读取 Git 临时工作目录失败（{}）：{e}", checkout.display()))?.next().is_some() {
        return Err(format!("Git 临时工作目录不是空目录：{}", checkout.display()));
    }
    let mut command = Command::new("git");
    command
        .args(["clone", "--depth", "1", "--recurse-submodules", "--shallow-submodules", "--"])
        .arg(url)
        .arg(".")
        .current_dir(checkout)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_command_proxy(&mut command, proxy);
    hide_console(&mut command);
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            let _ = fs::remove_dir_all(checkout);
            return Err(format!("无法执行 Git，请先安装并配置 git：{error}"));
        }
    };
    if !output.status.success() {
        let _ = fs::remove_dir_all(checkout);
        let detail = String::from_utf8_lossy(&output.stderr).replace(url, &sanitize_repository_url(url));
        let tail = detail.chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>();
        return Err(format!("Git 仓库克隆失败：{}", if tail.trim().is_empty() { output.status.to_string() } else { tail.trim().into() }));
    }
    Ok(())
}

fn clone_git_repository(state: &AppState, url: &str, checkout: &Path) -> Result<(), String> {
    if let Some(parent) = checkout.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建临时导入目录失败：{e}"))?;
    }
    let candidates = proxy_candidates(state);
    let attempted_proxy = candidates.len() > 1;
    for proxy in candidates {
        match clone_git_repository_once(url, checkout, proxy.as_deref()) {
            Ok(()) => return Ok(()),
            Err(error) if proxy.is_none() => {
                return Err(if attempted_proxy {
                    format!("{error}\n已分别尝试代理和直连，均未成功。")
                } else {
                    error
                });
            }
            Err(_) => {}
        }
    }
    Err("Git 仓库克隆失败".into())
}

fn sanitize_repository_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else { return url.into() };
    let scheme = &url[..scheme_end];
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return url.into();
    }
    let remainder = &url[scheme_end + 3..];
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let Some(at) = authority.rfind('@') else { return url.into() };
    format!("{scheme}://{}{}", &authority[at + 1..], &remainder[authority_end..])
}

#[tauri::command]
fn import_plugin(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<PluginImportOutcome, String> {
    let source = fs::canonicalize(path.trim()).map_err(|e| format!("插件目录不可访问：{e}"))?;
    if !source.is_dir() {
        return Err("请选择插件目录".into());
    }
    let origin = PluginOrigin { kind: "local".into(), local_path: Some(source.to_string_lossy().to_string()), repository: None };
    prepare_plugin_import(&app, &state, source, origin, None)
}

#[tauri::command(async)]
fn import_git_plugin(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<PluginImportOutcome, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("请输入 Git 仓库地址".into());
    }
    let checkout = temporary_path(&state, "plugin-import")?;
    clone_git_repository(&state, url, &checkout)?;
    let origin = PluginOrigin { kind: "git".into(), local_path: None, repository: Some(sanitize_repository_url(url)) };
    prepare_plugin_import(&app, &state, checkout.clone(), origin, Some(checkout))
}

fn archive_download_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches('/');
    if !matches!(trimmed.split("://").next(), Some("http" | "https")) {
        return Err("压缩包地址必须以 http:// 或 https:// 开头".into());
    }
    let base = trimmed.strip_suffix(".git").unwrap_or(trimmed).trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with(".zip") {
        return Ok(base.into());
    }
    if base.split("://").nth(1).is_some_and(|rest| rest.starts_with("github.com/")) {
        return Ok(format!("{base}/archive/HEAD.zip"));
    }
    Err("非 GitHub 地址请直接提供以 .zip 结尾的压缩包下载链接".into())
}

fn locate_archive_plugin_root(extracted: &Path) -> Result<PathBuf, String> {
    if extracted.join(MANIFEST_FILE).is_file() {
        return Ok(extracted.to_path_buf());
    }
    let mut candidates = Vec::new();
    let mut directories = Vec::new();
    let mut has_files = false;
    for entry in fs::read_dir(extracted).map_err(|e| format!("读取压缩包目录失败：{e}"))?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path.clone());
            if path.join(MANIFEST_FILE).is_file() {
                candidates.push(path);
            }
        } else {
            has_files = true;
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 if directories.len() == 1 && !has_files => Ok(directories.remove(0)),
        0 => Ok(extracted.to_path_buf()),
        _ => Err("压缩包中包含多个插件，请改用单独插件的压缩包或本地目录导入".into()),
    }
}

fn download_archive(download_url: &str, archive_path: &Path, proxy: Option<&str>) -> Result<(), String> {
    let mut agent = ureq::AgentBuilder::new();
    if let Some(proxy) = proxy {
        let proxy = ureq::Proxy::new(proxy).map_err(|e| format!("代理配置无效：{e}"))?;
        agent = agent.proxy(proxy);
    }
    let response = agent.build().get(download_url)
        .timeout(std::time::Duration::from_secs(120))
        .call()
        .map_err(|e| format!("下载插件压缩包失败：{e}"))?;
    let mut reader = response.into_reader();
    let mut file = fs::File::create(archive_path).map_err(|e| format!("创建压缩包缓存失败：{e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("保存插件压缩包失败：{e}"))?;
    Ok(())
}

/// 下载 ZIP 并解压，返回（临时目录，插件根目录）。调用方负责清理临时目录。
fn fetch_archive_plugin(state: &AppState, url: &str) -> Result<(PathBuf, PathBuf), String> {
    let download_url = archive_download_url(url)?;
    let temp = temporary_path(state, "plugin-archive")?;
    fs::create_dir_all(&temp).map_err(|e| format!("创建临时导入目录失败：{e}"))?;
    let result = (|| {
        let archive_path = temp.join("plugin.zip");
        let candidates = proxy_candidates(state);
        let attempted_proxy = candidates.len() > 1;
        for proxy in candidates {
            match download_archive(&download_url, &archive_path, proxy.as_deref()) {
                Ok(()) => break,
                Err(_) if proxy.is_some() => {}
                Err(error) => {
                    return Err(if attempted_proxy {
                        format!("{error}\n已分别尝试代理和直连，均未成功。")
                    } else {
                        error
                    });
                }
            }
        }

        let extracted = temp.join("extracted");
        fs::create_dir_all(&extracted).map_err(|e| format!("创建解压目录失败：{e}"))?;
        let file = fs::File::open(&archive_path).map_err(|e| format!("读取插件压缩包失败：{e}"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("压缩包格式无效：{e}"))?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| format!("读取压缩包内容失败：{e}"))?;
            let Some(relative) = entry.enclosed_name().map(|path| path.to_path_buf()) else { continue };
            let target = extracted.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|e| format!("创建解压目录失败：{e}"))?;
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建解压目录失败：{e}"))?;
            }
            let mut output = fs::File::create(&target).map_err(|e| format!("写入解压文件失败：{e}"))?;
            std::io::copy(&mut entry, &mut output).map_err(|e| format!("解压插件失败：{e}"))?;
        }
        locate_archive_plugin_root(&extracted)
    })();
    match result {
        Ok(root) => Ok((temp, root)),
        Err(error) => {
            let _ = fs::remove_dir_all(&temp);
            Err(error)
        }
    }
}

#[tauri::command(async)]
fn import_archive_plugin(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<PluginImportOutcome, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("请输入插件压缩包地址或 GitHub 仓库地址".into());
    }
    let (temp, source) = fetch_archive_plugin(&state, url)?;
    let origin = PluginOrigin { kind: "archive".into(), local_path: None, repository: Some(sanitize_repository_url(url)) };
    prepare_plugin_import(&app, &state, source, origin, Some(temp))
}

fn replace_plugin_from_source(app: &AppHandle, state: &AppState, plugin: &PluginView, source: &Path) -> Result<PluginView, String> {
    let source_has_manifest = source.join(MANIFEST_FILE).is_file();
    let manifest = if source_has_manifest { read_manifest(source)? } else { plugin.manifest.clone() };
    ensure_plugin_compatible(source, &manifest)?;
    if manifest.id != plugin.manifest.id {
        return Err(format!("更新来源的插件 id 为「{}」，与当前插件「{}」不一致", manifest.id, plugin.manifest.id));
    }
    let managed = PathBuf::from(&plugin.root);
    let canonical_source = fs::canonicalize(source).map_err(|e| format!("更新来源不可访问：{e}"))?;
    let canonical_managed = fs::canonicalize(&managed).map_err(|e| format!("当前插件目录不可访问：{e}"))?;
    if canonical_source == canonical_managed || canonical_source.starts_with(&canonical_managed) || canonical_managed.starts_with(&canonical_source) {
        return Err("更新来源与当前插件目录重叠，已取消更新".into());
    }
    let managed_parent = managed.parent().ok_or_else(|| "当前插件目录无效".to_string())?;
    let staging_root = managed_parent.join(".office-toolbox-tmp");
    fs::create_dir_all(&staging_root).map_err(|e| format!("创建更新临时目录失败：{e}"))?;
    let unique = temporary_path(state, "plugin-update")?;
    let name = unique.file_name().ok_or_else(|| "生成更新临时目录失败".to_string())?;
    let staging = staging_root.join(name);
    let backup = staging_root.join(format!("{}-backup", name.to_string_lossy()));
    let result = copy_plugin_directory(&canonical_source, &staging)
        .and_then(|_| copy_declared_executables(&manifest, &canonical_source, &staging))
        .and_then(|_| if source_has_manifest { Ok(()) } else { write_generated_manifest(&staging, &manifest) });
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if let Err(error) = stop_tracked(state, &plugin.manifest.id) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(&managed, &backup) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("备份旧插件失败：{error}"));
    }
    if let Err(error) = fs::rename(&staging, &managed) {
        let _ = fs::remove_dir_all(&staging);
        if let Err(restore_error) = fs::rename(&backup, &managed) {
            return Err(format!("替换插件失败：{error}；恢复旧版本也失败：{restore_error}。旧版本备份保留在 {}", backup.display()));
        }
        return Err(format!("替换插件失败，已恢复旧版本：{error}"));
    }
    let _ = fs::remove_dir_all(&backup);
    set_runtime(state, &plugin.manifest.id, PluginRuntime::new("stopped", None, "插件已更新"));
    find_plugin(app, state, &plugin.manifest.id)
}

#[tauri::command(async)]
fn update_plugin(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<PluginView, String> {
    let plugin = find_plugin(&app, &state, &id)?;
    if plugin.source != "imported" {
        return Err("内置插件随工具箱发布，不能单独更新".into());
    }
    if plugin.runtime.status == "detached" {
        return Err("请先退出独立运行的插件，再执行更新".into());
    }
    let origin = plugin.origin.clone().ok_or_else(|| "未记录插件来源，无法更新；请移除后重新导入".to_string())?;
    match origin.kind.as_str() {
        "local" => {
            let source = origin.local_path.as_deref().ok_or_else(|| "本地更新来源记录不完整".to_string())?;
            replace_plugin_from_source(&app, &state, &plugin, Path::new(source))
        }
        "git" => {
            let repository = origin.repository.as_deref().ok_or_else(|| "未记录 Git 仓库地址，无法更新；请移除后重新导入".to_string())?;
            let checkout = temporary_path(&state, "plugin-update-checkout")?;
            clone_git_repository(&state, repository, &checkout)?;
            let result = replace_plugin_from_source(&app, &state, &plugin, &checkout);
            let _ = fs::remove_dir_all(&checkout);
            result
        }
        "archive" => {
            let repository = origin.repository.as_deref().ok_or_else(|| "未记录压缩包地址，无法更新；请移除后重新导入".to_string())?;
            let (temp, source) = fetch_archive_plugin(&state, repository)?;
            let result = replace_plugin_from_source(&app, &state, &plugin, &source);
            let _ = fs::remove_dir_all(&temp);
            result
        }
        _ => Err("未知插件来源，无法更新".into()),
    }
}

#[tauri::command]
fn update_plugin_rules(app: AppHandle, state: State<'_, AppState>, id: String, rules: PluginRules) -> Result<PluginView, String> {
    find_plugin(&app, &state, &id)?;
    if !rules.enabled {
        let _ = stop_tracked(&state, &id)?;
    }
    state.rules.lock().unwrap().insert(id.clone(), rules);
    save_rules(&state)?;
    find_plugin(&app, &state, &id)
}

#[tauri::command(async)]
fn launch_plugin(app: AppHandle, state: State<'_, AppState>, id: String, detached: Option<bool>) -> Result<PluginRuntime, String> {
    launch_by_id(&app, &state, &id, detached)
}

#[tauri::command]
fn stop_plugin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    stop_tracked(&state, &id)?;
    Ok(())
}

fn original_source_for_removal(plugin: &PluginView) -> Result<PathBuf, String> {
    let origin = plugin.origin.as_ref().ok_or_else(|| "未记录此插件的原始本地目录，不能删除原项目".to_string())?;
    if origin.kind != "local" {
        return Err("只有本地项目导入的插件支持删除原项目目录".into());
    }
    let local_path = origin.local_path.as_deref().ok_or_else(|| "原始本地目录记录不完整".to_string())?;
    let source = fs::canonicalize(local_path).map_err(|e| format!("原始本地目录不可访问：{e}"))?;
    let managed = fs::canonicalize(&plugin.root).map_err(|e| format!("插件目录不可访问：{e}"))?;
    if !source.is_dir() || source.parent().is_none() || source == managed || source.starts_with(&managed) || managed.starts_with(&source) {
        return Err("原始本地目录不安全，已取消删除".into());
    }
    let source_manifest = read_manifest(&source).map_err(|e| format!("无法核对原始项目，已取消删除：{e}"))?;
    if source_manifest.id != plugin.manifest.id {
        return Err("原始目录中的插件 id 已变化，已取消删除".into());
    }
    Ok(source)
}

#[tauri::command]
fn remove_plugin(app: AppHandle, state: State<'_, AppState>, id: String, delete_source: Option<bool>) -> Result<(), String> {
    let plugin = find_plugin(&app, &state, &id)?;
    if plugin.source != "imported" {
        return Err("内置插件只能停用，不能移除".into());
    }
    let original = delete_source.unwrap_or(false).then(|| original_source_for_removal(&plugin)).transpose()?;
    stop_tracked(&state, &id)?;
    state.detached.lock().unwrap().remove(&id);
    fs::remove_dir_all(&plugin.root).map_err(|e| format!("移除插件失败：{e}"))?;
    let source_result = original.map_or(Ok(()), |source| {
        fs::remove_dir_all(&source).map_err(|e| format!("插件已移除，但原始本地目录删除失败：{e}"))
    });
    state.rules.lock().unwrap().remove(&id);
    state.origins.lock().unwrap().remove(&id);
    state.runtimes.lock().unwrap().remove(&id);
    let metadata_result = save_origins(&state).and_then(|_| save_rules(&state));
    source_result?;
    metadata_result
}

fn extract_declared_executables(plugin: &PluginView, destination: &Path) -> Result<ExtractPluginResult, String> {
    let output_dir = destination.join(format!("{}-compiled", plugin.manifest.id));
    let mut roots = Vec::new();
    if let Some(local_path) = plugin.origin.as_ref().and_then(|origin| origin.local_path.as_deref()) {
        roots.push(PathBuf::from(local_path));
    }
    roots.push(PathBuf::from(&plugin.root));
    let mut files = Vec::new();
    for candidate in &plugin.manifest.entry.executable {
        let candidate = candidate.path();
        let mut source = None;
        for root in &roots {
            let path = safe_join(root, candidate)?;
            if !path.is_file() { continue }
            let canonical = fs::canonicalize(&path).map_err(|e| format!("读取编译产物失败：{e}"))?;
            let canonical_root = fs::canonicalize(root).map_err(|e| format!("读取插件目录失败：{e}"))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(format!("编译产物不能链接到插件目录之外：{candidate}"));
            }
            source = Some(canonical);
            break;
        }
        let Some(source) = source else { continue };
        let target = safe_join(&output_dir, candidate)?;
        let parent = target.parent().ok_or_else(|| "编译产物提取路径无效".to_string())?;
        fs::create_dir_all(parent).map_err(|e| format!("创建提取目录失败：{e}"))?;
        let canonical_output = fs::canonicalize(&output_dir).map_err(|e| format!("读取提取目录失败：{e}"))?;
        let canonical_parent = fs::canonicalize(parent).map_err(|e| format!("读取提取目录失败：{e}"))?;
        if !canonical_output.starts_with(destination) || !canonical_parent.starts_with(&canonical_output) {
            return Err("提取目录不能链接到所选位置之外".into());
        }
        if fs::symlink_metadata(&target).map(|meta| meta.file_type().is_symlink()).unwrap_or(false) {
            return Err(format!("提取目标不能是符号链接：{candidate}"));
        }
        fs::copy(&source, &target).map_err(|e| format!("提取 {} 失败：{e}", source.display()))?;
        files.push(candidate.into());
    }
    if files.is_empty() {
        return Err("未找到清单 entry.executable 声明的编译产物，请先编译插件".into());
    }
    Ok(ExtractPluginResult { output_dir: output_dir.to_string_lossy().to_string(), files })
}

#[tauri::command]
fn extract_plugin(app: AppHandle, state: State<'_, AppState>, id: String, destination: String) -> Result<ExtractPluginResult, String> {
    let plugin = find_plugin(&app, &state, &id)?;
    let destination = fs::canonicalize(destination.trim()).map_err(|e| format!("提取目录不可访问：{e}"))?;
    if !destination.is_dir() {
        return Err("请选择一个文件夹作为提取位置".into());
    }
    extract_declared_executables(&plugin, &destination)
}

// ---------- 主程序应用内更新 ----------

/// 区分 Windows 安装版和便携版。只有卸载项明确指向当前 exe 时才按安装版处理，
/// 避免电脑同时存在安装版时，便携版被错误地更新到另一个目录。
#[tauri::command]
fn update_install_kind() -> String {
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};

        let Ok(exe) = std::env::current_exe() else { return "portable".into() };
        let exe_text = exe.to_string_lossy().to_lowercase();
        let exe_dir = exe.parent();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for name in ["办公工具箱", "office-toolbox", "com.office.toolbox"] {
                let path = format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{name}");
                let Ok(key) = RegKey::predef(hive).open_subkey(path) else { continue };
                let executable_matches = ["DisplayIcon", "UninstallString"].iter().any(|value_name| {
                    key.get_value::<String, _>(value_name).map(|value| {
                        let value = value.trim().trim_matches('"').to_lowercase();
                        value.strip_prefix(&exe_text).is_some_and(|suffix| {
                            suffix.is_empty() || suffix.starts_with([',', '"', ' ', '\t'])
                        })
                    }).unwrap_or(false)
                });
                let location_matches = key.get_value::<String, _>("InstallLocation").ok().as_deref().map(Path::new)
                    .zip(exe_dir)
                    .is_some_and(|(install_dir, current_dir)| install_dir == current_dir);
                if executable_matches || location_matches {
                    return "installed".into();
                }
            }
        }
        "portable".into()
    }
    #[cfg(target_os = "macos")]
    {
        "macos".into()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        "installed".into()
    }
}

#[tauri::command]
fn open_release_page() -> Result<(), String> {
    const RELEASES_URL: &str = "https://github.com/duangdangding/my_office_tool/releases/latest";
    #[cfg(windows)]
    let mut command = Command::new("rundll32.exe");
    #[cfg(windows)]
    command.arg("url.dll,FileProtocolHandler");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(RELEASES_URL);
    hide_console(&mut command);
    command.spawn().map(|_| ()).map_err(|e| format!("打开发布页面失败：{e}"))
}

#[tauri::command]
fn open_plugin_directory(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let plugin = find_plugin(&app, &state, &id)?;
    #[cfg(windows)]
    let mut command = Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(&plugin.root);
    hide_console(&mut command);
    command.spawn().map(|_| ()).map_err(|e| format!("打开插件目录失败：{e}"))
}

fn read_readme(root: &Path) -> Result<String, String> {
    let readme = fs::read_dir(root)
        .map_err(|error| format!("读取插件目录失败：{error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("readme.md"))
                && path.is_file()
        })
        .ok_or_else(|| "此插件没有 README.md。".to_string())?;
    let metadata = fs::metadata(&readme).map_err(|error| format!("读取 README.md 信息失败：{error}"))?;
    if metadata.len() > 2 * 1024 * 1024 {
        return Err("README.md 超过 2 MB，请打开插件目录查看。".into());
    }
    fs::read_to_string(&readme).map_err(|error| format!("读取 README.md 失败：{error}"))
}

#[tauri::command]
fn read_plugin_readme(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<String, String> {
    let plugin = find_plugin(&app, &state, &id)?;
    read_readme(Path::new(&plugin.root))
}

#[tauri::command]
fn open_plugin_page(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let plugin = find_plugin(&app, &state, &id)?;
    let web = plugin.manifest.entry.web.as_ref().ok_or_else(|| "此插件没有配置 Web 页面地址。".to_string())?;
    open_web_url(&web.url)
}

#[tauri::command]
fn read_plugin_log(state: State<'_, AppState>, id: String) -> String {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_')) {
        return "插件 id 无效。".into();
    }
    let text = read_log(&state.data_dir.join("logs").join(format!("{id}.log")));
    if text.trim().is_empty() { "暂无运行日志。".into() } else { text }
}

fn stop_all(state: &AppState) {
    let ids: Vec<String> = state.running.lock().unwrap().keys().cloned().collect();
    for id in ids {
        let _ = stop_tracked(state, &id);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = default_data_dir();
    let settings = load_settings(&data_dir);
    let plugin_dir = settings.plugin_dir.clone().unwrap_or_else(|| data_dir.join("plugins"));
    let silent_start = settings.silent_start;
    let state = AppState {
        rules: Mutex::new(load_rules(&data_dir)),
        origins: Mutex::new(load_origins(&data_dir)),
        plugin_dir: Mutex::new(plugin_dir),
        settings: Mutex::new(settings),
        data_dir,
        runtimes: Mutex::new(HashMap::new()),
        running: Mutex::new(HashMap::new()),
        detached: Mutex::new(HashMap::new()),
        pending_imports: Mutex::new(HashMap::new()),
    };
    let app = tauri::Builder::default()
        // 单实例插件必须最先注册；重复打开时唤起已有窗口，不再创建第二个主程序。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            list_plugins,
            get_plugin_directory,
            set_plugin_directory,
            import_plugin,
            import_git_plugin,
            import_archive_plugin,
            complete_plugin_import,
            cancel_plugin_import,
            get_settings,
            set_launch_at_startup,
            set_silent_start,
            set_theme,
            set_proxy,
            update_plugin,
            update_plugin_rules,
            launch_plugin,
            stop_plugin,
            remove_plugin,
            extract_plugin,
            update_install_kind,
            open_release_page,
            open_plugin_directory,
            read_plugin_readme,
            open_plugin_page,
            read_plugin_log,
        ])
        .setup(move |app| {
            let show = MenuItem::with_id(app, "show", "显示办公工具箱", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "隐藏窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("办公工具箱")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // 关闭按钮只隐藏窗口；从托盘「退出」才会结束主程序和托管插件。
            if let Some(window) = app.get_webview_window("main") {
                let window_handle = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_handle.hide();
                    }
                });
            }

            // 窗口默认不显示，避免静默启动时闪现；普通启动在这里主动显示。
            if silent_start {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            } else {
                show_main_window(app.handle());
            }

            let state = app.state::<AppState>();
            let ids: Vec<String> = discover_plugins(app.handle(), &state)
                .into_iter()
                .filter(|plugin| plugin.rules.enabled && plugin.rules.auto_start)
                .map(|plugin| plugin.manifest.id)
                .collect();
            for id in ids {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let state = handle.state::<AppState>();
                    let _ = launch_by_id(&handle, &state, &id, None);
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("构建办公工具箱失败");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            stop_all(&handle.state::<AppState>());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        serde_json::from_str(r#"{
            "schemaVersion": 1,
            "id": "demo-tool",
            "name": "演示插件",
            "version": "1.0.0",
            "description": "测试",
            "entry": {"mode": "dev", "dev": {"program": "node", "args": ["index.js"]}},
            "standalone": {"instructions": "node index.js"}
        }"#).unwrap()
    }

    fn executable_variant(path: &str, platforms: &[&str], architectures: &[&str]) -> PluginExecutable {
        PluginExecutable::Variant(PluginExecutableVariant {
            path: path.into(),
            platforms: platforms.iter().map(|item| (*item).into()).collect(),
            architectures: architectures.iter().map(|item| (*item).into()).collect(),
        })
    }

    #[test]
    fn accepts_valid_manifest() {
        assert!(validate_manifest(&manifest()).is_ok());
    }

    #[test]
    fn import_outcome_serializes_wizard_fields_in_camel_case() {
        let outcome = PluginImportOutcome::NeedsManifest {
            session_id: "session-1".into(),
            source: "C:\\plugins\\LibreTV".into(),
            draft: manifest(),
            executable_candidates: vec!["bin/tool.exe".into()],
            platform: "windows".into(),
            architecture: "x86_64".into(),
        };
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value.get("status").and_then(serde_json::Value::as_str), Some("needsManifest"));
        assert_eq!(value.get("sessionId").and_then(serde_json::Value::as_str), Some("session-1"));
        assert_eq!(value.get("executableCandidates").and_then(serde_json::Value::as_array).map(Vec::len), Some(1));
        assert!(value.get("session_id").is_none());
        assert!(value.get("executable_candidates").is_none());
    }

    #[test]
    fn manifest_draft_scans_entry_and_only_writes_to_import_copy() {
        let base = test_dir("manifest-draft");
        let source = base.join("My Tool");
        fs::create_dir_all(&source).unwrap();
        let executable_name = if cfg!(windows) { "tool.exe" } else { "tool" };
        let executable = source.join(executable_name);
        fs::write(&executable, "program").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(source.join("unins000.exe"), "uninstaller").unwrap();
        fs::write(source.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();

        assert_eq!(suggested_plugin_id("My Tool"), "my-tool");
        assert_eq!(suggested_plugin_id("演示插件"), "imported-plugin");
        let (draft, candidates) = manifest_draft(&source, "my-tool".into());
        assert_eq!(candidates, vec![executable_name]);
        assert_eq!(draft.entry.mode, "auto");
        assert_eq!(draft.entry.executable[0].path(), executable_name);
        assert_eq!(draft.entry.dev.as_ref().unwrap().args, vec!["run", "dev"]);
        assert!(validate_manifest(&draft).is_ok());
        assert!(ensure_plugin_compatible(&source, &draft).is_ok());
        assert!(!source.join(MANIFEST_FILE).exists());

        let imported = base.join("imported");
        copy_plugin_directory(&source, &imported).unwrap();
        copy_declared_executables(&draft, &source, &imported).unwrap();
        write_generated_manifest(&imported, &draft).unwrap();
        assert!(!source.join(MANIFEST_FILE).exists());
        assert!(imported.join(executable_name).is_file());
        assert_eq!(read_manifest(&imported).unwrap().id, "my-tool");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn manifest_draft_suggests_cargo_development_command() {
        let base = test_dir("cargo-draft");
        fs::write(base.join("Cargo.toml"), "[package]").unwrap();
        let (draft, candidates) = manifest_draft(&base, "cargo-demo".into());
        assert!(candidates.is_empty());
        assert_eq!(draft.entry.mode, "dev");
        assert_eq!(draft.entry.dev.as_ref().unwrap().program, "cargo");
        assert_eq!(draft.entry.dev.as_ref().unwrap().args, vec!["run"]);
        assert!(validate_manifest(&draft).is_ok());
        assert!(ensure_plugin_compatible(&base, &draft).is_ok());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn libretv_draft_installs_dependencies_and_detects_web_readiness() {
        let base = test_dir("libretv-draft");
        fs::write(base.join("package.json"), r#"{
            "name": "libretv",
            "scripts": {"dev": "next dev -p 8080", "build": "next build"}
        }"#).unwrap();
        let (draft, candidates) = manifest_draft(&base, "libretv".into());
        assert!(candidates.is_empty());
        let build = draft.entry.build.as_ref().unwrap();
        assert_eq!(build.program, "npm");
        assert_eq!(build.args, vec!["install", "--no-audit", "--no-fund"]);
        #[cfg(windows)]
        {
            let (program, args) = resolve_command(&build.program, &build.args);
            assert_eq!(program, "cmd.exe");
            assert_eq!(args, vec!["/D", "/S", "/C", "npm", "install", "--no-audit", "--no-fund"]);
        }
        let dev = draft.entry.dev.as_ref().unwrap();
        assert_eq!(dev.env.get("HOSTNAME").map(String::as_str), Some("127.0.0.1"));
        assert_eq!(dev.env.get("PASSWORD").map(String::as_str), Some("libretv"));
        assert_eq!(draft.entry.web.as_ref().unwrap().url, "http://127.0.0.1:8080");
        assert!(validate_manifest(&draft).is_ok());
        let serialized = serde_json::to_string(&draft).unwrap();
        let restored: PluginManifest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.entry.web.unwrap().url, "http://127.0.0.1:8080");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn web_readiness_accepts_http_response_and_rejects_closed_port() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert!(web_is_ready(&format!("http://{address}")));
        server.join().unwrap();
        assert!(!web_is_ready(&format!("http://{address}")));
    }

    #[test]
    fn readme_supports_common_casing_and_reports_missing_file() {
        let base = test_dir("plugin-readme");
        assert!(read_readme(&base).unwrap_err().contains("没有 README.md"));
        fs::write(base.join("README.md"), "# 插件详情\n\n使用说明").unwrap();
        assert_eq!(read_readme(&base).unwrap(), "# 插件详情\n\n使用说明");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn archive_without_manifest_uses_single_wrapped_directory() {
        let base = test_dir("archive-draft");
        let wrapped = base.join("wrapped");
        let project = wrapped.join("project-main");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("program.txt"), "content").unwrap();
        assert_eq!(locate_archive_plugin_root(&wrapped).unwrap(), project);

        let direct = base.join("direct");
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join("program.txt"), "content").unwrap();
        assert_eq!(locate_archive_plugin_root(&direct).unwrap(), direct);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cancelling_manifest_wizard_removes_pending_source() {
        let base = test_dir("cancel-import");
        let checkout = base.join("checkout");
        fs::create_dir_all(&checkout).unwrap();
        let state = AppState {
            data_dir: base.join("data"),
            plugin_dir: Mutex::new(base.join("plugins")),
            settings: Mutex::new(StoredSettings::default()),
            rules: Mutex::new(HashMap::new()),
            origins: Mutex::new(HashMap::new()),
            runtimes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
            pending_imports: Mutex::new(HashMap::new()),
        };
        state.pending_imports.lock().unwrap().insert(
            "session".into(),
            PendingPluginImport {
                source: checkout.clone(),
                origin: PluginOrigin { kind: "git".into(), local_path: None, repository: None },
                cleanup: Some(checkout.clone()),
            },
        );

        discard_pending_import(&state, "session");

        assert!(!checkout.exists());
        assert!(state.pending_imports.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn accepts_legacy_and_targeted_executable_entries() {
        let legacy: PluginExecutable = serde_json::from_str(r#""bin/legacy-tool""#).unwrap();
        let targeted: PluginExecutable = serde_json::from_str(
            r#"{"path":"bin/macos-arm64","platforms":["macos"],"architectures":["aarch64"]}"#,
        )
        .unwrap();
        assert_eq!(legacy.path(), "bin/legacy-tool");
        assert!(legacy.matches_target("windows", "x86_64"));
        assert!(targeted.matches_target("macos", "aarch64"));
        assert!(!targeted.matches_target("macos", "x86_64"));
        assert!(!targeted.matches_target("windows", "aarch64"));

        let mut value = manifest();
        value.entry.executable = vec![legacy, targeted];
        assert!(validate_manifest(&value).is_ok());
    }

    #[test]
    fn rejects_unknown_executable_target() {
        let mut value = manifest();
        value.entry.executable = vec![executable_variant("bin/tool", &["linux"], &["x86_64"])];
        assert!(validate_manifest(&value).is_err());

        value.entry.executable = vec![executable_variant("bin/tool", &["windows"], &["arm64"])];
        assert!(validate_manifest(&value).is_err());
    }

    #[test]
    fn rejects_unsafe_plugin_id() {
        let mut value = manifest();
        value.id = "../escape".into();
        assert!(validate_manifest(&value).is_err());
    }

    #[test]
    fn rejects_parent_directory_in_entry() {
        let mut value = manifest();
        value.entry.dev.as_mut().unwrap().cwd = "../outside".into();
        assert!(validate_manifest(&value).is_err());
    }

    #[test]
    fn rules_default_to_managed_and_enabled() {
        let rules = PluginRules::default();
        assert!(rules.enabled);
        assert!(!rules.detach);
        assert!(!rules.auto_start);
        assert!(!rules.allow_plugin_autostart);
    }

    #[test]
    fn ready_marker_must_match_exact_plugin() {
        assert!(output_is_ready("Compiling demo\nOFFICE_TOOLBOX_READY:demo-tool\n", "demo-tool"));
        assert!(!output_is_ready("OFFICE_TOOLBOX_READY:other\n", "demo-tool"));
        assert!(!output_is_ready("prefix OFFICE_TOOLBOX_READY:demo-tool\n", "demo-tool"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_builtin_plugins_prefer_current_dev_command() {
        let mut manifest = manifest();
        manifest.entry.mode = "auto".into();
        manifest.entry.executable = vec!["Cargo.toml".into()];
        let plugin = PluginView {
            manifest,
            root: env!("CARGO_MANIFEST_DIR").into(),
            source: "builtin".into(),
            origin: None,
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };
        let (_, args, _, dev_mode) = prepare_launch(&plugin).unwrap();
        assert!(dev_mode);
        assert_eq!(args, vec!["index.js"]);
    }

    #[test]
    fn launch_selects_executable_for_current_platform_and_architecture() {
        let base = test_dir("targeted-launch");
        for file in ["wrong-platform", "wrong-architecture", "current"] {
            fs::write(base.join(file), file).unwrap();
        }
        let other_platform = if current_platform() == "windows" { "macos" } else { "windows" };
        let other_architecture = if current_architecture() == "x86_64" { "aarch64" } else { "x86_64" };
        let mut manifest = manifest();
        manifest.entry.mode = "executable".into();
        manifest.entry.dev = None;
        manifest.entry.executable = vec![
            executable_variant("wrong-platform", &[other_platform], &[current_architecture()]),
            executable_variant("wrong-architecture", &[current_platform()], &[other_architecture]),
            executable_variant("current", &[current_platform()], &[current_architecture()]),
        ];
        let plugin = PluginView {
            manifest,
            root: base.to_string_lossy().to_string(),
            source: "imported".into(),
            origin: None,
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };

        let (program, _, _, dev_mode) = prepare_launch(&plugin).unwrap();
        assert_eq!(PathBuf::from(program), base.join("current"));
        assert!(!dev_mode);
        assert!(ensure_plugin_compatible(&base, &plugin.manifest).is_ok());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    #[ignore = "需要先按 README 构建当前平台的多架构示例插件"]
    fn multi_arch_example_copies_and_launches() {
        use std::io::{BufRead, BufReader};
        use std::sync::mpsc;
        use std::time::Duration;

        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples")
            .join("multi-arch-plugin");
        let source_manifest = read_manifest(&example).unwrap();
        ensure_plugin_compatible(&example, &source_manifest).unwrap();

        let base = test_dir("multi-arch-example");
        let source = base.join("source");
        copy_plugin_directory(&example, &source).unwrap();
        copy_declared_executables(&source_manifest, &example, &source).unwrap();
        fs::remove_file(source.join(MANIFEST_FILE)).unwrap();
        let (generated, candidates) = manifest_draft(&source, "multi-arch-plugin".into());
        assert!(!candidates.is_empty());
        assert!(!source.join(MANIFEST_FILE).exists());
        ensure_plugin_compatible(&source, &generated).unwrap();

        let imported = base.join("imported");
        copy_plugin_directory(&source, &imported).unwrap();
        copy_declared_executables(&generated, &source, &imported).unwrap();
        write_generated_manifest(&imported, &generated).unwrap();
        let manifest = read_manifest(&imported).unwrap();
        ensure_plugin_compatible(&imported, &manifest).unwrap();
        let plugin = PluginView {
            manifest,
            root: imported.to_string_lossy().to_string(),
            source: "imported".into(),
            origin: None,
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };
        let (program, args, cwd, dev_mode) = prepare_launch(&plugin).unwrap();
        assert!(!dev_mode);
        assert!(PathBuf::from(&program).starts_with(&imported));

        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut output = Vec::new();
        let ready = loop {
            match receiver.recv_timeout(Duration::from_secs(10)) {
                Ok(line) => {
                    let matched = output_is_ready(&line, "multi-arch-plugin");
                    output.push(line);
                    if matched {
                        break true;
                    }
                }
                Err(_) => break false,
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        reader.join().unwrap();

        assert!(ready, "示例插件未报告就绪：{}", output.join("\n"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn process_errors_become_persistent_failed_state() {
        let state = AppState {
            data_dir: PathBuf::from("."),
            plugin_dir: Mutex::new(PathBuf::from(".")),
            settings: Mutex::new(StoredSettings::default()),
            rules: Mutex::new(HashMap::new()),
            origins: Mutex::new(HashMap::new()),
            runtimes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
        pending_imports: Mutex::new(HashMap::new()),
        };
        let runtime = exited_runtime(&state, "demo-tool", Err("编译进程读取失败".into()), true, false);
        assert_eq!(runtime.status, "failed");
        assert_eq!(runtime_for(&state, "demo-tool").detail.as_deref(), Some("编译进程读取失败"));
    }

    fn test_dir(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-data")
            .join(format!("{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn copies_declared_executable_without_whole_build_cache() {
        let base = test_dir("copy-executable");
        let source = base.join("source");
        let target = base.join("target");
        let release = source.join("src-tauri").join("target").join("release");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("demo.exe"), "program").unwrap();
        fs::write(release.join("cache.obj"), "cache").unwrap();
        let mut manifest = manifest();
        manifest.entry.executable = vec!["src-tauri/target/release/demo.exe".into()];

        copy_declared_executables(&manifest, &source, &target).unwrap();

        assert_eq!(fs::read_to_string(target.join("src-tauri/target/release/demo.exe")).unwrap(), "program");
        assert!(!target.join("src-tauri/target/release/cache.obj").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn extracts_latest_original_executable_with_relative_path() {
        let base = test_dir("extract-executable");
        let original = base.join("original");
        let managed = base.join("managed");
        let destination = base.join("output");
        let relative = "src-tauri/target/release/demo.exe";
        for (root, contents) in [(&original, "new"), (&managed, "old")] {
            let release = root.join("src-tauri").join("target").join("release");
            fs::create_dir_all(&release).unwrap();
            fs::write(release.join("demo.exe"), contents).unwrap();
        }
        fs::create_dir_all(&destination).unwrap();
        let mut manifest = manifest();
        manifest.entry.executable = vec![relative.into()];
        let plugin = PluginView {
            manifest,
            root: managed.to_string_lossy().to_string(),
            source: "imported".into(),
            origin: Some(PluginOrigin { kind: "local".into(), local_path: Some(original.to_string_lossy().to_string()), repository: None }),
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };

        let result = extract_declared_executables(&plugin, &fs::canonicalize(&destination).unwrap()).unwrap();

        assert_eq!(result.files, vec![relative]);
        assert_eq!(fs::read_to_string(destination.join("demo-tool-compiled").join(relative)).unwrap(), "new");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn refuses_to_delete_original_when_manifest_id_changed() {
        let base = test_dir("remove-origin");
        let source = base.join("source");
        let managed = base.join("managed");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&managed).unwrap();
        let mut changed = manifest();
        changed.id = "another-plugin".into();
        fs::write(source.join(MANIFEST_FILE), serde_json::to_vec_pretty(&changed).unwrap()).unwrap();
        let plugin = PluginView {
            manifest: manifest(),
            root: managed.to_string_lossy().to_string(),
            source: "imported".into(),
            origin: Some(PluginOrigin { kind: "local".into(), local_path: Some(source.to_string_lossy().to_string()), repository: None }),
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };

        assert!(original_source_for_removal(&plugin).is_err());
        assert!(source.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn compatibility_rejects_unsupported_platform_and_missing_entrypoint() {
        let base = test_dir("compatibility");
        let mut value = manifest();
        value.platforms = vec![if current_platform() == "windows" { "macos".into() } else { "windows".into() }];
        assert!(ensure_plugin_compatible(&base, &value).is_err());

        value.platforms.clear();
        value.entry.mode = "executable".into();
        value.entry.executable = vec!["missing.exe".into()];
        value.entry.dev = None;
        assert!(ensure_plugin_compatible(&base, &value).is_err());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn git_clone_accepts_precreated_unicode_work_tree() {
        let base = test_dir("git-unicode-checkout");
        let source = base.join("source.git");
        let output = Command::new("git")
            .args(["init", "--bare", "--"])
            .arg(&source)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let checkout = base.join("办公工具箱").join(".office-toolbox-tmp").join("plugin-import-测试");
        clone_git_repository_once(&source.to_string_lossy(), &checkout, None).unwrap();
        assert!(checkout.join(".git").is_dir());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn git_repository_metadata_does_not_keep_http_userinfo() {
        assert_eq!(sanitize_repository_url("https://user:secret@example.com/owner/plugin.git"), "https://example.com/owner/plugin.git");
        assert_eq!(sanitize_repository_url("git@example.com:owner/plugin.git"), "git@example.com:owner/plugin.git");
    }

    #[test]
    fn legacy_settings_use_light_theme() {
        let settings: StoredSettings = serde_json::from_str("{}").unwrap();
        assert!(matches!(settings.theme, AppTheme::Light));
        assert!(matches!(StoredSettings::default().theme, AppTheme::Light));
    }

    #[test]
    fn custom_plugin_directory_is_persisted() {
        let base = test_dir("settings");
        let custom = base.join("custom-plugins");
        fs::create_dir_all(&custom).unwrap();
        let state = AppState {
            data_dir: base.clone(),
            plugin_dir: Mutex::new(base.join("plugins")),
            settings: Mutex::new(StoredSettings::default()),
            rules: Mutex::new(HashMap::new()),
            origins: Mutex::new(HashMap::new()),
            runtimes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
            pending_imports: Mutex::new(HashMap::new()),
        };

        state.settings.lock().unwrap().theme = AppTheme::System;
        save_plugin_dir(&state, &custom).unwrap();
        *state.plugin_dir.lock().unwrap() = custom.clone();
        let temporary = temporary_path(&state, "plugin-import").unwrap();
        assert!(temporary.starts_with(custom.join(".office-toolbox-tmp")));
        assert!(!temporary.starts_with(base.join("tmp")));
        assert_eq!(load_plugin_dir(&base), custom);
        assert!(matches!(load_settings(&base).theme, AppTheme::System));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn custom_proxy_is_validated_normalized_and_persisted() {
        let base = test_dir("proxy-settings");
        let state = AppState {
            data_dir: base.clone(),
            plugin_dir: Mutex::new(base.join("plugins")),
            settings: Mutex::new(StoredSettings::default()),
            rules: Mutex::new(HashMap::new()),
            origins: Mutex::new(HashMap::new()),
            runtimes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
            pending_imports: Mutex::new(HashMap::new()),
        };

        save_proxy(&state, "  127.0.0.1:7890  ").unwrap();
        assert_eq!(state.settings.lock().unwrap().proxy, "http://127.0.0.1:7890");
        assert_eq!(load_settings(&base).proxy, "http://127.0.0.1:7890");
        assert!(save_proxy(&state, "http://bad proxy").is_err());
        assert_eq!(load_settings(&base).proxy, "http://127.0.0.1:7890");

        save_proxy(&state, "").unwrap();
        assert!(load_settings(&base).proxy.is_empty());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn proxy_candidates_prefer_custom_or_local_proxy_then_direct() {
        let custom = Some("http://127.0.0.1:7890".to_string());
        let local = Some("http://127.0.0.1:8080".to_string());
        assert_eq!(
            proxy_candidates_from("http://127.0.0.1:7890", local.clone()),
            vec![custom.clone(), None]
        );
        assert_eq!(proxy_candidates_from("", local.clone()), vec![local, None]);
        assert_eq!(proxy_candidates_from("", None), vec![None]);
        assert_eq!(proxy_candidates_from("  ", custom.clone()), vec![custom, None]);
    }

    #[test]
    fn direct_commands_remove_inherited_proxy_variables() {
        let mut command = Command::new("test");
        command.env("HTTPS_PROXY", "http://127.0.0.1:7890");
        apply_command_proxy(&mut command, None);
        assert!(command.get_envs().any(|(name, value)| name == "HTTPS_PROXY" && value.is_none()));

        apply_command_proxy(&mut command, Some("http://127.0.0.1:8080"));
        assert!(command.get_envs().any(|(name, value)| {
            name == "HTTPS_PROXY" && value.is_some_and(|value| value == "http://127.0.0.1:8080")
        }));
    }

    #[test]
    fn parses_windows_proxy_and_prefers_https_endpoint() {
        assert_eq!(parse_windows_proxy_server("127.0.0.1:7890").as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(
            parse_windows_proxy_server("http=127.0.0.1:8080;https=127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            parse_windows_proxy_server("socks=127.0.0.1:1080;http=127.0.0.1:8080").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(parse_windows_proxy_server(""), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_macos_system_proxy() {
        let output = "HTTPEnable : 1\nHTTPProxy : 127.0.0.1\nHTTPPort : 8080\nHTTPSEnable : 1\nHTTPSProxy : 127.0.0.1\nHTTPSPort : 7890";
        assert_eq!(parse_macos_proxy(output).as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn github_repository_converts_to_archive_url_without_git() {
        assert_eq!(
            archive_download_url("https://github.com/owner/plugin.git").unwrap(),
            "https://github.com/owner/plugin/archive/HEAD.zip"
        );
        assert_eq!(
            archive_download_url("https://github.com/owner/plugin/").unwrap(),
            "https://github.com/owner/plugin/archive/HEAD.zip"
        );
        assert_eq!(
            archive_download_url("https://example.com/plugin.zip").unwrap(),
            "https://example.com/plugin.zip"
        );
        assert!(archive_download_url("git@github.com:owner/plugin.git").is_err());
    }
}
