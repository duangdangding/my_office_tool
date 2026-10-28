use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use pinyin::ToPinyin;
use rand::RngCore;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::{AtomicBool, Ordering}, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

mod office_plugin;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

const ENCRYPTED_GROUP: &str = "__encrypted__";
const DB_FILE: &str = "suqi.db";
const POINTER_FILE: &str = "suqi-data-dir.txt";

/// 默认全局快捷键：Windows 用 Win+`；macOS 的 Cmd+` 与系统「切换窗口」冲突，改用 Alt+Space
#[cfg(target_os = "windows")]
const DEFAULT_HOTKEY: &str = "Win+`";
#[cfg(target_os = "macos")]
const DEFAULT_HOTKEY: &str = "Alt+Space";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_HOTKEY: &str = "Ctrl+`";

// ---------- 数据结构 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LauncherItem {
    id: String,
    name: String,
    /// 搜索关键字：用户自定义，留空时保存为 name
    #[serde(default)]
    keyword: String,
    /// 说明 / 备注：不参与搜索，仅在面板悬停提示中展示
    #[serde(default)]
    description: String,
    target: String,
    /// 来源快捷方式（.lnk）路径：存在时优先经它启动，由系统 Shell 还原 目标+参数+工作目录
    #[serde(default)]
    lnk_path: Option<String>,
    group_id: Option<String>,
    is_encrypted: bool,
    #[serde(default)]
    is_cmd: bool,
    icon_path: Option<String>,
    sort_order: i64,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Group { id: String, name: String, sort_order: i64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordHash { salt: String, hash: String }

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store { items: Vec<LauncherItem>, groups: Vec<Group>, password: Option<PasswordHash> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    hotkey: String,
    autostart: bool,
    silent_start: bool,
    always_on_top: bool,
    remember_size: bool,
    window_width: i32,
    window_height: i32,
    theme: String,
    font_family: String,
    font_size: i32,
    highlight_color: String,
    window_effect: String,
    #[serde(default = "default_view_mode")]
    view_mode: String,
    #[serde(default = "default_encrypt_unlock_mode")]
    encrypt_unlock_mode: String,
    #[serde(default = "default_hotkey_unfocused_behavior")]
    hotkey_unfocused_behavior: String,
    /// 搜索框「直接运行」（仿 Win+R）开关：默认开启，搜索启动项的同时可当运行框用；关闭后仅搜索启动项
    #[serde(default = "default_run_target_enabled")]
    run_target_enabled: bool,
    /// 重新打开面板时是否清空搜索框：默认清空，回到全部条目重新开始
    #[serde(default = "default_clear_search_on_reopen")]
    clear_search_on_reopen: bool,
    config_dir: Option<String>,
    db_dir: Option<String>,
    background: BackgroundConfig,
}

/// 主面板显示方式默认值："card" 卡片 | "list" 列表
fn default_view_mode() -> String { "card".into() }

/// 加密分组解锁策略默认值："always" 每次打开面板都需输入密码 | "session" 仅重启应用才需输入密码
fn default_encrypt_unlock_mode() -> String { "session".into() }

/// 面板已显示但未聚焦时按全局快捷键的行为："front" 置前显示 | "hide" 隐藏面板
fn default_hotkey_unfocused_behavior() -> String { "front".into() }

/// 「直接运行」功能默认开启
fn default_run_target_enabled() -> bool { true }

/// 重新打开面板清空搜索框：默认开启
fn default_clear_search_on_reopen() -> bool { true }

/// 面板背景图设置：选区 / 旋转在设置页用 canvas 烘焙进缓存图（suqi-bg.png），
/// 运行时主面板只按 mode + opacity + scale 用 CSS 应用缓存图
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BackgroundConfig {
    enabled: bool,        // 是否启用面板背景图
    mode: String,         // "stretch" 拉伸铺满 | "tile" 原尺寸平铺
    opacity: f64,         // 背景图不透明度 0.05 - 1.0
    scale_w: f64,         // 图片宽度占面板百分比 5 - 300
    scale_h: f64,         // 图片高度占面板百分比 5 - 300
    apply_settings: bool, // 背景同时应用到设置窗口
    rotation: i32,        // 旋转角度（0/90/180/270，烘焙进缓存图）
    region: Option<BgRegion>, // 归一化选区（基于旋转后的源图），None = 整图
    has_image: bool,      // 是否已选择图片
}

/// 背景图选区：归一化坐标（0-1），相对于旋转后的源图
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
struct BgRegion { x: f64, y: f64, w: f64, h: f64 }

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "stretch".into(),
            opacity: 0.6,
            scale_w: 100.0,
            scale_h: 100.0,
            apply_settings: false,
            rotation: 0,
            region: None,
            has_image: false,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.into(),
            autostart: false,
            silent_start: false,
            always_on_top: false,
            remember_size: false,
            window_width: 980,
            window_height: 680,
            theme: "dark".into(),
            font_family: "system-ui".into(),
            font_size: 14,
            highlight_color: String::new(), // 空 = 跟随主题强调色
            window_effect: "default".into(),
            view_mode: "card".into(),
            encrypt_unlock_mode: "session".into(),
            hotkey_unfocused_behavior: "front".into(),
            run_target_enabled: true,
            clear_search_on_reopen: true,
            config_dir: None,
            db_dir: None,
            background: BackgroundConfig::default(),
        }
    }
}

// ---------- 目录解析 ----------

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 默认数据目录：Windows 用 exe 同目录（便携）；macOS 的 .app 包内不可写，用 ~/Library/Application Support
#[cfg(windows)]
fn default_data_dir() -> PathBuf { exe_dir() }

#[cfg(not(windows))]
fn default_data_dir() -> PathBuf {
    dirs::data_dir().map(|dir| dir.join("com.suqi.launcher")).unwrap_or_else(exe_dir)
}

fn default_config_path() -> PathBuf { default_data_dir().join("config.json") }

fn pointer_path() -> PathBuf { default_data_dir().join(POINTER_FILE) }

fn effective_config_path() -> PathBuf {
    if let Ok(ptr) = fs::read_to_string(pointer_path()) {
        let dir = ptr.trim();
        if !dir.is_empty() { return PathBuf::from(dir).join("config.json"); }
    }
    default_config_path()
}

fn effective_db_dir() -> PathBuf {
    let cfg = load_config();
    cfg.db_dir.as_ref().map(PathBuf::from).unwrap_or_else(default_data_dir)
}

fn db_path() -> PathBuf { effective_db_dir().join(DB_FILE) }

fn load_config() -> AppConfig {
    let path = effective_config_path();
    fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_config_file(config: &AppConfig) -> Result<(), String> {
    let path = effective_config_path();
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    fs::write(&path, serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// ---------- 数据库 ----------

fn open_db() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建数据库目录失败：{e}"))?; }
    let conn = Connection::open(&path).map_err(|e| format!("打开数据库失败：{e}"))?;
    init_db(&conn)?;
    Ok(conn)
}

fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS items (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            keyword TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            target TEXT NOT NULL,
            lnk_path TEXT,
            group_id TEXT,
            is_encrypted INTEGER NOT NULL DEFAULT 0,
            is_cmd INTEGER NOT NULL DEFAULT 0,
            icon_path TEXT,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS password (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            salt TEXT NOT NULL,
            hash TEXT NOT NULL
        );"
    ).map_err(|e| format!("初始化数据库失败：{e}"))?;
    // 兼容旧库：items 表缺少新增列时补上（keyword 旧数据默认回填 name）
    let columns = conn.prepare("PRAGMA table_info(items)").and_then(|mut stmt| {
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut found: Vec<String> = Vec::new();
        for name in names { found.push(name?); }
        Ok(found)
    }).map_err(|e| format!("读取数据表结构失败：{e}"))?;
    let has_column = |column: &str| columns.iter().any(|name| name.eq_ignore_ascii_case(column));
    if !has_column("keyword") {
        conn.execute("ALTER TABLE items ADD COLUMN keyword TEXT NOT NULL DEFAULT ''", []).map_err(|e| format!("升级数据库失败：{e}"))?;
        conn.execute("UPDATE items SET keyword = name WHERE keyword = ''", []).map_err(|e| format!("升级数据库失败：{e}"))?;
    }
    if !has_column("description") {
        conn.execute("ALTER TABLE items ADD COLUMN description TEXT NOT NULL DEFAULT ''", []).map_err(|e| format!("升级数据库失败：{e}"))?;
    }
    if !has_column("lnk_path") {
        conn.execute("ALTER TABLE items ADD COLUMN lnk_path TEXT", []).map_err(|e| format!("升级数据库失败：{e}"))?;
    }
    Ok(())
}

fn load_store_from_db(conn: &Connection) -> Result<Store, String> {
    let mut store = Store::default();

    let mut stmt = conn.prepare("SELECT id, name, target, group_id, is_encrypted, is_cmd, icon_path, sort_order, created_at, keyword, description, lnk_path FROM items ORDER BY sort_order").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(LauncherItem {
            id: row.get(0)?,
            name: row.get(1)?,
            target: row.get(2)?,
            group_id: row.get(3)?,
            is_encrypted: row.get::<_, i64>(4)? != 0,
            is_cmd: row.get::<_, i64>(5)? != 0,
            icon_path: row.get(6)?,
            sort_order: row.get(7)?,
            created_at: row.get(8)?,
            keyword: row.get(9)?,
            description: row.get(10)?,
            lnk_path: row.get(11)?,
        })
    }).map_err(|e| e.to_string())?;
    for row in rows { store.items.push(row.map_err(|e| e.to_string())?); }

    let mut stmt = conn.prepare("SELECT id, name, sort_order FROM groups ORDER BY sort_order").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(Group { id: row.get(0)?, name: row.get(1)?, sort_order: row.get(2)? })
    }).map_err(|e| e.to_string())?;
    for row in rows { store.groups.push(row.map_err(|e| e.to_string())?); }

    let mut stmt = conn.prepare("SELECT salt, hash FROM password WHERE id = 1").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(PasswordHash { salt: row.get(0)?, hash: row.get(1)? })
    }).map_err(|e| e.to_string())?;
    for row in rows { store.password = Some(row.map_err(|e| e.to_string())?); }

    Ok(store)
}

