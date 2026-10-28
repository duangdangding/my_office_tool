use std::io::Write;

fn main() {
    println!(
        "多架构示例插件已启动：{}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("OFFICE_TOOLBOX_READY:multi-arch-plugin");
    std::io::stdout().flush().expect("输出插件就绪信号失败");

    loop {
        std::thread::park();
    }
}
