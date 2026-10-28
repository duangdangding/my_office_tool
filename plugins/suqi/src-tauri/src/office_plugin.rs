/// 办公工具箱协作规则：未从工具箱启动时，环境变量不存在，全部保持原行为。
pub fn allowed(rule: &str) -> bool {
    std::env::var(format!("OFFICE_PLUGIN_{rule}"))
        .map(|value| !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
        .unwrap_or(true)
}