fn save_store_to_db(conn: &Connection, store: &Store) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM items", []).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM groups", []).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM password", []).map_err(|e| e.to_string())?;
    for item in &store.items {
        tx.execute(
            "INSERT INTO items (id, name, target, group_id, is_encrypted, is_cmd, icon_path, sort_order, created_at, keyword, description, lnk_path) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![item.id, item.name, item.target, item.group_id, item.is_encrypted as i64, item.is_cmd as i64, item.icon_path, item.sort_order, item.created_at, item.keyword, item.description, item.lnk_path],
        ).map_err(|e| e.to_string())?;
    }
    for group in &store.groups {
        tx.execute(
            "INSERT INTO groups (id, name, sort_order) VALUES (?1,?2,?3)",
            params![group.id, group.name, group.sort_order],
        ).map_err(|e| e.to_string())?;
    }
    if let Some(pw) = &store.password {
        tx.execute(
            "INSERT INTO password (id, salt, hash) VALUES (1, ?1, ?2)",
            params![pw.salt, pw.hash],
        ).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn migrate_json_if_needed() {
    let json_path = default_data_dir().join("launchers.json");
    if !json_path.exists() { return; }
    let Ok(store) = serde_json::from_str::<Store>(&fs::read_to_string(&json_path).unwrap_or_default()) else { return };
    if let Ok(conn) = open_db() {
        let _ = save_store_to_db(&conn, &store);
    }
    let _ = fs::rename(&json_path, json_path.with_extension("json.bak"));
}

// ---------- 状态 ----------

// 会话级解锁状态：进程存活期间保持解锁，避免面板隐藏/重开后重复输入密码
struct AppData { store: Mutex<Store>, conn: Mutex<Connection>, unlocked: AtomicBool }

struct ConfigState {
    config: Mutex<AppConfig>,
    data_dir: PathBuf,
    // 托盘「钉住面板」菜单项句柄，用于同步勾选态
    tray_toggle: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
}

// ---------- 辅助函数 ----------

fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64 }
fn new_id() -> String { let mut bytes = [0u8; 12]; rand::thread_rng().fill_bytes(&mut bytes); hex::encode(bytes) }
fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c) || matches!(c, ' ' | '-' | '_' | '－' | '—' | '(' | ')' | '（' | '）' | '.' | '·'))
}
fn check_name(name: &str) -> Result<(), String> { if valid_name(name) { Ok(()) } else { Err("名称仅允许中文、英文、数字、空格及常见连字符".into()) } }
fn is_url(value: &str) -> bool { value.starts_with("http://") || value.starts_with("https://") }
fn normalize_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !is_url(value) || value.contains(char::is_whitespace) { return Err("URL 仅支持以 http:// 或 https:// 开头的有效地址".into()); }
    let (scheme, rest) = value.split_once("://").unwrap();
    if rest.is_empty() { return Err("URL 地址不能为空".into()); }
    Ok(format!("{}://{}", scheme.to_ascii_lowercase(), rest.trim_end_matches('/')))
}
fn resolve_lnk(path: &Path) -> String {
    #[cfg(windows)] {
        let escaped = path.display().to_string().replace('\'', "''");
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", &format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; $s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath", escaped)]);
        hide_console(&mut command);
        if let Ok(output) = command.output() {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !value.is_empty() { return value; }
        }
    }
    path.display().to_string()
}
fn display_path(value: impl AsRef<str>) -> String {
    value.as_ref().strip_prefix(r"\\?\").unwrap_or(value.as_ref()).to_string()
}
/// Windows 下以「无窗口」方式启动子进程，避免 powershell / cmd 等控制台程序闪现黑窗
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = command.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
#[allow(dead_code)]
fn hide_console(_command: &mut Command) {}
#[cfg(windows)]
fn ps_quote(value: impl AsRef<str>) -> String { format!("'{}'", value.as_ref().replace('\'', "''")) }
fn resolve_shortcuts(paths: &[PathBuf]) -> Vec<String> {
    #[cfg(windows)] {
        if paths.is_empty() { return Vec::new(); }
        let values = paths.iter().map(|path| ps_quote(path.display().to_string())).collect::<Vec<_>>().join(",");
        // 强制 stdout 为 UTF-8：中文 Windows 默认按 GBK 输出，含中文的目标路径会乱码导致导入判为无效；
        // $null 的 TargetPath 不产出任何行，显式转成空串，保证每个输入对应一行、与 paths 按序对齐\r
        let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; $w=New-Object -ComObject WScript.Shell; @({values}) | ForEach-Object {{ try {{ $t=$w.CreateShortcut($_).TargetPath; if ($null -eq $t) {{ '' }} else {{ $t }} }} catch {{ '' }} }}");
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", &script]);
        hide_console(&mut command);
        if let Ok(output) = command.output() {
            return String::from_utf8_lossy(&output.stdout).lines().map(|value| display_path(value.trim())).collect();
        }
    }
    paths.iter().map(|path| display_path(path.display().to_string())).collect()
}
/// Windows 图标提取：用 shell32 的 SHGetFileInfo 取系统图标，文件 / 文件夹 / 快捷方式均适用。
/// 注意：System.Drawing 的 ExtractAssociatedIcon 对「目录」会抛异常（对文件夹永远取不到图标），故改用 SHGetFileInfo。
/// 图标数量较多时直接拼进命令行会超出长度上限导致部分失败，这里把脚本写入临时文件后执行。
#[cfg(windows)]
fn extract_icons(pairs: &[(PathBuf, PathBuf)]) {
    if pairs.is_empty() { return; }
    const TYPE_DEF: &str = r#"Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class SuqiIcon {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Auto)]
  public struct SHFILEINFO { public IntPtr hIcon; public int iIcon; public uint dwAttributes; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string szDisplayName; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=80)] public string szTypeName; }
  [DllImport("shell32.dll", CharSet=CharSet.Auto)] public static extern IntPtr SHGetFileInfo(string pszPath, uint dwFileAttributes, ref SHFILEINFO psfi, uint cbFileInfo, uint uFlags);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr hIcon);
  public static bool Save(string path, string output) {
    SHFILEINFO info = new SHFILEINFO();
    IntPtr res = SHGetFileInfo(path, 0, ref info, (uint)Marshal.SizeOf(info), 0x100);
    if (res == IntPtr.Zero || info.hIcon == IntPtr.Zero) { return false; }
    try { using (Icon icon = Icon.FromHandle(info.hIcon)) using (Bitmap bmp = icon.ToBitmap()) { bmp.Save(output, System.Drawing.Imaging.ImageFormat.Png); } }
    catch { return false; }
    DestroyIcon(info.hIcon);
    return true;
  }
}
'@
"#;
    let values = pairs.iter().flat_map(|(input, output)| [ps_quote(input.display().to_string()), ps_quote(output.display().to_string())]).collect::<Vec<_>>().join(",");
    let script = format!("{TYPE_DEF}$p=@({values}); for($i=0;$i -lt $p.Count;$i+=2){{ [SuqiIcon]::Save($p[$i],$p[$i+1]) | Out-Null }}");
    let script_file = std::env::temp_dir().join(format!("suqi-icons-{}-{}.ps1", std::process::id(), now()));
    // PowerShell 5.1 默认按 ANSI 读取脚本文件，含中文路径时必须带 UTF-8 BOM 才不会乱码
    let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    if fs::write(&script_file, bytes).is_err() { return; }
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script_file);
    hide_console(&mut command);
    let _ = command.output();
    let _ = fs::remove_file(&script_file);
}
/// macOS 下判断是否为 `.app` 应用包（应用包是目录形态，以 .app 结尾）
#[cfg(target_os = "macos")]
fn is_app_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
}
/// macOS 下取 `.app` 应用包内的图标文件：优先 AppIcon.icns，否则取体积最大的 .icns
#[cfg(target_os = "macos")]
fn app_icon_icns(app: &Path) -> Option<PathBuf> {
    let mut preferred: Option<PathBuf> = None;
    let mut largest: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(app.join("Contents").join("Resources")).ok()?.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("icns")) { continue; }
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case("appicon.icns") { preferred = Some(path.clone()); }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let better = match &largest { Some((best, _)) => size > *best, None => true };
        if better { largest = Some((size, path)); }
    }
    preferred.or_else(|| largest.map(|(_, path)| path))
}
/// macOS 下用 plutil（系统自带）读取 plist / .strings 文件里的字符串键值，失败返回 None
#[cfg(target_os = "macos")]
fn plutil_string(path: &Path, key: &str) -> Option<String> {
    if !path.exists() { return None; }
    let output = Command::new("plutil").args(["-extract", key, "raw", "-o", "-"]).arg(path).output().ok()?;
    if !output.status.success() { return None; }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}
/// macOS 系统语言（如 zh_CN）。GUI 应用进程里通常没有 LANG 环境变量，读系统默认设置；进程内只读一次
#[cfg(target_os = "macos")]
fn mac_locale() -> &'static str {
    static LOCALE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LOCALE.get_or_init(|| {
        Command::new("defaults").args(["read", "-g", "AppleLocale"]).output().ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    })
}
/// macOS 下取 `.app` 的本地化显示名称（如「微信」而非「WeChat」）：
/// 1. 按系统语言在对应 lproj 的 InfoPlist.strings 里找 CFBundleDisplayName / CFBundleName；
/// 2. 回退 Info.plist 的 CFBundleDisplayName / CFBundleName；
/// 3. 都没有则返回 None，由调用方用文件名兜底
#[cfg(target_os = "macos")]
fn app_display_name(app: &Path) -> Option<String> {
    let resources = app.join("Contents").join("Resources");
    // 不同 App 的 lproj 命名不一致（zh_CN / zh-Hans_CN / zh-Hans ...），按系统语言生成候选逐个尝试
    let locale = mac_locale();
    let mut lprojs: Vec<String> = Vec::new();
    if !locale.is_empty() {
        lprojs.push(locale.to_string());
        let (lang, region) = locale.split_once('_').unwrap_or((locale, ""));
        if lang == "zh" {
            let script = if matches!(region, "TW" | "HK" | "MO") { "zh-Hant" } else { "zh-Hans" };
            lprojs.push(format!("{script}_{region}"));
            lprojs.push(script.to_string());
        }
        lprojs.push(lang.to_string());
    }
    for key in ["CFBundleDisplayName", "CFBundleName"] {
        for lproj in &lprojs {
            if let Some(value) = plutil_string(&resources.join(format!("{lproj}.lproj")).join("InfoPlist.strings"), key) { return Some(value); }
        }
    }
    let info = app.join("Contents").join("Info.plist");
    for key in ["CFBundleDisplayName", "CFBundleName"] {
        if let Some(value) = plutil_string(&info, key) { return Some(value); }
    }
    None
}
/// macOS 下用 sips（系统自带）把 icns / 图片转成 128px PNG。比 qlmanage 快很多，也不会崩溃或挂起
#[cfg(target_os = "macos")]
fn icns_to_png(source: &Path, output: &Path) -> bool {
    if output.exists() { return true; }
    let mut command = Command::new("sips");
    command.args(["-s", "format", "png", "-Z", "128"]).arg(source).arg("--out").arg(output);
    run_with_timeout(&mut command, std::time::Duration::from_secs(10));
    output.exists()
}
/// macOS 下系统 CoreTypes.bundle 里的通用图标（应用 / 文件夹等）
#[cfg(target_os = "macos")]
fn system_icon_icns(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from("/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources").join(name);
    path.exists().then_some(path)
}
/// 把同一张通用 icns 转换后复制给一批目标路径（只转换一次，避免逐个调用外部命令）
#[cfg(target_os = "macos")]
fn copy_generic_icon(source: &Path, outputs: &[&PathBuf]) {
    let mut cached: Option<PathBuf> = None;
    let mut converted = false;
    for output in outputs {
        if output.exists() { continue; }
        if !converted {
            converted = true;
            let temp = output.with_extension("tmp.png");
            if icns_to_png(source, &temp) { cached = Some(temp); }
        }
        if let Some(temp) = &cached { let _ = fs::copy(temp, output); }
    }
    if let Some(temp) = cached { let _ = fs::remove_file(temp); }
}
/// macOS 图标提取：
/// 1. `.app` 应用包：直接读包内 `.icns`（优先 AppIcon.icns）用 sips 转 128px PNG，并发执行；
/// 2. 仍缺图标的应用包 / 文件夹：用系统 CoreTypes.bundle 里的通用图标兜底；
/// 3. 普通文件：图片类直接用 sips 转缩略图，其余用系统通用文档图标兜底。
///    全程不用 qlmanage —— 它对部分文件会直接崩溃，导致系统反复弹「“qlmanage”意外退出」
#[cfg(target_os = "macos")]
fn extract_icons(pairs: &[(PathBuf, PathBuf)]) {
    if pairs.is_empty() { return; }
    // 1) .app 应用包：并发调用 sips，避免应用数量多时串行等待
    let apps: Vec<&(PathBuf, PathBuf)> = pairs.iter().filter(|(input, _)| is_app_bundle(input)).collect();
    if !apps.is_empty() {
        let workers = std::thread::available_parallelism().map(|count| count.get().min(4)).unwrap_or(2);
        let chunk_size = apps.len().div_ceil(workers).max(1);
        std::thread::scope(|scope| {
            for chunk in apps.chunks(chunk_size) {
                scope.spawn(move || {
                    for (input, output) in chunk {
                        if output.exists() { continue; }
                        if let Some(icns) = app_icon_icns(input) { icns_to_png(&icns, output); }
                    }
                });
            }
        });
    }
    // 2) 通用图标兜底：仍缺图标的应用包 / 文件夹
    let app_outputs: Vec<&PathBuf> = pairs.iter().filter(|(input, output)| !output.exists() && is_app_bundle(input)).map(|(_, output)| output).collect();
    if !app_outputs.is_empty() {
        if let Some(source) = system_icon_icns("GenericApplicationIcon.icns") { copy_generic_icon(&source, &app_outputs); }
    }
    let folder_outputs: Vec<&PathBuf> = pairs.iter().filter(|(input, output)| !output.exists() && !is_app_bundle(input) && input.is_dir()).map(|(_, output)| output).collect();
    if !folder_outputs.is_empty() {
        if let Some(source) = system_icon_icns("GenericFolderIcon.icns") { copy_generic_icon(&source, &folder_outputs); }
    }
    // 3) 普通文件：图片类直接用 sips 转缩略图，其余用系统通用文档图标兜底。
    //    早期版本回退 qlmanage，但它对部分文件会崩溃，系统反复弹「“qlmanage”意外退出」，故彻底移除
    let image_exts = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "ico", "icns"];
    let mut doc_outputs: Vec<&PathBuf> = Vec::new();
    for (input, output) in pairs.iter().filter(|(_, output)| !output.exists()) {
        let is_image = input.extension().is_some_and(|ext| image_exts.iter().any(|e| ext.eq_ignore_ascii_case(e)));
        if is_image && icns_to_png(input, output) { continue; }
        doc_outputs.push(output);
    }
    if !doc_outputs.is_empty() {
        if let Some(source) = system_icon_icns("GenericDocumentIcon.icns") { copy_generic_icon(&source, &doc_outputs); }
    }
}
/// 运行外部命令并在超时后强制结束，返回是否正常退出（避免 qlmanage 等工具挂起导致界面长时间无响应）
#[cfg(target_os = "macos")]
fn run_with_timeout(command: &mut Command, timeout: std::time::Duration) -> bool {
    let Ok(mut child) = command.spawn() else { return false; };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(std::time::Duration::from_millis(100)),
            Ok(None) => { let _ = child.kill(); let _ = child.wait(); return false; }
            Err(_) => { let _ = child.kill(); return false; }
        }
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
fn extract_icons(_pairs: &[(PathBuf, PathBuf)]) {}
fn normalize_target(value: &str, is_cmd: bool) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() { return Err("启动目标不能为空".into()); }
    if is_cmd { return Ok(value.to_string()); }
    if is_url(value) { return normalize_url(value); }
    // Windows 外壳命名空间目标（shell: 开头，如 shell:RecycleBinFolder）：由 explorer 负责解析，不能按本地路径校验
    #[cfg(windows)]
    if value.len() > 6 && value[..6].eq_ignore_ascii_case("shell:") { return Ok(value.to_string()); }
    let path = Path::new(value);
    if !path.is_absolute() { return Err("本地路径必须为绝对路径".into()); }
    let resolved = if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) { PathBuf::from(resolve_lnk(path)) } else { path.to_path_buf() };
    fs::canonicalize(&resolved).map_err(|_| "本地路径不存在或无法访问".to_string()).map(|p| display_path(p.display().to_string()))
}
/// 记录条目的快捷方式来源（仅 Windows 的 .lnk 有意义）：
/// 目标本身是 .lnk → 记录之；目标未变（仅改名等编辑）→ 沿用原值；其他情况 → 清空
#[cfg(windows)]
fn resolve_lnk_path(raw_target: &str, resolved: &str, is_cmd: bool, existing: Option<&LauncherItem>) -> Option<String> {
    if !is_cmd && !is_url(raw_target) && Path::new(raw_target).extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
        return Some(fs::canonicalize(raw_target).map(|p| display_path(p.display().to_string())).unwrap_or_else(|_| raw_target.to_string()));
    }
    if existing.is_some_and(|old| old.target == resolved) { return existing.and_then(|old| old.lnk_path.clone()); }
    None
}
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn resolve_lnk_path(_raw_target: &str, _resolved: &str, _is_cmd: bool, _existing: Option<&LauncherItem>) -> Option<String> { None }

