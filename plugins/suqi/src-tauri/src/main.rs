// 发布版隐藏控制台黑窗口（调试版保留控制台便于看日志；仅 Windows 适用）
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    suqi_lib::run();
}
