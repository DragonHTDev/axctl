//! axctl —— axum 项目的一体化开发工具。
//!
//! 对标 tauri-cli 的开发体验，为 axum + Vite 项目提供
//! `dev` / `build` / `serve` / `package` / `info` 等子命令。
//!
//! 本 crate 是 CLI 二进制，注释不会进 docs.rs，统一使用中文；
//! 公开文档（docs.rs）注释仅在库 crate `axctl-core` 中保留英文。

mod backend;
mod commands;
mod config;
mod logging;
mod process;
mod proxy;
mod vite;
mod watch_set;
mod watcher;
mod workspace;

use anyhow::Context;
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
    /// 生产构建：先 vite build 出 dist（后端 release 构建与哨兵后续接入）
    #[command(about = "Production build: build frontend assets with vite")]
    Build(commands::build::BuildArgs),
    /// 静态预览：封装 vite preview 服务构建产物（纯前端，无后端）
    #[command(about = "Preview the built frontend via vite preview (static, no backend)")]
    Serve(commands::serve::ServeArgs),
    /// 打包：对接 cargo-packager 生成安装包
    #[command(about = "Package: generate installers via cargo-packager")]
    Package,
    /// 环境诊断：输出 Rust / 前端 / 系统信息
    #[command(about = "Environment diagnostics: print Rust / frontend / system info")]
    Info,
    /// 调试：打印 workspace 解析结果与监听集合（隐藏，不进 help）
    #[command(about = "Debug: print workspace info and computed watch set (hidden)", hide = true)]
    DebugWs,
}

fn main() {
    // 日志管道最先初始化（必须在任何 tracing 调用之前）
    crate::logging::init();

    let cli = Cli::parse();
    let result = dispatch(cli.command);
    if let Err(error) = result {
        // 顶层错误统一走日志管道（Error 前缀 + anyhow 原因链）
        crate::logging::error(format!("{error:#}"));
        std::process::exit(1);
    }
}

/// 分发到各子命令的实现；错误由 main 统一打印。
fn dispatch(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Init => {
            crate::logging::warn("init is not implemented yet");
            Ok(())
        }
        Command::Dev => {
            // dev 需要 Tokio 运行时；手动创建，确保 logging::init 已完成
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::dev::run())
        }
        Command::Build(args) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::build::run(args))
        }
        Command::Serve(args) => {
            // serve 需要 Tokio 运行时（vite preview 就绪探测 + Ctrl+C 处理）
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            rt.block_on(commands::serve::run(args))
        }
        Command::Package => {
            crate::logging::warn("package is not implemented yet");
            Ok(())
        }
        Command::Info => commands::info::run(),
        Command::DebugWs => debug_ws(),
    }
}

/// 调试：在 cwd 解析 workspace 与 WatchSet，逐项打印。
///
/// 纯只读、不启动任何服务，用于验证 cargo_metadata 路径在真实项目上的表现。
fn debug_ws() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let workspace = crate::workspace::WorkspaceInfo::load(&cwd)?;

    println!("workspace root: {}", workspace.root.display());
    println!("target dir:     {}", workspace.target_dir.display());
    println!("members ({}):", workspace.members.len());
    for m in &workspace.members {
        let bins: Vec<&str> = m
            .targets
            .iter()
            .filter(|t| t.kind.iter().any(|k| matches!(k, cargo_metadata::TargetKind::Bin)))
            .map(|t| t.name.as_str())
            .collect();
        let dep_names: Vec<&str> = m
            .path_dep_ids
            .iter()
            .filter_map(|id| workspace.member_by_id(id).map(|dep| dep.name.as_str()))
            .collect();
        println!(
            "  - {} @ {}  bins=[{}]  path_deps=[{}]",
            m.name,
            m.root.display(),
            bins.join(", "),
            dep_names.join(", ")
        );
    }

    // 当前目录所属 member
    if let Some(m) = workspace.member_for_dir(&cwd) {
        println!("cwd member: {}", m.name);
    } else {
        println!("cwd member: (workspace root or none)");
    }

    // 配置读取
    let config = crate::config::load_from_project(&cwd)?;
    println!("config: {config:#?}");

    // WatchSet（如果可解析出 backend）
    match crate::watch_set::WatchSet::from_config(&workspace, &config, &cwd) {
        Ok(ws) => {
            println!("watch recursive dirs:");
            for d in &ws.recursive_dirs {
                println!("  {}", d.display());
            }
            println!("watch files:");
            for f in &ws.files {
                println!("  {}", f.display());
            }
            println!("ignored names: {:?}", ws.ignored_dir_names);
        }
        Err(e) => println!("watch set: cannot resolve backend member -> {e:#}"),
    }
    Ok(())
}