fn pinyin_text(name: &str) -> (String, String) {
    let parts: Vec<String> = name.to_pinyin().map(|x| x.map(|p| p.plain().to_string()).unwrap_or_default()).collect();
    let full = parts.join("");
    let initials = parts.iter().filter_map(|p| p.chars().next()).collect();
    (full, initials)
}
fn encrypted(item: &LauncherItem) -> bool { item.is_encrypted || item.group_id.as_deref() == Some(ENCRYPTED_GROUP) }

/// 条目所属分组的显示名称：加密分组 / 自定义分组名 / 未分组（用于导入结果中提示重复项位置）
fn item_group_label(store: &Store, item: &LauncherItem) -> String {
    if encrypted(item) { return "加密".into(); }
    item.group_id.as_ref()
        .and_then(|id| store.groups.iter().find(|group| &group.id == id))
        .map(|group| group.name.clone())
        .unwrap_or_else(|| "未分组".into())
}

fn persist(data: &AppData, store: &Store) -> Result<(), String> {
    let conn = data.conn.lock().unwrap();
    save_store_to_db(&conn, store)
}

// ---------- 启动项命令 ----------

#[tauri::command]
fn list_data(data: State<'_, AppData>, include_encrypted: bool) -> ListResult {
    let store = data.store.lock().unwrap();
    let unlocked = data.unlocked.load(Ordering::SeqCst);
    ListResult {
        items: store.items.iter().filter(|item| include_encrypted || unlocked || !encrypted(item)).cloned().collect(),
        groups: store.groups.clone(),
        password_set: store.password.is_some(),
        unlocked,
    }
}

#[tauri::command]
fn searchable_items(data: State<'_, AppData>, include_encrypted: bool) -> Vec<SearchText> {
    let unlocked = data.unlocked.load(Ordering::SeqCst);
    data.store.lock().unwrap().items.iter().filter(|item| include_encrypted || unlocked || !encrypted(item)).map(|item| {
        let (full_pinyin, initials) = pinyin_text(&item.name);
        let (keyword_full_pinyin, keyword_initials) = pinyin_text(&item.keyword);
        SearchText { id: item.id.clone(), name: item.name.clone(), full_pinyin, initials, keyword: item.keyword.clone(), keyword_full_pinyin, keyword_initials }
    }).collect()
}

#[tauri::command]
fn save_item(data: State<'_, AppData>, mut item: LauncherItem, is_cmd: bool) -> Result<LauncherItem, String> {
    check_name(&item.name)?;
    item.name = item.name.trim().to_string();
    // 关键字留空时回退为名称，保证每项都有可搜索关键字
    item.keyword = item.keyword.trim().to_string();
    if item.keyword.is_empty() { item.keyword = item.name.clone(); }
    item.description = item.description.trim().to_string();
    // 保存前取一次原条目，便于目标未变（仅改名等）时保留快捷方式来源
    let existing = data.store.lock().unwrap().items.iter().find(|x| x.id == item.id).cloned();
    let raw_target = item.target.trim().to_string();
    item.target = normalize_target(&raw_target, is_cmd)?;
    item.lnk_path = resolve_lnk_path(&raw_target, &item.target, is_cmd, existing.as_ref());
    item.is_encrypted = item.group_id.as_deref() == Some(ENCRYPTED_GROUP);
    item.is_cmd = is_cmd;
    let mut store = data.store.lock().unwrap();
    if item.group_id.as_deref() != Some(ENCRYPTED_GROUP) && item.group_id.as_ref().is_some_and(|id| !store.groups.iter().any(|g| &g.id == id)) { return Err("所选分组不存在".into()); }
    if store.items.iter().any(|x| x.id != item.id && x.target.eq_ignore_ascii_case(&item.target)) { return Err("该真实目标已存在，不能重复添加".into()); }
    if item.id.is_empty() {
        item.id = new_id(); item.created_at = now(); item.sort_order = store.items.iter().map(|x| x.sort_order).max().unwrap_or(-1) + 1; store.items.push(item.clone());
    } else if let Some(existing) = store.items.iter_mut().find(|x| x.id == item.id) {
        item.created_at = existing.created_at; item.sort_order = existing.sort_order; *existing = item.clone();
    } else { return Err("要编辑的启动项不存在".into()); }
    persist(&data, &store)?; Ok(item)
}

#[tauri::command]
fn delete_item(data: State<'_, AppData>, id: String) -> Result<(), String> {
    let mut store = data.store.lock().unwrap(); store.items.retain(|x| x.id != id); persist(&data, &store)
}

/// 批量删除启动项（面板多选后一次性删除）
#[tauri::command]
fn delete_items(data: State<'_, AppData>, ids: Vec<String>) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    store.items.retain(|item| !ids.contains(&item.id));
    persist(&data, &store)
}

/// 批量移动启动项到指定分组（group_id 为 None 表示「未分组」）
#[tauri::command]
fn move_items(data: State<'_, AppData>, ids: Vec<String>, group_id: Option<String>) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    if group_id.as_deref() != Some(ENCRYPTED_GROUP) && group_id.as_ref().is_some_and(|id| !store.groups.iter().any(|g| &g.id == id)) {
        return Err("所选分组不存在".into());
    }
    let encrypted = group_id.as_deref() == Some(ENCRYPTED_GROUP);
    for item in store.items.iter_mut() {
        if ids.contains(&item.id) {
            item.group_id = group_id.clone();
            item.is_encrypted = encrypted;
        }
    }
    persist(&data, &store)
}

#[tauri::command]
fn open_item(data: State<'_, AppData>, id: String, unlocked: bool) -> Result<(), String> {
    let item = data.store.lock().unwrap().items.iter().find(|x| x.id == id).cloned().ok_or("启动项不存在")?;
    if encrypted(&item) && !unlocked && !data.unlocked.load(Ordering::SeqCst) { return Err("请先解锁加密分组".into()); }
    #[cfg(windows)] {
        if item.is_cmd {
            // CMD 命令以无窗口方式运行，避免弹出控制台黑窗
            let mut command = Command::new("cmd");
            command.args(["/C", &item.target]);
            hide_console(&mut command);
            command.spawn().map_err(|e| format!("无法打开目标：{e}"))?;
        } else {
            // 快捷方式来源的条目优先启动原始 .lnk（Shell 会还原 目标+参数+工作目录）；lnk 缺失时回退解析后的目标
            let lnk = item.lnk_path.as_deref().filter(|path| Path::new(path).is_file());
            let launch = lnk.unwrap_or(&item.target);
            Command::new("explorer.exe").arg(launch).spawn().map_err(|e| format!("无法打开目标：{e}"))?;
        }
    }
    #[cfg(not(windows))] {
        if item.is_cmd {
            // 命令行可能长时间运行，保持异步拉起，不等退出码
            Command::new("sh").args(["-c", &item.target]).spawn().map_err(|e| format!("无法打开目标：{e}"))?;
        } else {
            // open 失败时（应用已损坏、被隔离、无权限等）以非零码退出并把原因写到 stderr。
            // 之前只 spawn 不等结果，启动失败完全无提示；这里等它结束（通常毫秒级）把原因反馈给用户
            let output = Command::new("open").arg(&item.target).output().map_err(|e| format!("无法打开目标：{e}"))?;
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if detail.is_empty() { "无法打开目标".to_string() } else { format!("无法打开目标：{detail}") });
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn create_group(data: State<'_, AppData>, name: String) -> Result<Group, String> {
    check_name(&name)?;
    let mut store = data.store.lock().unwrap();
    if store.groups.iter().any(|g| g.name == name.trim()) { return Err("已存在同名分组".into()); }
    let group = Group { id: new_id(), name: name.trim().into(), sort_order: store.groups.iter().map(|g| g.sort_order).max().unwrap_or(-1) + 1 };
    store.groups.push(group.clone()); persist(&data, &store)?; Ok(group)
}

#[tauri::command]
fn rename_group(data: State<'_, AppData>, id: String, name: String) -> Result<(), String> {
    check_name(&name)?;
    let mut store = data.store.lock().unwrap();
    if store.groups.iter().any(|g| g.id != id && g.name == name.trim()) { return Err("已存在同名分组".into()); }
    store.groups.iter_mut().find(|g| g.id == id).ok_or("分组不存在")?.name = name.trim().into(); persist(&data, &store)
}

#[tauri::command]
fn delete_group(data: State<'_, AppData>, id: String, delete_items: bool) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    if !store.groups.iter().any(|g| g.id == id) { return Err("分组不存在".into()); }
    store.groups.retain(|g| g.id != id);
    // delete_items 为真时连同组内项目一起删除，否则仅清空项目上的分组归属
    if delete_items {
        store.items.retain(|item| item.group_id.as_deref() != Some(&id));
    } else {
        for item in &mut store.items { if item.group_id.as_deref() == Some(&id) { item.group_id = None; } }
    }
    persist(&data, &store)
}

#[tauri::command]
fn save_group_order(data: State<'_, AppData>, ids: Vec<String>) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    for (index, id) in ids.iter().enumerate() { if let Some(group) = store.groups.iter_mut().find(|g| &g.id == id) { group.sort_order = index as i64; } }
    persist(&data, &store)
}

#[tauri::command]
fn save_item_order(data: State<'_, AppData>, ids: Vec<String>) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    for (index, id) in ids.iter().enumerate() { if let Some(item) = store.items.iter_mut().find(|x| &x.id == id) { item.sort_order = index as i64; } }
    persist(&data, &store)
}

// ---------- 直接运行（仿 Win + R） ----------

/// 把搜索框内容当「运行」输入执行：UNC 路径、URL、shell: 目标、盘符路径、命令名（可带参数）均可
#[tauri::command]
fn run_target(raw: String) -> Result<(), String> {
    let input = raw.trim();
    if input.is_empty() { return Err("请输入要运行的内容".into()); }
    #[cfg(windows)]
    { run_target_windows(input) }
    #[cfg(not(windows))]
    { run_target_fallback(input) }
}

#[cfg(windows)]
fn explorer_open(target: &str) -> Result<(), String> {
    Command::new("explorer.exe").arg(target).spawn().map(|_| ()).map_err(|e| format!("无法打开：{e}"))
}

/// 展开 %VAR% 形式的环境变量（Win+R 同样支持，如 %appdata%）；未定义的变量原样保留
#[cfg(windows)]
fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        match tail.find('%') {
            Some(end) if end > 0 => {
                let name = &tail[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => { out.push('%'); out.push_str(&tail[..=end]); }
                }
                rest = &tail[end + 1..];
            }
            _ => { out.push('%'); out.push_str(tail); rest = ""; }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
fn run_target_windows(input: &str) -> Result<(), String> {
    let expanded = expand_env_vars(input);
    let lower = expanded.to_lowercase();
    // UNC / URL / shell: 命名空间目标 → 交给资源管理器（等同 Win+R 的 ShellExecute 语义）
    if expanded.starts_with(r"\\") || expanded.starts_with("//") || lower.contains("://") || lower.starts_with("shell:") {
        return explorer_open(&expanded);
    }
    let words: Vec<&str> = expanded.split_whitespace().collect();
    // 前缀探测：允许不带引号的含空格路径——前 k 个词拼起来是存在的路径，其余视为参数
    for k in (1..=words.len()).rev() {
        let candidate = words[..k].join(" ");
        if Path::new(&candidate).is_absolute() && Path::new(&candidate).exists() {
            return launch_resolved(&candidate, &words[k..]);
        }
    }
    // 单个词按 Win+R 规则解析命令名（App Paths 注册表 → PATH + PATHEXT）
    let first = words[0];
    match resolve_command(first) {
        Some(resolved) => launch_resolved(&resolved, &words[1..]),
        None => Err(format!("找不到命令、路径或程序：{first}")),
    }
}

/// 启动已解析的目标：exe / 批处理直接拉起；其余类型（.msc、文档、文件夹等）交给 explorer 按文件关联打开
#[cfg(windows)]
fn launch_resolved(path: &str, args: &[&str]) -> Result<(), String> {
    let ext = Path::new(path).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd") {
        // 不隐藏控制台：用户输入 cmd / ping 等命令时预期看到窗口
        Command::new(path).args(args).spawn().map(|_| ()).map_err(|e| format!("无法运行：{e}"))
    } else if args.is_empty() {
        explorer_open(path)
    } else {
        // 非可执行文件带参数：经 PowerShell Start-Process 走 ShellExecute，按文件关联启动并透传参数
        let arg_list = args.iter().map(ps_quote).collect::<Vec<_>>().join(",");
        let script = format!("try {{ Start-Process -FilePath {} -ArgumentList @({arg_list}) -ErrorAction Stop }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}", ps_quote(path));
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", &script]);
        hide_console(&mut command);
        let output = command.output().map_err(|e| format!("无法运行：{e}"))?;
        if output.status.success() { Ok(()) } else {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if detail.is_empty() { "无法打开目标".into() } else { format!("无法打开目标：{detail}") })
        }
    }
}

