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
    AppHandle, Manager, State,
};

const MANIFEST_FILE: &str = "office-plugin.json";
const RULES_FILE: &str = "rules.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginCommand {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "current_dir")]
    cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginEntry {
    #[serde(default = "auto_mode")]
    mode: String,
    #[serde(default)]
    executable: Vec<String>,
    dev: Option<PluginCommand>,
    #[allow(dead_code)]
    build: Option<PluginCommand>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginView {
    manifest: PluginManifest,
    root: String,
    source: String,
    rules: PluginRules,
    runtime: PluginRuntime,
}

struct AppState {
    data_dir: PathBuf,
    rules: Mutex<HashMap<String, PluginRules>>,
    runtimes: Mutex<HashMap<String, PluginRuntime>>,
    running: Mutex<HashMap<String, Child>>,
    detached: Mutex<HashMap<String, Child>>,
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
    for candidate in &manifest.entry.executable {
        safe_join(Path::new(""), candidate)?;
    }
    if let Some(command) = &manifest.entry.dev {
        if command.program.trim().is_empty() {
            return Err("开发启动命令不能为空".into());
        }
        safe_join(Path::new(""), &command.cwd)?;
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

fn plugin_roots(app: &AppHandle, state: &AppState) -> Vec<(PathBuf, String)> {
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
    roots.push((state.data_dir.join("plugins"), "imported".into()));

    let mut seen = HashSet::new();
    roots.retain(|(root, _)| {
        let key = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        seen.insert(key)
    });
    roots
}

fn discover_plugins(app: &AppHandle, state: &AppState) -> Vec<PluginView> {
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
            let runtime = runtime_for(state, &manifest.id);
            plugins.push(PluginView {
                manifest,
                root: path.to_string_lossy().to_string(),
                source: source.clone(),
                rules,
                runtime,
            });
        }
    }
    plugins.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    plugins
}

fn find_plugin(app: &AppHandle, state: &AppState, id: &str) -> Result<PluginView, String> {
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

fn resolve_program(program: &str) -> String {
    #[cfg(windows)]
    if program.eq_ignore_ascii_case("bun") {
        if let Some(home) = dirs::home_dir() {
            let bun = home.join(".bun").join("bin").join("bun.exe");
            if bun.is_file() {
                return bun.to_string_lossy().to_string();
            }
        }
    }
    program.into()
}

fn prepare_launch(plugin: &PluginView) -> Result<(String, Vec<String>, PathBuf, bool), String> {
    let root = PathBuf::from(&plugin.root);
    let entry = &plugin.manifest.entry;
    // 开发宿主中的内置插件始终走 dev 命令，确保配置和源码更新生效，同时复用编译缓存。
    let prefer_dev = cfg!(debug_assertions) && plugin.source == "builtin" && entry.dev.is_some();
    if entry.mode != "dev" && !prefer_dev {
        for candidate in &entry.executable {
            let executable = safe_join(&root, candidate)?;
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
            return Ok((resolve_program(&command.program), command.args.clone(), cwd, true));
        }
    }
    Err("未找到已构建的插件程序；请先构建插件，或提供有效的 dev 启动命令".into())
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

        if plugin_is_ready(&log_path, id) {
            if !state.running.lock().unwrap().contains_key(id) {
                return Err("插件启动已取消。".into());
            }
            return Ok(set_runtime(state, id, PluginRuntime::new("running", Some(pid), "插件已就绪")));
        }
        // 未适配就绪信号的单文件插件保持兼容；开发命令必须等待明确信号。
        if !dev_mode && started.elapsed() >= std::time::Duration::from_secs(2) {
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
fn import_plugin(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<PluginView, String> {
    let source = fs::canonicalize(path.trim()).map_err(|e| format!("插件目录不可访问：{e}"))?;
    if !source.is_dir() {
        return Err("请选择包含 office-plugin.json 的插件目录".into());
    }
    let manifest = read_manifest(&source)?;
    if find_plugin(&app, &state, &manifest.id).is_ok() {
        return Err(format!("插件 id「{}」已存在，请先移除旧插件", manifest.id));
    }
    let imported_root = state.data_dir.join("plugins");
    let target = imported_root.join(&manifest.id);
    if source.starts_with(&target) || target.exists() {
        return Err("目标插件目录已存在或导入路径无效".into());
    }
    fs::create_dir_all(&imported_root).map_err(|e| e.to_string())?;
    if let Err(error) = copy_plugin_directory(&source, &target) {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    find_plugin(&app, &state, &manifest.id)
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

#[tauri::command]
fn remove_plugin(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let plugin = find_plugin(&app, &state, &id)?;
    if plugin.source != "imported" {
        return Err("内置插件只能停用，不能移除".into());
    }
    stop_tracked(&state, &id)?;
    state.detached.lock().unwrap().remove(&id);
    fs::remove_dir_all(&plugin.root).map_err(|e| format!("移除插件失败：{e}"))?;
    state.rules.lock().unwrap().remove(&id);
    save_rules(&state)
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
    let state = AppState {
        rules: Mutex::new(load_rules(&data_dir)),
        data_dir,
        runtimes: Mutex::new(HashMap::new()),
        running: Mutex::new(HashMap::new()),
        detached: Mutex::new(HashMap::new()),
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            list_plugins,
            import_plugin,
            update_plugin_rules,
            launch_plugin,
            stop_plugin,
            remove_plugin,
            open_plugin_directory,
            read_plugin_log,
        ])
        .setup(|app| {
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

    #[test]
    fn accepts_valid_manifest() {
        assert!(validate_manifest(&manifest()).is_ok());
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
            rules: PluginRules::default(),
            runtime: PluginRuntime::stopped(),
        };
        let (_, args, _, dev_mode) = prepare_launch(&plugin).unwrap();
        assert!(dev_mode);
        assert_eq!(args, vec!["index.js"]);
    }

    #[test]
    fn process_errors_become_persistent_failed_state() {
        let state = AppState {
            data_dir: PathBuf::from("."),
            rules: Mutex::new(HashMap::new()),
            runtimes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            detached: Mutex::new(HashMap::new()),
        };
        let runtime = exited_runtime(&state, "demo-tool", Err("编译进程读取失败".into()), true, false);
        assert_eq!(runtime.status, "failed");
        assert_eq!(runtime_for(&state, "demo-tool").detail.as_deref(), Some("编译进程读取失败"));
    }
}
