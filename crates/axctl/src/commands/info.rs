//! `axctl info` 命令：环境诊断。
//!
//! 输出 axctl 版本、Rust / Node / 包管理器版本、系统平台与当前目录，
//! 帮助用户排查环境问题。

use axctl_core::probe;

/// 环境诊断的入口。
pub fn run() -> anyhow::Result<()> {
    let report = probe::probe(None);

    println!("axctl: v{}", report.axctl_version);
    println!("platform: {} ({})", report.os, report.arch);
    println!("cwd: {}", report.cwd.display());
    println!();
    println_tool("rustc", &report.rustc);
    println_tool("cargo", &report.cargo);
    println_tool("node", &report.node);
    println_tool("pnpm", &report.pnpm);
    println_tool("npm", &report.npm);
    println_tool("yarn", &report.yarn);

    // 顺便判断当前目录是否是 axctl 项目根，给出提示
    if probe::is_project_root(&report.cwd) {
        println!();
        println!("project root: yes (Cargo.toml found)");
    } else {
        println!();
        println!("project root: no (no Cargo.toml in cwd)");
    }

    Ok(())
}

/// 输出单个工具的行；不可用时给出明确提示。
fn println_tool(name: &str, tool: &probe::ToolInfo) {
    match &tool.version {
        Some(version) => println!("{name}: {version}"),
        None => println!("{name}: (not found)"),
    }
}