/// 按 Win+R 的规则解析命令名：1) 自身即存在的路径 2) App Paths 注册表 3) PATH + PATHEXT 搜索
#[cfg(windows)]
fn resolve_command(name: &str) -> Option<String> {
    if Path::new(name).exists() && (Path::new(name).is_absolute() || name.contains(['\\', '/'])) { return Some(name.to_string()); }
    let has_ext = Path::new(name).extension().is_some();
    // App Paths：chrome / winword 等注册的执行别名（HKLM 优先，HKCU 兜底）
    for root in [winreg::enums::HKEY_LOCAL_MACHINE, winreg::enums::HKEY_CURRENT_USER] {
        let base = winreg::RegKey::predef(root);
        let key_names = if has_ext { vec![name.to_string()] } else { vec![format!("{name}.exe"), name.to_string()] };
        for key_name in key_names {
            if let Ok(key) = base.open_subkey(format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{key_name}")) {
                if let Ok(value) = key.get_value::<String, _>("") {
                    if Path::new(&value).exists() { return Some(value); }
                }
            }
        }
    }
    // PATH 搜索：无扩展名时按 PATHEXT 依次补全（NTFS 不区分大小写，无需枚举大小写组合）
    let path_var = std::env::var_os("PATH")?;
    let pathext: Vec<String> = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    for dir in std::env::split_paths(&path_var) {
        let candidates: Vec<PathBuf> = if has_ext { vec![dir.join(name)] } else { pathext.iter().map(|ext| dir.join(format!("{name}{ext}"))).collect() };
        for candidate in candidates {
            if candidate.is_file() { return Some(display_path(candidate.display().to_string())); }
        }
    }
    None
}

/// macOS 兜底：路径 / URL 交给 open，其余按命令直接执行（Command 会走 PATH 查找）
#[cfg(not(windows))]
fn run_target_fallback(input: &str) -> Result<(), String> {
    if input.starts_with('/') || input.starts_with('~') || input.to_lowercase().contains("://") {
        let output = Command::new("open").arg(input).output().map_err(|e| format!("无法打开：{e}"))?;
        if output.status.success() { return Ok(()); }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() { "无法打开目标".into() } else { format!("无法打开目标：{detail}") });
    }
    let words: Vec<&str> = input.split_whitespace().collect();
    Command::new(words[0]).args(&words[1..]).spawn().map(|_| ()).map_err(|_| format!("找不到命令：{}", words[0]))
}

fn password_hash(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new(); hasher.update(salt.as_bytes()); hasher.update(password.as_bytes()); hex::encode(hasher.finalize())
}

#[tauri::command]
fn unlock_encrypted(data: State<'_, AppData>, password: String) -> Result<bool, String> {
    let store = data.store.lock().unwrap();
    let Some(saved) = &store.password else { return Err("请先设置密码".into()) };
    let ok = password_hash(&password, &saved.salt) == saved.hash;
    if ok { data.unlocked.store(true, Ordering::SeqCst); }
    Ok(ok)
}

