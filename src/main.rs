//! axctl — an all-in-one development tool for axum projects.
//!
//! Inspired by the tauri-cli developer experience, this CLI provides
//! `dev` / `build` / `serve` / `package` / `info` subcommands for
//! axum + Vite projects.
//!
//! 本 crate 面向公开文档（docs.rs），模块注释保持英文；
//! 源码内部的过程性注释使用中文，见各函数实现。

use clap::Parser;

/// axctl command-line entry point.
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

/// The set of subcommands.
#[derive(clap::Subcommand)]
enum Command {
    /// Initialize a project (detect environment, generate config)
    Init,
    /// Development mode: watch source changes, rebuild and restart the server automatically
    Dev,
    /// Production build: build frontend assets + cargo release build
    Build,
    /// Static preview: serve built assets directly from disk or embedded
    Serve,
    /// Package: generate installers via cargo-packager
    Package,
    /// Environment diagnostics: print Rust / frontend / system info
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
