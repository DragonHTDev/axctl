//! axctl —— axum 项目的一体化开发工具。
//!
//! 对标 tauri-cli 的开发体验，为 axum + Vite 项目提供
//! dev / build / serve / package / info 等子命令。

use clap::Parser;

/// axctl 命令行入口。
#[derive(Parser)]
#[command(name = "axctl", version, about = "axum 项目的一体化开发工具", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// 子命令集合。
#[derive(clap::Subcommand)]
enum Command {
    /// 初始化项目（探测环境、生成配置）
    Init,
    /// 开发模式：监听源码变化，自动重编译并重启 server
    Dev,
    /// 生产构建：构建前端产物 + cargo release 构建
    Build,
    /// 静态预览：直接用内嵌/磁盘静态资源服务 dist/
    Serve,
    /// 打包：对接 cargo-packager 生成安装包
    Package,
    /// 环境诊断：输出 Rust / 前端 / 系统信息
    Info,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => {
            println!("axctl init —— 待实现");
            Ok(())
        }
        Command::Dev => {
            println!("axctl dev —— 待实现");
            Ok(())
        }
        Command::Build => {
            println!("axctl build —— 待实现");
            Ok(())
        }
        Command::Serve => {
            println!("axctl serve —— 待实现");
            Ok(())
        }
        Command::Package => {
            println!("axctl package —— 待实现");
            Ok(())
        }
        Command::Info => {
            println!("axctl info —— 待实现");
            Ok(())
        }
    }
}