/// 手动上锁：立即锁定加密分组，重新查看需再次输入密码
#[tauri::command]
fn lock_encrypted(data: State<'_, AppData>) {
    data.unlocked.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn reset_password(data: State<'_, AppData>, current_password: Option<String>, new_password: String) -> Result<(), String> {
    if new_password.trim().len() < 4 { return Err("密码至少需要 4 个字符".into()); }
    let mut store = data.store.lock().unwrap();
    if let Some(saved) = &store.password {
        let current = current_password.ok_or("请输入当前密码")?;
        if password_hash(&current, &saved.salt) != saved.hash { return Err("当前密码不正确".into()); }
    }
    let mut bytes = [0u8; 16]; rand::thread_rng().fill_bytes(&mut bytes);
    let salt = hex::encode(bytes);
    store.password = Some(PasswordHash { hash: password_hash(&new_password, &salt), salt });
    data.unlocked.store(true, Ordering::SeqCst);
    persist(&data, &store)
}

/// 忘记密码：清空加密分组内全部条目并重置密码，让用户从零开始
#[tauri::command]
fn clear_encrypted(data: State<'_, AppData>) -> Result<(), String> {
    let mut store = data.store.lock().unwrap();
    store.items.retain(|item| !encrypted(item));
    store.password = None;
    data.unlocked.store(false, Ordering::SeqCst);
    persist(&data, &store)
}

// ---------- 桌面扫描 ----------

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DesktopEntry {
    name: String,
    source_path: String,
    target: String,
    is_shortcut: bool,
    entry_type: String,
    scan_icon_path: Option<String>,
    /// 条目所在文件夹名称（用于扫描结果页展示）
    folder_name: Option<String>,
    /// 条目相对扫描根目录的路径分段（含条目自身名称），用于扫描结果树状展示
    #[serde(default)]
    rel_path: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportSkip { name: String, target: String, group_name: Option<String>, reason: Option<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult { added: usize, skipped_duplicates: usize, skipped_invalid: usize, duplicates: Vec<ImportSkip>, invalids: Vec<ImportSkip>, imported_shortcuts: Vec<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemIcon { id: String, icon_path: Option<String> }

/// 删除源快捷方式的结果：deleted 为成功数，failures 复用 ImportSkip 记录失败明细
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteShortcutsResult { deleted: usize, failures: Vec<ImportSkip> }

/// 扫描一个目录下的文件 / 文件夹 / 快捷方式，生成可导入条目；recursive 为真时同时扫描子文件夹
fn scan_dir(dir: &Path, recursive: bool) -> Result<Vec<DesktopEntry>, String> {
    let mut entries = Vec::new();
    let icon_dir = default_data_dir().join("scan-icons");
    let _ = fs::create_dir_all(&icon_dir);
    let mut icon_pairs = Vec::new();
    // 待扫描目录栈：(目录, 是否为根目录)，根目录读取失败直接报错，子目录失败则跳过
    let mut pending = vec![(dir.to_path_buf(), true)];
    while let Some((current, is_root)) = pending.pop() {
        let reader = match fs::read_dir(&current) {
            Ok(reader) => reader,
            Err(e) if is_root => return Err(format!("读取目录失败：{e}")),
            Err(_) => continue,
        };
        for entry in reader {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.eq_ignore_ascii_case("desktop.ini") || file_name == ".DS_Store" { continue; }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            // macOS 的 .app 是目录形态的应用包，按「快捷方式」处理且不再递归进入其内部
            let is_app_bundle = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("app"));
            let shortcut = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) || is_app_bundle;
            if recursive && kind.is_dir() && !is_app_bundle { pending.push((path.clone(), false)); }
            let entry_type = if shortcut { "shortcut" } else if kind.is_dir() { "folder" } else if kind.is_file() { "file" } else { continue };
            let source_path = display_path(path.display().to_string());
            let folder_name = parent_folder_name(&source_path);
            let rel_path: Vec<String> = path.strip_prefix(dir)
                .map(|relative| relative.components().map(|part| part.as_os_str().to_string_lossy().to_string()).collect())
                .unwrap_or_default();
            let mut hasher = Sha256::new(); hasher.update(source_path.as_bytes());
            let icon_file = icon_dir.join(format!("{}.png", hex::encode(hasher.finalize())));
            if !icon_file.exists() { icon_pairs.push((path.clone(), icon_file.clone())); }
            // macOS 应用包优先用本地化显示名（如「微信」而非「WeChat」），其余条目用文件名
            let fallback = || path.file_stem().or_else(|| path.file_name()).unwrap_or_default().to_string_lossy().into();
            #[cfg(target_os = "macos")]
            let name = if is_app_bundle { app_display_name(&path) } else { None }.unwrap_or_else(fallback);
            #[cfg(not(target_os = "macos"))]
            let name: String = fallback();
            entries.push(DesktopEntry {
                name,
                target: source_path.clone(),
                source_path,
                is_shortcut: shortcut,
                entry_type: entry_type.into(),
                scan_icon_path: None,
                folder_name,
                rel_path,
            });
        }
    }
    extract_icons(&icon_pairs);
    for entry in &mut entries {
        let mut hasher = Sha256::new(); hasher.update(entry.source_path.as_bytes());
        let icon_file = icon_dir.join(format!("{}.png", hex::encode(hasher.finalize())));
        entry.scan_icon_path = icon_file.exists().then(|| display_path(icon_file.display().to_string()));
    }
    sort_scan_entries(&mut entries);
    Ok(entries)
}

/// 扫描结果排序：快捷方式在前，其次是系统图标（此电脑 / 回收站等），其余类型按名称排序
fn sort_scan_entries(entries: &mut [DesktopEntry]) {
    entries.sort_by(|a, b| {
        let rank = |e: &DesktopEntry| if e.is_shortcut { 0 } else if e.entry_type == "system" { 1 } else { 2 };
        rank(a).cmp(&rank(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

// ---------- 桌面系统图标（此电脑 / 回收站等） ----------

/// 桌面「系统图标」是外壳虚拟对象而非文件，目录扫描读不到；按注册表配置单独补充进桌面扫描结果
#[cfg(windows)]
struct ShellIconSpec { clsid: &'static str, target: &'static str, fallback_name: &'static str, default_shown: bool, prefer_fallback_name: bool }

/// 「桌面图标设置」对话框中的五个系统图标：CLSID 即注册表 HideDesktopIcons 下的值名
#[cfg(windows)]
const SHELL_ICON_SPECS: &[ShellIconSpec] = &[
    ShellIconSpec { clsid: "{20D04FE0-3AEA-1069-A2D8-08002B30309D}", target: "shell:MyComputerFolder", fallback_name: "此电脑", default_shown: false, prefer_fallback_name: false },
    ShellIconSpec { clsid: "{645FF040-5081-101B-9F08-00AA002F954E}", target: "shell:RecycleBinFolder", fallback_name: "回收站", default_shown: true, prefer_fallback_name: false },
    // 命名空间显示名是「所有控制面板项」，与桌面图标标签「控制面板」不一致，这里以桌面标签为准
    ShellIconSpec { clsid: "{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}", target: "shell:ControlPanelFolder", fallback_name: "控制面板", default_shown: false, prefer_fallback_name: true },
    ShellIconSpec { clsid: "{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", target: "shell:NetworkPlacesFolder", fallback_name: "网络", default_shown: false, prefer_fallback_name: false },
    ShellIconSpec { clsid: "{59031A47-3F72-44A7-89C5-5595FE6B30EE}", target: "shell:::{59031A47-3F72-44A7-89C5-5595FE6B30EE}", fallback_name: "用户的文件", default_shown: false, prefer_fallback_name: false },
];

/// 读注册表判断某系统图标当前是否显示在桌面：HideDesktopIcons 下值 0 显示、1 隐藏；
/// 两个面板配置都未写过该项时用默认值（全新系统默认只显示回收站）
#[cfg(windows)]
fn shell_icon_shown(clsid: &str, default_shown: bool) -> bool {
    let base = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    for subkey in [
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\ClassicStartMenu",
    ] {
        if let Ok(key) = base.open_subkey(subkey) {
            if let Ok(value) = key.get_value::<u32, _>(clsid) { return value == 0; }
        }
    }
    default_shown
}

/// 提取外壳虚拟对象（shell: 目标）的图标与系统显示名：SHGetFileInfo 不认这类字符串路径，
/// 需先 SHParseDisplayName 解析成 PIDL 再取图标（SHGFI_PIDL）。
/// 返回与 pairs 等长的显示名列表（取不到为空串），图标写入对应输出文件；脚本写临时文件执行，理由同 extract_icons
#[cfg(windows)]
fn extract_shell_icons(pairs: &[(String, PathBuf)]) -> Vec<String> {
    if pairs.is_empty() { return Vec::new(); }
    const TYPE_DEF: &str = r#"Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class SuqiShellIcon {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Auto)]
  public struct SHFILEINFO { public IntPtr hIcon; public int iIcon; public uint dwAttributes; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string szDisplayName; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=80)] public string szTypeName; }
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SHGetFileInfo(IntPtr pszPath, uint dwFileAttributes, ref SHFILEINFO psfi, uint cbFileInfo, uint uFlags);
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern int SHParseDisplayName(string pszName, IntPtr pbc, out IntPtr ppidl, uint sfgaoIn, out uint psfgaoOut);
  [DllImport("ole32.dll")] public static extern void CoTaskMemFree(IntPtr pv);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr hIcon);
  public static void Save(string name, string output) {
    IntPtr pidl; uint attr;
    if (SHParseDisplayName(name, IntPtr.Zero, out pidl, 0, out attr) != 0) { WriteName(""); return; }
    SHFILEINFO info = new SHFILEINFO();
    IntPtr res = SHGetFileInfo(pidl, 0, ref info, (uint)Marshal.SizeOf(info), 0x308);
    CoTaskMemFree(pidl);
    if (res == IntPtr.Zero || info.hIcon == IntPtr.Zero) { WriteName(""); return; }
    try { using (Icon icon = Icon.FromHandle(info.hIcon)) using (Bitmap bmp = icon.ToBitmap()) { bmp.Save(output, System.Drawing.Imaging.ImageFormat.Png); } } catch {}
    DestroyIcon(info.hIcon);
    WriteName(info.szDisplayName ?? "");
  }
  // 直接往标准输出写 UTF-8 原始字节：绕开控制台编码，中文显示名不会乱码
  static void WriteName(string value) {
    byte[] bytes = System.Text.Encoding.UTF8.GetBytes(value + "\n");
    using (System.IO.Stream stdout = System.Console.OpenStandardOutput()) { stdout.Write(bytes, 0, bytes.Length); }
  }
}
'@
"#;
    let values = pairs.iter().flat_map(|(input, output)| [ps_quote(input), ps_quote(output.display().to_string())]).collect::<Vec<_>>().join(",");
    let script = format!("{TYPE_DEF}$p=@({values}); for($i=0;$i -lt $p.Count;$i+=2){{ [SuqiShellIcon]::Save($p[$i],$p[$i+1]) }}");
    let script_file = std::env::temp_dir().join(format!("suqi-shell-icons-{}-{}.ps1", std::process::id(), now()));
    // PowerShell 5.1 默认按 ANSI 读取脚本文件，必须带 UTF-8 BOM（同 extract_icons）
    let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    if fs::write(&script_file, bytes).is_err() { return vec![String::new(); pairs.len()]; }
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script_file);
    hide_console(&mut command);
    let output = command.output();
    let _ = fs::remove_file(&script_file);
    let mut names: Vec<String> = match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout).lines().map(|line| line.trim().to_string()).collect(),
        Err(_) => Vec::new(),
    };
    names.resize(pairs.len(), String::new());
    names
}

/// 把当前显示在桌面的系统图标并入扫描结果：条目类型为 system，启动目标用 shell: 命名空间字符串，
/// 导入后经 normalize_target 的 shell: 分支原样保存，open_item 里由 explorer 解析打开
#[cfg(windows)]
fn append_desktop_shell_entries(entries: &mut Vec<DesktopEntry>) {
    let icon_dir = default_data_dir().join("scan-icons");
    let _ = fs::create_dir_all(&icon_dir);
    let specs: Vec<&ShellIconSpec> = SHELL_ICON_SPECS.iter().filter(|spec| shell_icon_shown(spec.clsid, spec.default_shown)).collect();
    if specs.is_empty() { return; }
    // 图标文件名规则与 scan_dir 一致（来源字符串的 sha256）；每次扫描都重新提取，回收站空/满等图标变化能跟进，同时拿到系统本地化显示名
    let pairs: Vec<(String, PathBuf)> = specs.iter().map(|spec| {
        let mut hasher = Sha256::new(); hasher.update(spec.target.as_bytes());
        (spec.target.to_string(), icon_dir.join(format!("{}.png", hex::encode(hasher.finalize()))))
    }).collect();
    let names = extract_shell_icons(&pairs);
    for ((spec, (_, icon_file)), display_name) in specs.iter().zip(pairs.iter()).zip(names) {
        let name = if display_name.is_empty() || spec.prefer_fallback_name { spec.fallback_name.to_string() } else { display_name };
        entries.push(DesktopEntry {
            name: name.clone(),
            source_path: spec.target.to_string(),
            target: spec.target.to_string(),
            is_shortcut: false,
            entry_type: "system".into(),
            scan_icon_path: icon_file.exists().then(|| display_path(icon_file.display().to_string())),
            folder_name: None,
            rel_path: vec![name],
        });
    }
}

/// 扫描桌面（默认入口）。声明为 async 以脱离主线程执行，避免耗时扫描阻塞界面（macOS 上曾表现为「点击扫描没反应」）
#[tauri::command]
async fn scan_desktop(recursive: Option<bool>) -> Result<Vec<DesktopEntry>, String> {
    let desktop = dirs::desktop_dir().or_else(|| dirs::home_dir().map(|p| p.join("Desktop"))).ok_or("无法定位桌面目录")?;
    let mut entries = scan_dir(&desktop, recursive.unwrap_or(false))?;
    // Windows 下把显示在桌面的系统图标（此电脑 / 回收站等虚拟对象）一并并入，再统一排序
    #[cfg(windows)]
    append_desktop_shell_entries(&mut entries);
    sort_scan_entries(&mut entries);
    Ok(entries)
}

/// 扫描用户手动选择的文件夹（与桌面扫描同规则）
#[tauri::command]
async fn scan_folder(dir: String, recursive: Option<bool>) -> Result<Vec<DesktopEntry>, String> {
    let path = PathBuf::from(dir.trim());
    if !path.is_dir() { return Err("请选择一个有效的文件夹".into()); }
    scan_dir(&path, recursive.unwrap_or(false))
}

/// 系统「开始菜单 / 应用程序」目录（当前用户 + 所有用户），不存在的直接忽略
fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Some(appdata) = dirs::data_dir() {
            dirs.push(appdata.join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
        }
        if let Ok(program_data) = std::env::var("ProgramData") {
            dirs.push(PathBuf::from(program_data).join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Applications"));
        // macOS Catalina 起系统自带应用（App Store、备忘录、短信等）迁到了 /System/Applications
        dirs.push(PathBuf::from("/System/Applications"));
        if let Some(home) = dirs::home_dir() { dirs.push(home.join("Applications")); }
    }
    dirs.retain(|dir| dir.is_dir());
    dirs
}

/// 扫描系统开始菜单 / macOS 应用程序目录（当前用户 + 所有用户）。async 理由同 scan_desktop
#[tauri::command]
async fn scan_start_menu(recursive: Option<bool>) -> Result<Vec<DesktopEntry>, String> {
    let dirs = start_menu_dirs();
    if dirs.is_empty() { return Err("未找到系统应用目录".into()); }
    let recursive = recursive.unwrap_or(false);
    let mut entries = Vec::new();
    for dir in dirs { entries.append(&mut scan_dir(&dir, recursive)?); }
    sort_scan_entries(&mut entries);
    // 同一快捷方式可能同时存在于用户与全局开始菜单，按来源路径去重
    let mut seen = HashSet::new();
    entries.retain(|entry| seen.insert(entry.source_path.to_lowercase()));
    Ok(entries)
}

/// 取路径所在文件夹的名称，用于导入时「按文件夹名称分组」
fn parent_folder_name(path: &str) -> Option<String> {
    Path::new(path).parent()?.file_name().map(|name| name.to_string_lossy().to_string())
}

/// 按名称查找分组，不存在则新建并返回其 id
fn group_id_by_name(store: &mut Store, name: &str) -> String {
    if let Some(group) = store.groups.iter().find(|group| group.name == name) { return group.id.clone(); }
    let group = Group { id: new_id(), name: name.to_string(), sort_order: store.groups.iter().map(|group| group.sort_order).max().unwrap_or(-1) + 1 };
    store.groups.push(group.clone());
    group.id
}

#[tauri::command]
fn import_scanned(data: State<'_, AppData>, entries: Vec<DesktopEntry>, group_id: Option<String>, group_by_folder: bool) -> Result<ImportResult, String> {
    let shortcut_paths: Vec<PathBuf> = entries.iter().filter(|e| e.is_shortcut).map(|e| PathBuf::from(&e.source_path)).collect();
    let shortcut_targets = resolve_shortcuts(&shortcut_paths);
    let mut shortcut_index = 0;
    // (名称, 解析后的真实目标, 所在文件夹, 原始快捷方式路径)
    let mut prepared: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    // 失败明细：记录条目名、来源路径与失败原因，导入结束后统一列出
    let mut invalids: Vec<ImportSkip> = Vec::new();
    for entry in entries {
        if entry.name.trim().is_empty() {
            invalids.push(ImportSkip { name: "（未命名）".into(), target: entry.source_path.clone(), group_name: None, reason: Some("名称为空".into()) });
            continue;
        }
        let folder = parent_folder_name(&entry.source_path);
        // 快捷方式条目同时保留原始 .lnk 路径，启动时经它还原 目标+参数+工作目录
        let (candidate, lnk_path) = if entry.is_shortcut {
            let target = shortcut_targets.get(shortcut_index).cloned().unwrap_or_default();
            shortcut_index += 1; (target, Some(entry.source_path.clone()))
        } else { (entry.source_path.clone(), None) };
        match normalize_target(&candidate, false) {
            Ok(target) => prepared.push((entry.name, target, folder, lnk_path)),
            Err(error) if lnk_path.is_some() => {
                // 快捷方式目标解析失败（目标为空、已被删除等特殊链接）：退化为按 .lnk 自身导入，
                // 启动时仍经 lnk_path 由 Shell 处理，避免误报「无效」
                match fs::canonicalize(&entry.source_path) {
                    Ok(path) => prepared.push((entry.name, display_path(path.display().to_string()), folder, lnk_path)),
                    Err(_) => invalids.push(ImportSkip { name: entry.name, target: entry.source_path, group_name: None, reason: Some(error) }),
                }
            }
            Err(error) => invalids.push(ImportSkip { name: entry.name, target: entry.source_path, group_name: None, reason: Some(error) }),
        }
    }
    let mut store = data.store.lock().unwrap();
    let mut known: HashSet<String> = store.items.iter().map(|item| item.target.to_lowercase()).collect();
    let mut added = 0;
    // 重复明细：记录与哪个已有条目冲突及其所在分组（同批次先入组的条目也能取到分组名）
    let mut duplicates: Vec<ImportSkip> = Vec::new();
    // 成功导入的快捷方式原始路径（仅 .lnk；macOS 的「快捷方式」是 .app 应用包本体，绝不收集）：供导入完成后可选删除
    let mut imported_shortcuts: Vec<String> = Vec::new();
    let mut order = store.items.iter().map(|item| item.sort_order).max().unwrap_or(-1) + 1;
    for (name, target, folder, lnk_path) in prepared {
        if !known.insert(target.to_lowercase()) {
            // 旧条目缺少快捷方式来源时，本次扫描按真实目标匹配补齐，顺带修复历史数据
            if let Some(lnk_path) = lnk_path {
                if let Some(old) = store.items.iter_mut().find(|x| x.target.eq_ignore_ascii_case(&target)) {
                    if old.lnk_path.is_none() { old.lnk_path = Some(lnk_path); }
                }
            }
            let group_name = store.items.iter().find(|x| x.target.eq_ignore_ascii_case(&target)).map(|item| item_group_label(&store, item));
            duplicates.push(ImportSkip { name, target, group_name, reason: None });
            continue;
        }
        // 勾选「按文件夹名称分组」时优先使用条目所在文件夹名（不存在则新建），取不到时回退到当前分组
        let target_group = if group_by_folder {
            folder.filter(|name| !name.trim().is_empty()).map(|name| group_id_by_name(&mut store, &name)).or_else(|| group_id.clone())
        } else { group_id.clone() };
        if let Some(lnk) = &lnk_path {
            // 只收集真正的 .lnk 文件：macOS 的「快捷方式」是 .app 应用包本体，绝不能删
            if Path::new(lnk).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) { imported_shortcuts.push(lnk.clone()); }
        }
        store.items.push(LauncherItem { id: new_id(), name: name.clone(), keyword: name, target, lnk_path, group_id: target_group, is_encrypted: false, is_cmd: false, icon_path: None, sort_order: order, created_at: now(), description: String::new() });
        order += 1; added += 1;
    }
    persist(&data, &store)?;
    Ok(ImportResult { added, skipped_duplicates: duplicates.len(), skipped_invalid: invalids.len(), duplicates, invalids, imported_shortcuts })
}

/// 导入成功后删除源快捷方式：仅限 .lnk 文件本身，绝不触碰其指向的目标文件；
/// 同时清掉对应条目的 lnk_path，之后启动直接打开已解析的真实目标（open_item 本就有 lnk 缺失回退）
#[tauri::command]
fn delete_shortcut_files(data: State<'_, AppData>, paths: Vec<String>) -> Result<DeleteShortcutsResult, String> {
    let mut deleted_paths: Vec<String> = Vec::new();
    let mut failures: Vec<ImportSkip> = Vec::new();
    for raw in paths {
        let path = PathBuf::from(&raw);
        let name = || path.file_stem().or_else(|| path.file_name()).unwrap_or_default().to_string_lossy().to_string();
        // 安全闸：不是 .lnk 快捷方式文件的一律拒绝删除，避免误删目标程序 / 文件
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) || !path.is_file() {
            failures.push(ImportSkip { name: name(), target: raw.clone(), group_name: None, reason: Some("不是有效的快捷方式文件".into()) });
            continue;
        }
        match fs::remove_file(&path) {
            Ok(_) => deleted_paths.push(raw),
            Err(error) => failures.push(ImportSkip { name: name(), target: raw.clone(), group_name: None, reason: Some(error.to_string()) }),
        }
    }
    if !deleted_paths.is_empty() {
        let deleted: HashSet<String> = deleted_paths.iter().map(|p| p.to_lowercase()).collect();
        let mut store = data.store.lock().unwrap();
        for item in &mut store.items {
            if item.lnk_path.as_ref().is_some_and(|lnk| deleted.contains(&lnk.to_lowercase())) { item.lnk_path = None; }
        }
        persist(&data, &store)?;
    }
    Ok(DeleteShortcutsResult { deleted: deleted_paths.len(), failures })
}

// ---------- 还原启动项到文件夹 ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    /// 导出目标文件夹
    dir: String,
    /// 是否按分组创建子文件夹（「未分组」始终直接放在根目录）
    group_folders: bool,
    /// 勾选的启动项 id
    item_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportResult {
    exported: usize,
    /// 文件 / 文件夹类型的启动项不导出，逐条列出
    skipped: Vec<ImportSkip>,
    failures: Vec<ImportSkip>,
    /// 导出目标文件夹（结果页「打开文件夹查看」用）
    dir: String,
}

