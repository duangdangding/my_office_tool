/// 办公工具箱协作规则：未从工具箱启动时，环境变量不存在，全部保持原行为。
pub fn allowed(rule: &str) -> bool {
    std::env::var(format!("OFFICE_PLUGIN_{rule}"))
        .map(|value| !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
        .unwrap_or(true)
}

/// 全部初始化完成后通知工具箱；独立运行时没有环境变量，不输出额外信息。
pub fn ready() {
    if std::env::var("OFFICE_TOOLBOX").as_deref() != Ok("1") {
        return;
    }
    let id = std::env::var("OFFICE_PLUGIN_ID").unwrap_or_else(|_| "unknown".into());
    println!("OFFICE_TOOLBOX_READY:{id}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}
