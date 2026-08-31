//! axctl —— axum 项目的一体化开发工具。
//!
//! 对标 tauri-cli 的开发体验，为 axum + Vite 项目提供
//! `dev` / `build` / `serve` / `package` / `info` 等子命令。
//!
//! 本 crate 是 CLI 二进制，注释不会进 docs.rs，统一使用中文；
//! 公开文档（docs.rs）注释仅在库 crate `axctl-core` 中保留英文。

use clap::Parser;

/// axctl 命令行入口。
#[derive(Parser)]
#[command(
    name = "axctl",
    version,
    about = "All-in-one development tool for axum projects",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// 子命令集合。
#[derive(clap::Subcommand)]
enum Command {
    /// 初始化项目（探测环境、生成配置）
    #[command(about = "Initialize a project (detect environment, generate config)")]
    Init,
    /// 开发模式：监听源码变化，自动重编译并重启 server
    #[command(about = "Development mode: watch source changes, rebuild and restart the server automatically")]
    Dev,
    /// 生产构建：构建前端产物 + cargo release 构建
    #[command(about = "Production build: build frontend assets + cargo release build")]
    Build,
    /// 静态预览：直接服务构建产物（磁盘或内嵌）
    #[command(about = "Static preview: serve built assets directly from disk or embedded")]
    Serve,
    /// 打包：对接 cargo-packager 生成安装包
    #[command(about = "Package: generate installers via cargo-packager")]
    Package,
    /// 环境诊断：输出 Rust / 前端 / 系统信息
    #[command(about = "Environment diagnostics: print Rust / frontend / system info")]
    Info,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => {
            println!("axctl init —— not implemented yet");
            Ok(())
        }
        Command::Dev => {
            println!("axctl dev —— not implemented yet");
            Ok(())
        }
        Command::Build => {
            println!("axctl build —— not implemented yet");
            Ok(())
        }
        Command::Serve => {
            println!("axctl serve —— not implemented yet");
            Ok(())
        }
        Command::Package => {
            println!("axctl package —— not implemented yet");
            Ok(())
        }
        Command::Info => {
            println!("axctl info —— not implemented yet");
            Ok(())
        }
    }
}