/// 清洗为合法的文件 / 文件夹名：替换 Windows 保留字符与控制字符，去掉首尾空格与点
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == ' ' || c == '.').to_string();
    if cleaned.is_empty() { "未命名".into() } else { cleaned }
}

/// 同目录下重名时追加序号：name.ext、name (2).ext …（同时避开磁盘上已存在的同名文件）
fn unique_dest(used: &mut HashMap<PathBuf, HashSet<String>>, folder: &Path, name: &str, ext: &str) -> PathBuf {
    let occupied = used.entry(folder.to_path_buf()).or_default();
    let mut candidate = if ext.is_empty() { name.to_string() } else { format!("{name}.{ext}") };
    let mut counter = 2;
    while !occupied.insert(candidate.to_lowercase()) || folder.join(&candidate).exists() {
        candidate = if ext.is_empty() { format!("{name} ({counter})") } else { format!("{name} ({counter}).{ext}") };
        counter += 1;
    }
    folder.join(candidate)
}

/// 条目导出时所在的子文件夹名：未分组返回 None（放根目录）；加密分组固定为「加密」
fn export_subfolder(item: &LauncherItem, groups: &[Group], group_folders: bool) -> Option<String> {
    if !group_folders { return None; }
    let id = item.group_id.as_deref()?;
    if id == ENCRYPTED_GROUP { return Some("加密".into()); }
    groups.iter().find(|group| group.id == id).map(|group| sanitize_filename(&group.name))
}

fn export_skip(item: &LauncherItem, reason: &str) -> ImportSkip {
    ImportSkip { name: item.name.clone(), target: item.target.clone(), group_name: None, reason: Some(reason.into()) }
}

/// 写入一个导出文件（自动创建所属分组文件夹）
fn write_export_file(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建文件夹失败：{e}"))?; }
    fs::write(dest, bytes).map_err(|e| format!("写入文件失败：{e}"))
}

/// 复制现成文件（原始 .lnk / .url）到导出目录
fn copy_export_file(source: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建文件夹失败：{e}"))?; }
    fs::copy(source, dest).map_err(|e| format!("复制文件失败：{e}"))?;
    Ok(())
}

/// Windows 批量创建 .lnk 快捷方式：经 PowerShell WScript.Shell，返回值与 jobs 按序对齐。
/// 脚本写入临时文件执行：避免命令行长度上限与中文路径乱码（同 extract_icons 的做法）
#[cfg(windows)]
fn create_shortcuts(jobs: &[(PathBuf, String)]) -> Vec<bool> {
    if jobs.is_empty() { return Vec::new(); }
    let values = jobs.iter().flat_map(|(lnk, target)| [ps_quote(lnk.display().to_string()), ps_quote(target)]).collect::<Vec<_>>().join(",");
    let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; $w=New-Object -ComObject WScript.Shell; $p=@({values}); for($i=0;$i -lt $p.Count;$i+=2){{ try {{ $s=$w.CreateShortcut($p[$i]); $s.TargetPath=$p[$i+1]; $s.Save(); '1' }} catch {{ '0' }} }}");
    let script_file = std::env::temp_dir().join(format!("suqi-links-{}-{}.ps1", std::process::id(), now()));
    // PowerShell 5.1 默认按 ANSI 读取脚本文件，含中文路径时必须带 UTF-8 BOM 才不会乱码
    let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    if fs::write(&script_file, bytes).is_err() { return jobs.iter().map(|_| false).collect(); }
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script_file);
    hide_console(&mut command);
    let output = command.output();
    let _ = fs::remove_file(&script_file);
    match output {
        Ok(output) => {
            let lines: Vec<String> = String::from_utf8_lossy(&output.stdout).lines().map(|line| line.trim().to_string()).collect();
            jobs.iter().enumerate().map(|(index, _)| lines.get(index).map(String::as_str) == Some("1")).collect()
        }
        Err(_) => jobs.iter().map(|_| false).collect(),
    }
}

/// macOS / Linux 下用符号链接代替 .lnk
#[cfg(all(not(windows), unix))]
fn create_link(target: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建文件夹失败：{e}"))?; }
    std::os::unix::fs::symlink(target, dest).map_err(|e| format!("创建链接失败：{e}"))
}

/// XML 转义（macOS .webloc 用）
#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 导出执行体。Windows 创建 .lnk 要调 PowerShell，整体放进 spawn_blocking 避免阻塞主线程
fn export_items_inner(dir: &Path, items: &[LauncherItem], groups: &[Group], group_folders: bool) -> Result<ExportResult, String> {
    let mut used: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let mut skipped: Vec<ImportSkip> = Vec::new();
    let mut failures: Vec<ImportSkip> = Vec::new();
    let mut exported = 0;
    // Windows 待创建的 .lnk：(快捷方式路径, 真实目标, 条目名)，最后批量交给 PowerShell
    #[cfg(windows)]
    let mut link_jobs: Vec<(PathBuf, String, String)> = Vec::new();

    for item in items {
        let folder = match export_subfolder(item, groups, group_folders) {
            Some(name) => dir.join(name),
            None => dir.to_path_buf(),
        };
        let name = sanitize_filename(&item.name);
        // 1) CMD 命令 → .bat（macOS / Linux → 可执行的 .command）
        if item.is_cmd {
            #[cfg(windows)]
            let (ext, content) = ("bat", format!("@echo off\r\nchcp 65001>nul\r\n{}\r\n", item.target));
            #[cfg(not(windows))]
            let (ext, content) = ("command", format!("#!/bin/sh\n{}\n", item.target));
            let dest = unique_dest(&mut used, &folder, &name, ext);
            match write_export_file(&dest, content.as_bytes()) {
                Ok(_) => {
                    #[cfg(all(not(windows), unix))]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = fs::metadata(&dest) { let mut perm = meta.permissions(); perm.set_mode(perm.mode() | 0o755); let _ = fs::set_permissions(&dest, perm); }
                    }
                    exported += 1;
                }
                Err(error) => failures.push(ImportSkip { reason: Some(error), ..export_skip(item, "") }),
            }
            continue;
        }
        // 2) 网址 → .url（Windows Internet Shortcut；macOS → .webloc）
        if is_url(&item.target) {
            #[cfg(target_os = "macos")]
            let (ext, content) = ("webloc", format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>URL</key><string>{}</string></dict></plist>\n", xml_escape(&item.target)));
            #[cfg(not(target_os = "macos"))]
            let (ext, content) = ("url", format!("[InternetShortcut]\r\nURL={}\r\n", item.target));
            let dest = unique_dest(&mut used, &folder, &name, ext);
            match write_export_file(&dest, content.as_bytes()) {
                Ok(_) => exported += 1,
                Err(error) => failures.push(ImportSkip { reason: Some(error), ..export_skip(item, "") }),
            }
            continue;
        }
        let path = Path::new(&item.target);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
        // macOS 的 .app 是目录形态的应用包，按程序处理
        let is_app = cfg!(target_os = "macos") && ext == "app";
        // 3) 原始 .lnk 还在 → 直接复制（保留参数 / 工作目录）
        if let Some(lnk) = item.lnk_path.as_deref().filter(|p| Path::new(p).is_file()) {
            let dest = unique_dest(&mut used, &folder, &name, "lnk");
            match copy_export_file(Path::new(lnk), &dest) {
                Ok(_) => exported += 1,
                Err(error) => failures.push(ImportSkip { reason: Some(error), ..export_skip(item, "") }),
            }
            continue;
        }
        // 4) 文件夹不导出（macOS .app 应用包除外）
        if path.is_dir() && !is_app {
            skipped.push(export_skip(item, "文件夹不导出"));
            continue;
        }
        // 5) 目标本身是 .lnk / .url 文件 → 直接复制即可使用
        if matches!(ext.as_str(), "lnk" | "url") && path.is_file() {
            let dest = unique_dest(&mut used, &folder, &name, &ext);
            match copy_export_file(path, &dest) {
                Ok(_) => exported += 1,
                Err(error) => failures.push(ImportSkip { reason: Some(error), ..export_skip(item, "") }),
            }
            continue;
        }
        // 6) 可执行程序 → 创建 .lnk 快捷方式（macOS / Linux → 符号链接）
        let is_executable = matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd" | "ps1" | "msi") || is_app;
        if is_executable && (path.is_file() || is_app) {
            #[cfg(windows)]
            {
                let dest = unique_dest(&mut used, &folder, &name, "lnk");
                link_jobs.push((dest, item.target.clone(), item.name.clone()));
            }
            #[cfg(all(not(windows), unix))]
            {
                let dest = unique_dest(&mut used, &folder, &name, "");
                match create_link(&item.target, &dest) {
                    Ok(_) => exported += 1,
                    Err(error) => failures.push(ImportSkip { reason: Some(error), ..export_skip(item, "") }),
                }
            }
            continue;
        }
        // 7) 其余普通文件不导出
        skipped.push(export_skip(item, "文件不导出"));
    }

    #[cfg(windows)]
    if !link_jobs.is_empty() {
        for (lnk, _, _) in &link_jobs {
            if let Some(parent) = lnk.parent() { let _ = fs::create_dir_all(parent); }
        }
        let jobs: Vec<(PathBuf, String)> = link_jobs.iter().map(|(lnk, target, _)| (lnk.clone(), target.clone())).collect();
        let results = create_shortcuts(&jobs);
        for ((_, target, name), ok) in link_jobs.iter().zip(results) {
            if ok { exported += 1; }
            else { failures.push(ImportSkip { name: name.clone(), target: target.clone(), group_name: None, reason: Some("创建快捷方式失败".into()) }); }
        }
    }

    Ok(ExportResult { exported, skipped, failures, dir: String::new() })
}

/// 把勾选的启动项还原到指定文件夹：
/// 程序 / 快捷方式 → .lnk（原始 .lnk 还在时直接复制，保留参数与工作目录）；CMD 命令 → .bat；网址 → .url；
/// 文件 / 文件夹不生成任何内容，逐条记入 skipped。声明为 async：导出在后台线程执行，不阻塞界面。
#[tauri::command]
async fn export_items(data: State<'_, AppData>, request: ExportRequest) -> Result<ExportResult, String> {
    let dir = PathBuf::from(request.dir.trim());
    if !dir.is_dir() { return Err("请选择一个有效的导出文件夹".into()); }
    let (selected, groups) = {
        let store = data.store.lock().unwrap();
        let wanted: HashSet<&str> = request.item_ids.iter().map(String::as_str).collect();
        let mut selected: Vec<LauncherItem> = store.items.iter().filter(|item| wanted.contains(item.id.as_str())).cloned().collect();
        if selected.is_empty() { return Err("请至少选择一个启动项".into()); }
        // 加密条目必须先解锁才允许导出（前端会引导解锁，这里兜底校验）
        if !data.unlocked.load(Ordering::SeqCst) && selected.iter().any(encrypted) { return Err("加密分组请先解锁后再导出".into()); }
        selected.sort_by_key(|item| item.sort_order);
        (selected, store.groups.clone())
    };
    let group_folders = request.group_folders;
    let display = display_path(dir.display().to_string());
    let mut result = tauri::async_runtime::spawn_blocking(move || export_items_inner(&dir, &selected, &groups, group_folders))
        .await
        .map_err(|e| e.to_string())??;
    result.dir = display;
    Ok(result)
}

/// 补齐缺失的条目图标。声明为 async + spawn_blocking：图标提取要调用外部命令
/// （Windows PowerShell / macOS sips），同步命令会在主线程执行导致界面卡死（导入后刷新图标时尤其明显）。
#[tauri::command]
async fn item_icons(data: State<'_, AppData>, state: State<'_, ConfigState>) -> Result<Vec<ItemIcon>, String> {
    let items = data.store.lock().unwrap().items.clone();
    let cache_dir = state.data_dir.join("icons");
    let _ = fs::create_dir_all(&cache_dir);
    #[cfg(any(windows, target_os = "macos"))]
    {
        let pairs = items.iter().filter_map(|item| {
            if item.icon_path.is_some() || item.is_cmd || is_url(&item.target) { return None; }
            let output = cache_dir.join(format!("{}.png", item.id));
            (!output.exists()).then(|| (PathBuf::from(&item.target), output))
        }).collect::<Vec<_>>();
        let _ = tauri::async_runtime::spawn_blocking(move || extract_icons(&pairs)).await;
    }
    let icons = items.into_iter().map(|item| {
        let cached = cache_dir.join(format!("{}.png", item.id));
        ItemIcon { id: item.id, icon_path: item.icon_path.or_else(|| cached.exists().then(|| display_path(cached.display().to_string()))) }
    }).collect();
    Ok(icons)
}

// ---------- 设置页命令 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResult { items: Vec<LauncherItem>, groups: Vec<Group>, password_set: bool, unlocked: bool }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchText { id: String, name: String, full_pinyin: String, initials: String, keyword: String, keyword_full_pinyin: String, keyword_initials: String }

#[tauri::command]
fn get_config(state: State<'_, ConfigState>) -> AppConfig { state.config.lock().unwrap().clone() }

#[tauri::command]
fn save_config(state: State<'_, ConfigState>, app: tauri::AppHandle, config: AppConfig) -> Result<(), String> {
    *state.config.lock().unwrap() = config.clone();
    save_config_file(&config)?;
    register_hotkey(&app, &config.hotkey)?;
    apply_autostart(&app, config.autostart);
    if let Some(win) = app.get_webview_window("main") { let _ = win.set_always_on_top(config.always_on_top); }
    sync_tray_pinned(&app, config.always_on_top);
    apply_window_effect_all(&app, &config.window_effect, config.theme != "light");
    let _ = app.emit("config-changed", config);
    Ok(())
}

#[tauri::command]
fn get_db_info(data: State<'_, AppData>, _state: State<'_, ConfigState>) -> serde_json::Value {
    let store = data.store.lock().unwrap();
    let path = db_path();
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    serde_json::json!({
        "path": display_path(path.display().to_string()),
        "size": size,
        "itemCount": store.items.len(),
        "groupCount": store.groups.len(),
        "passwordSet": store.password.is_some(),
    })
}

#[tauri::command]
fn show_panel(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if !win.is_visible().unwrap_or(false) {
            // 先通知前端再显示窗口：前端可在面板可见前清空搜索并重绘，避免先闪出旧内容
            let _ = app.emit("panel-shown", ());
            let _ = win.show(); let _ = win.set_focus();
            return;
        }
    }
    let _ = app.emit("panel-shown", ());
}

#[tauri::command]
fn hide_panel(app: tauri::AppHandle) {
    lock_encrypted_if_needed(&app);
    if let Some(win) = app.get_webview_window("main") { let _ = win.hide(); }
}

#[tauri::command]
fn set_panel_pinned(state: State<'_, ConfigState>, app: tauri::AppHandle, pinned: bool) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.always_on_top = pinned;
    let snapshot = cfg.clone();
    drop(cfg);
    save_config_file(&snapshot)?;
    if let Some(win) = app.get_webview_window("main") { let _ = win.set_always_on_top(pinned); }
    sync_tray_pinned(&app, pinned);
    let _ = app.emit("config-changed", snapshot);
    Ok(())
}

#[tauri::command]
fn get_panel_pinned(state: State<'_, ConfigState>) -> bool {
    state.config.lock().unwrap().always_on_top
}

// 同步托盘菜单「钉住面板」勾选态
fn sync_tray_pinned(app: &tauri::AppHandle, pinned: bool) {
    let state = app.state::<ConfigState>();
    let guard = state.tray_toggle.lock().unwrap();
    if let Some(item) = guard.as_ref() {
        let _ = item.set_checked(pinned);
    }
}

#[tauri::command]
fn set_window_effect(app: tauri::AppHandle, state: State<'_, ConfigState>, effect: String) -> Result<(), String> {
    const VALID: [&str; 4] = ["default", "acrylic", "vibrancy", "mica"];
    if !VALID.contains(&effect.as_str()) { return Err(format!("未知的窗口效果: {effect}")); }
    let mut cfg = state.config.lock().unwrap();
    cfg.window_effect = effect;
    let cfg_clone = cfg.clone();
    save_config_file(&cfg_clone)?;
    apply_window_effect_all(&app, &cfg_clone.window_effect, cfg_clone.theme != "light");
    let _ = app.emit("config-changed", cfg_clone);
    Ok(())
}

#[tauri::command]
fn get_config_dir(_state: State<'_, ConfigState>) -> String {
    // 返回「当前生效」的配置文件目录（可能来自指针文件指向的自定义目录），
    // 而非启动时的默认目录；否则重启 / 更新后会显示成默认路径，看起来像配置丢失
    let dir = effective_config_path().parent().map(PathBuf::from).unwrap_or_else(default_data_dir);
    display_path(dir.display().to_string())
}

#[tauri::command]
fn get_db_dir(_state: State<'_, ConfigState>) -> String {
    display_path(effective_db_dir().display().to_string())
}

#[tauri::command]
fn set_data_dirs(state: State<'_, ConfigState>, config_dir: Option<String>, db_dir: Option<String>, action: String) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    let old_db_dir = effective_db_dir();

    if let Some(ref dir) = config_dir {
        if !dir.is_empty() {
            let new_path = PathBuf::from(dir).join("config.json");
            if let Some(parent) = new_path.parent() { fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?; }
            if action == "move" {
                let old = effective_config_path();
                if old != new_path { let _ = fs::rename(&old, &new_path); }
            }
            if let Some(parent) = pointer_path().parent() { fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败：{e}"))?; }
            fs::write(pointer_path(), dir).map_err(|e| format!("写入指针文件失败：{e}"))?;
            cfg.config_dir = Some(dir.clone());
        }
    }

    if let Some(ref dir) = db_dir {
        if !dir.is_empty() {
            let new_db_dir = PathBuf::from(dir);
            fs::create_dir_all(&new_db_dir).map_err(|e| format!("创建数据库目录失败：{e}"))?;
            if action == "move" && old_db_dir != new_db_dir {
                let old_db = old_db_dir.join(DB_FILE);
                let new_db = new_db_dir.join(DB_FILE);
                if old_db.exists() { let _ = fs::rename(&old_db, &new_db); }
            }
            cfg.db_dir = Some(dir.clone());
        }
    }

    if action == "reset" {
        cfg.config_dir = None;
        cfg.db_dir = None;
        let _ = fs::remove_file(pointer_path());
    }

    save_config_file(&cfg)?;
    Ok(())
}

// ---------- 面板背景图 ----------
// 源图复制为配置目录下的 suqi-bg-src.<ext>；设置页烘焙（旋转+选区）后的成品
// 存为 suqi-bg.png，主面板直接按 base64 读取应用。换目录时随配置文件一起迁移。

const BG_SRC_STEM: &str = "suqi-bg-src";
const BG_CACHE_NAME: &str = "suqi-bg.png";
const BG_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// 背景图存放目录：跟随当前生效的配置文件所在目录
fn bg_dir() -> PathBuf {
    effective_config_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_data_dir)
}

/// 目录下已存在的背景源图（按支持的扩展名枚举）
fn bg_src_files(dir: &Path) -> Vec<PathBuf> {
    BG_EXTS
        .iter()
        .map(|e| dir.join(format!("{BG_SRC_STEM}.{e}")))
        .filter(|p| p.exists())
        .collect()
}

#[derive(Serialize)]
struct BgImageData { b64: String, mime: String }

/// 选择背景图：把源图复制进配置目录（换图后旧的烘焙缓存作废）
#[tauri::command]
fn import_background_source(path: String) -> Result<(), String> {
    let src = PathBuf::from(&path);
    if !src.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    if !BG_EXTS.contains(&ext.as_str()) {
        return Err("仅支持 png / jpg / webp / bmp / gif 图片".into());
    }
    let size = fs::metadata(&src).map_err(|e| e.to_string())?.len();
    if size > 30 * 1024 * 1024 {
        return Err("图片过大（超过 30MB），请换一张".into());
    }
    let dir = bg_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for f in bg_src_files(&dir) {
        let _ = fs::remove_file(f);
    }
    let _ = fs::remove_file(dir.join(BG_CACHE_NAME));
    fs::copy(&src, dir.join(format!("{BG_SRC_STEM}.{ext}")))
        .map_err(|e| format!("复制图片失败: {e}"))?;
    Ok(())
}

/// 读取背景源图（设置页重新编辑选区/旋转时回显用）
#[tauri::command]
fn read_background_source() -> Option<BgImageData> {
    let f = bg_src_files(&bg_dir()).into_iter().next()?;
    let bytes = fs::read(&f).ok()?;
    let mime = match f
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        _ => return None, // bg_src_files 只枚举已知扩展名，兜底防御
    };
    Some(BgImageData { b64: B64.encode(bytes), mime: mime.to_string() })
}

/// 保存设置页烘焙后的成品背景图（base64 PNG）
#[tauri::command]
fn save_background_cache(b64: String) -> Result<(), String> {
    let bytes = B64.decode(b64).map_err(|e| format!("图片数据解码失败: {e}"))?;
    if bytes.len() > 30 * 1024 * 1024 {
        return Err("处理后的图片过大".into());
    }
    let dir = bg_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(BG_CACHE_NAME), bytes).map_err(|e| format!("写入背景图失败: {e}"))
}

/// 主面板读取烘焙后的背景图
#[tauri::command]
fn get_background_cache() -> Option<String> {
    fs::read(bg_dir().join(BG_CACHE_NAME)).ok().map(|b| B64.encode(b))
}

/// 保存背景图设置（即时落盘 + 广播，与 set_window_effect 同一模式）
#[tauri::command]
fn set_background_config(
    state: State<'_, ConfigState>,
    app: tauri::AppHandle,
    bg: BackgroundConfig,
) -> Result<(), String> {
    let snapshot = {
        let mut cfg = state.config.lock().unwrap();
        cfg.background = bg;
        cfg.clone()
    };
    save_config_file(&snapshot)?;
    let _ = app.emit("config-changed", snapshot);
    Ok(())
}

/// 清除背景图：删除源图与缓存，并重置背景设置为默认
#[tauri::command]
fn clear_background_image(state: State<'_, ConfigState>, app: tauri::AppHandle) -> Result<(), String> {
    let dir = bg_dir();
    for f in bg_src_files(&dir) {
        let _ = fs::remove_file(f);
    }
    let _ = fs::remove_file(dir.join(BG_CACHE_NAME));
    let snapshot = {
        let mut cfg = state.config.lock().unwrap();
        cfg.background = BackgroundConfig::default();
        cfg.clone()
    };
    save_config_file(&snapshot)?;
    let _ = app.emit("config-changed", snapshot);
    Ok(())
}

// ---------- 应用内更新 ----------
// NSIS 安装版：把安装包流式下载到系统「下载」目录，校验 SHA-256 后由用户确认运行安装程序
// （NSIS 会装回注册表记录的原目录）。
// 绿色便携版：单文件 exe，配置/数据库都在 exe 同目录，不能装到别处——下载便携版 exe 到
// 本目录 → 校验 SHA-256 → 退出后由辅助进程替换旧 exe 并重启。

/// 判断当前是安装版还是便携版：注册表存在 NSIS 卸载项即为安装版
#[tauri::command]
fn update_install_kind() -> String {
    #[cfg(target_family = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        // NSIS 卸载项以产品名 / 应用标识符命名，逐个探测
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for key in ["速启", "suqi", "com.suqi.launcher"] {
                let path =
                    format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key}");
                if RegKey::predef(hive).open_subkey(&path).is_ok() {
                    return "installed".to_string();
                }
            }
        }
        "portable".to_string()
    }
    #[cfg(not(target_family = "windows"))]
    "installed".to_string()
}

/// 计算便携版 exe 的下载目标路径：默认 exe 同目录的 suqi_new.exe（等待替换）；
/// 指定目录时为该目录下的 suqi.exe（用户选择「下载到其他目录」）
#[tauri::command]
fn portable_update_begin(target_dir: Option<String>) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("无法确定程序目录")?.to_path_buf();
    let (dir, name) = match target_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(d) => (PathBuf::from(d), "suqi.exe"),
        None => (exe_dir, "suqi_new.exe"),
    };
    fs::create_dir_all(&dir).map_err(|e| format!("无法写入目录 {}：{e}", dir.display()))?;
    let path = dir.join(name);
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    Ok(path.to_string_lossy().to_string())
}

/// 构建 ureq Agent：优先尝试系统/环境代理，代理不可用时回退直连
fn ureq_agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new();
    // 优先读环境变量（Clash/V2Ray 等命令行代理会写 HTTPS_PROXY）
    if let Ok(proxy) = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
    {
        if let Ok(proxy) = ureq::Proxy::new(&proxy) {
            return builder.proxy(proxy).build();
        }
    }
    // Windows 下尝试读注册表系统代理
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings") {
            let enable: u32 = hkcu.get_value("ProxyEnable").unwrap_or(0);
            let server: String = hkcu.get_value("ProxyServer").unwrap_or_default();
            if enable != 0 && !server.is_empty() {
                // ProxyServer 格式可能是 "http=host:port;https=host:port" 或 "host:port"
                let server = if server.contains('=') {
                    server.split(';').find_map(|s| {
                        let mut kv = s.splitn(2, '=');
                        match (kv.next(), kv.next()) {
                            (Some("https"), Some(v)) => Some(v.to_string()),
                            _ => None,
                        }
                    })
                } else {
                    Some(server)
                };
                if let Some(server) = server {
                    let proxy_url = format!("http://{}", server);
                    if let Ok(proxy) = ureq::Proxy::new(&proxy_url) {
                        return builder.proxy(proxy).build();
                    }
                }
            }
        }
    }
    builder.build()
}

/// 带代理回退的 GET：先用代理，连接类错误时自动降级直连
fn ureq_get_with_fallback(url: &str) -> Result<ureq::Response, Box<ureq::Error>> {
    let agent = ureq_agent();
    let resp = agent.get(url).call();
    match resp {
        Ok(r) => Ok(r),
        Err(ureq::Error::Transport(t)) => {
            // 代理连接失败/超时，回退直连
            eprintln!("代理连接失败，回退直连：{t}");
            ureq::AgentBuilder::new().build().get(url).call().map_err(Box::new)
        }
        Err(e) => Err(Box::new(e)),
    }
}

/// 流式下载更新包到指定路径，进度通过 portable-update-progress 事件回传
#[tauri::command]
async fn portable_update_download(
    app: tauri::AppHandle,
    url: String,
    path: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let resp = ureq_get_with_fallback(&url).map_err(|e| format!("下载失败：{e}"))?;
        let total: u64 = resp
            .header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut reader = resp.into_reader();
        let mut file = fs::File::create(&path).map_err(|e| format!("无法创建文件 {path}：{e}"))?;
        let mut buf = [0u8; 65536];
        let mut sent: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            sent += n as u64;
            let _ = app.emit(
                "portable-update-progress",
                serde_json::json!({ "sent": sent, "total": total }),
            );
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 拉取文本资源（用于读取 Release 的 sha256sums 校验文件）
#[tauri::command]
async fn http_get_text(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ureq_get_with_fallback(&url)
            .map_err(|e| format!("请求失败：{e}"))?
            .into_string()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 校验下载文件的 SHA-256（与 Release 的 sha256sums 比对，防止下载损坏/被篡改）
#[tauri::command]
fn portable_update_verify(path: String, expected_sha256: String) -> Result<bool, String> {
    let data = fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
    let hash = hex::encode(Sha256::digest(&data));
    Ok(hash.eq_ignore_ascii_case(expected_sha256.trim()))
}

/// 退出本进程，由辅助进程等待退出后替换 exe 并重启新版。
/// 辅助进程 = 自身 exe 拷到 %TEMP% 的副本：GUI 子系统天然无控制台窗口。
#[tauri::command]
fn portable_update_apply(app: tauri::AppHandle, new_path: String) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = std::env::temp_dir().join("suqi-updater.exe");
    fs::copy(&exe, &helper).map_err(|e| format!("创建更新辅助程序失败：{e}"))?;
    let mut cmd = Command::new(&helper);
    cmd.arg("--apply-update").arg(&new_path).arg(&exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    hide_console(&mut cmd);
    cmd.spawn().map_err(|e| format!("启动更新辅助程序失败：{e}"))?;
    app.exit(0);
    Ok(())
}

/// 便携版更新辅助进程入口：等旧进程退出（exe 文件锁释放）后用新 exe 覆盖旧 exe，再启动新版。
/// 运行在 %TEMP% 的副本上，与替换的源/目标路径都不冲突。
#[cfg(windows)]
fn apply_update_helper(new_path: &str, old_path: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match fs::rename(new_path, old_path) {
            Ok(_) => break,
            // 旧进程尚未退出时目标 exe 被占用，rename 失败，稍候重试
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(400))
            }
            // 超时放弃：新 exe 保留在原位，用户可手动替换
            Err(_) => return,
        }
    }
    let mut cmd = Command::new(old_path);
    if let Some(dir) = Path::new(old_path).parent() {
        cmd.current_dir(dir);
    }
    hide_console(&mut cmd);
    let _ = cmd.spawn();
}

/// 安装包的下载目标路径：系统「下载」目录下的安装包文件
#[tauri::command]
fn installer_update_path(filename: String) -> Result<String, String> {
    let dir = dirs::download_dir().ok_or("找不到系统下载目录")?;
    let path = dir.join(filename);
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    Ok(path.to_string_lossy().to_string())
}

/// 运行下载好的 NSIS 安装包并退出本程序（安装程序装回原目录后会重启新版）
#[tauri::command]
fn installer_update_run(app: tauri::AppHandle, path: String) -> Result<(), String> {
    // 无窗口启动安装程序，避免安装过程闪现控制台黑窗
    let mut cmd = Command::new(&path);
    hide_console(&mut cmd);
    cmd.spawn().map_err(|e| format!("启动安装程序失败：{e}"))?;
    app.exit(0);
    Ok(())
}

/// 在系统文件管理器中定位文件（Windows 选中文件；macOS 访达中显示）；传入目录时直接打开该目录
#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("文件不存在（可能已被移动或删除）".to_string());
    }
    let is_dir = p.is_dir();
    #[cfg(target_family = "windows")]
    {
        let mut cmd = Command::new("explorer");
        if is_dir {
            cmd.arg(&path);
        } else {
            cmd.arg(format!("/select,{path}"));
        }
        cmd.spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        if !is_dir {
            cmd.arg("-R");
        }
        cmd.arg(&path).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("当前平台不支持".to_string())
}

// ---------- 窗口控制 ----------

#[tauri::command]
fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> { open_settings_window_internal(&app) }

fn open_settings_window_internal(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") { let _ = w.show(); let _ = w.set_focus(); }
    Ok(())
}

#[tauri::command]
fn set_always_on_top(state: State<'_, ConfigState>, app: tauri::AppHandle, enable: bool) -> Result<(), String> {
    set_panel_pinned(state, app, enable)
}

// ---------- 托盘 / 快捷键 / 自启 ----------

fn parse_hotkey(s: &str) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in s.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" => mods |= Modifiers::ALT,
            "win" | "super" | "meta" | "cmd" => mods |= Modifiers::SUPER,
            other => code = code_from_name(other),
        }
    }
    code.map(|c| Shortcut::new(Some(mods), c))
}

fn code_from_name(name: &str) -> Option<Code> {
    let upper = name.to_uppercase();
    Some(match upper.as_str() {
        "`" | "~" | "BACKQUOTE" => Code::Backquote,
        "SPACE" => Code::Space,
        "ESCAPE" => Code::Escape,
        "ENTER" | "RETURN" => Code::Enter,
        "TAB" => Code::Tab,
        "UP" => Code::ArrowUp, "DOWN" => Code::ArrowDown, "LEFT" => Code::ArrowLeft, "RIGHT" => Code::ArrowRight,
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        "F1" => Code::F1, "F2" => Code::F2, "F3" => Code::F3, "F4" => Code::F4,
        "F5" => Code::F5, "F6" => Code::F6, "F7" => Code::F7, "F8" => Code::F8,
        "F9" => Code::F9, "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        _ => return None,
    })
}

fn register_hotkey(app: &tauri::AppHandle, hotkey: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if !office_plugin::allowed("ALLOW_GLOBAL_SHORTCUT") { return Ok(()); }
    let sc = parse_hotkey(hotkey).ok_or_else(|| format!("无法识别的快捷键: {hotkey}"))?;
    gs.register(sc).map_err(|e| format!("注册快捷键失败: {e}"))
}

fn apply_autostart(app: &tauri::AppHandle, enable: bool) {
    if enable && !office_plugin::allowed("ALLOW_AUTOSTART") { return; }
    let mgr = app.autolaunch();
    let enabled = mgr.is_enabled().unwrap_or(false);
    if enable && !enabled { let _ = mgr.enable(); } else if !enable && enabled { let _ = mgr.disable(); }
}

/// 面板隐藏时按配置上锁：encrypt_unlock_mode == "always" 时每次隐藏面板都要求重新输入密码
fn lock_encrypted_if_needed(app: &tauri::AppHandle) {
    let mode = app.state::<ConfigState>().config.lock().unwrap().encrypt_unlock_mode.clone();
    if mode == "always" {
        if let Some(data) = app.try_state::<AppData>() {
            data.unlocked.store(false, Ordering::SeqCst);
        }
    }
}

fn hide_panel_internal(app: &tauri::AppHandle) {
    lock_encrypted_if_needed(app);
    if let Some(w) = app.get_webview_window("main") { let _ = w.hide(); }
}

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            // 面板已显示但被其他窗口盖住（未聚焦）时，按配置决定置前还是隐藏
            let focused = win.is_focused().unwrap_or(true);
            let behavior = app.try_state::<ConfigState>()
                .map(|state| state.config.lock().unwrap().hotkey_unfocused_behavior.clone())
                .unwrap_or_else(|| "front".into());
            if !focused && behavior != "hide" { let _ = win.set_focus(); let _ = app.emit("panel-shown", ()); }
            else { hide_panel_internal(app); }
        }
        else { let _ = app.emit("panel-shown", ()); let _ = win.show(); let _ = win.set_focus(); }
    }
}

fn apply_window_effect_all(app: &tauri::AppHandle, effect: &str, dark: bool) {
    for label in ["main", "settings"] {
        if let Some(win) = app.get_webview_window(label) { apply_window_effect(&win, effect, dark); }
    }
}

fn apply_window_effect(win: &tauri::WebviewWindow, effect: &str, dark: bool) {
    #[cfg(target_os = "windows")]
    {
        let _ = window_vibrancy::clear_acrylic(win);
        let _ = window_vibrancy::clear_mica(win);
        let _ = window_vibrancy::clear_blur(win);
        match effect {
            "acrylic" => { let color = if dark { (28, 28, 44, 140) } else { (240, 241, 245, 140) }; let _ = window_vibrancy::apply_acrylic(win, Some(color)); }
            "mica" => { let _ = window_vibrancy::apply_mica(win, Some(dark)); }
            "vibrancy" => { let color = if dark { (22, 22, 34, 110) } else { (239, 241, 245, 110) }; let _ = window_vibrancy::apply_blur(win, Some(color)); }
            _ => {}
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = dark;
        let _ = window_vibrancy::clear_vibrancy(win);
        let material = match effect {
            "acrylic" => Some(window_vibrancy::NSVisualEffectMaterial::Sidebar),
            "mica" => Some(window_vibrancy::NSVisualEffectMaterial::UnderWindowBackground),
            "vibrancy" => Some(window_vibrancy::NSVisualEffectMaterial::HudWindow),
            _ => None,
        };
        if let Some(m) = material { let _ = window_vibrancy::apply_vibrancy(win, m, None, Some(10.0)); }
    }
}

// ---------- 入口 ----------

pub fn run() {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        // 便携版更新辅助进程：--apply-update <新exe> <旧exe>，替换完即退，不进入 Tauri 初始化
        if args.len() >= 4 && args[1] == "--apply-update" {
            apply_update_helper(&args[2], &args[3]);
            return;
        }
        // 正常启动时清理上次更新遗留的临时辅助程序副本（可能仍被占用，失败无碍）
        let _ = std::fs::remove_file(std::env::temp_dir().join("suqi-updater.exe"));
    }

    migrate_json_if_needed();

    let conn = open_db().expect("无法打开数据库");
    let store = load_store_from_db(&conn).expect("无法加载数据");
    let config = load_config();
    let data_dir = default_data_dir();
    let config_state = ConfigState {
        config: Mutex::new(config.clone()),
        data_dir: data_dir.clone(),
        tray_toggle: Mutex::new(None),
    };

    let hotkey = config.hotkey.clone();
    let autostart = config.autostart;
    let always_on_top = config.always_on_top;
    let remember_size = config.remember_size;
    let window_width = config.window_width;
    let window_height = config.window_height;
    let silent_start = config.silent_start;
    let theme_dark = config.theme != "light";
    let window_effect_cfg = config.window_effect.clone();

    tauri::Builder::default()
        // 单实例插件必须最先注册：重复启动时新进程直接退出，并在已运行的实例里提示
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            app.dialog()
                .message("速启已在运行，请从系统托盘或全局快捷键呼出面板。")
                .title("速启")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed { toggle_window(app); }
            })
            .build())
        .manage(AppData { store: Mutex::new(store), conn: Mutex::new(conn), unlocked: AtomicBool::new(false) })
        .manage(config_state)
        .invoke_handler(tauri::generate_handler![
            list_data, searchable_items, save_item, delete_item, delete_items, move_items, open_item, create_group, rename_group,
            delete_group, save_group_order, save_item_order, unlock_encrypted, lock_encrypted, reset_password, clear_encrypted, scan_desktop, scan_folder, scan_start_menu,
            import_scanned, delete_shortcut_files, export_items, item_icons, get_config, save_config, get_db_info, run_target,
            show_panel, hide_panel, set_panel_pinned, get_panel_pinned, set_window_effect,
            get_config_dir, get_db_dir, set_data_dirs,
            import_background_source, read_background_source, save_background_cache,
            get_background_cache, set_background_config, clear_background_image,
            update_install_kind, portable_update_begin, portable_update_download, http_get_text,
            portable_update_verify, portable_update_apply, installer_update_path, installer_update_run, reveal_path,
            open_settings_window, set_always_on_top
        ])
        .setup(move |app| {
            let show = MenuItem::with_id(app, "show", "显示/隐藏面板", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let toggle = CheckMenuItem::with_id(app, "toggle", "钉住面板", true, always_on_top, None::<&str>)?;
            {
                let state = app.state::<ConfigState>();
                *state.tray_toggle.lock().unwrap() = Some(toggle.clone());
            }
            let menu = Menu::with_items(app, &[&show, &settings, &toggle, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip(format!("速启 ({hotkey})"))
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_window(app),
                    "settings" => { let _ = open_settings_window_internal(app); }
                    "quit" => app.exit(0),
                    "toggle" => {
                        let state = app.state::<ConfigState>();
                        let new_state = { let cfg = state.config.lock().unwrap(); !cfg.always_on_top };
                        let _ = set_panel_pinned(state, app.clone(), new_state);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;
            // 快捷键注册失败（与系统或其他应用冲突）不应阻断启动，仅记录日志
            if let Err(error) = register_hotkey(app.handle(), &hotkey) {
                eprintln!("注册全局快捷键失败：{error}");
            }
            apply_autostart(app.handle(), autostart);
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_always_on_top(always_on_top);
                if remember_size && window_width > 0 && window_height > 0 {
                    let _ = win.set_size(tauri::PhysicalSize::new(window_width, window_height));
                }
                apply_window_effect(&win, &window_effect_cfg, theme_dark);
                // 窗口默认隐藏（visible: false），非静默启动时在这里统一显示，避免启动闪现
                if !silent_start { let _ = win.show(); let _ = win.set_focus(); }
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        hide_panel_internal(w.app_handle());
                    }
                });
            }
            if let Some(win) = app.get_webview_window("settings") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动速启失败");
}
