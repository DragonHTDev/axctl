//! Environment probing for the axctl toolchain.
//!
//! Provides a [`ProbeReport`] that collects versions of the toolchain
//! (Rust, Node, pnpm, package managers) and the current platform, used
//! by the `axctl info` subcommand and reusable by other tools.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 单个工具（命令）的探测结果。
#[derive(Debug, Clone, Default)]
pub struct ToolInfo {
    /// 命令是否可用（能找到且能执行）。
    pub available: bool,
    /// 版本信息（`--version` 输出的第一行，去尾部空白）。
    pub version: Option<String>,
}

/// 一次环境探测的完整报告。
#[derive(Debug, Clone, Default)]
pub struct ProbeReport {
    /// axctl 自身版本。
    pub axctl_version: String,
    /// 当前工作目录。
    pub cwd: PathBuf,
    /// rustc 工具链信息。
    pub rustc: ToolInfo,
    /// cargo 信息。
    pub cargo: ToolInfo,
    /// Node.js 信息。
    pub node: ToolInfo,
    /// pnpm 信息。
    pub pnpm: ToolInfo,
    /// npm 信息。
    pub npm: ToolInfo,
    /// yarn 信息。
    pub yarn: ToolInfo,
    /// 操作系统类型（如 windows / linux / macos）。
    pub os: &'static str,
    /// CPU 架构（如 x86_64 / aarch64）。
    pub arch: &'static str,
}

/// 运行 `command --version` 并解析结果。
///
/// 返回 `None` 表示命令不存在或执行失败；否则返回首行输出去尾空白。
fn version_of(command: &str) -> Option<String> {
    // Windows 上 pnpm/npm/yarn 等既有 .cmd shim 也有无扩展名的 shim 脚本，
    // 直接 Command::new 会解析到无扩展名的脚本而失败；这里统一走 cmd 执行。
    let output = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", &format!("{command} --version")])
            .output()
            .ok()?
    } else {
        Command::new(command).arg("--version").output().ok()?
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        // 取首行（部分工具会输出多行，如 rustc 的 verbose 信息）
        Some(text.lines().next().unwrap_or("").to_string())
    }
}

/// 执行一次完整的环境探测。
///
/// 所有命令探测都是尽力而为：某个工具缺失不会导致整体失败，
/// 仅对应字段标记为 `available = false`。
pub fn probe(cwd: Option<&Path>) -> ProbeReport {
    ProbeReport {
        axctl_version: env!("CARGO_PKG_VERSION").to_string(),
        cwd: cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
        rustc: probe_tool("rustc"),
        cargo: probe_tool("cargo"),
        node: probe_tool("node"),
        pnpm: probe_tool("pnpm"),
        npm: probe_tool("npm"),
        yarn: probe_tool("yarn"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// 探测单个命令的可用性与版本。
fn probe_tool(name: &str) -> ToolInfo {
    let version = version_of(name);
    ToolInfo { available: version.is_some(), version }
}

/// 判断一个路径是否像 axctl 项目根（存在 `Cargo.toml`）。
pub fn is_project_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_root_detection() {
        // 临时目录里没有 Cargo.toml → 不是项目根
        let dir = std::env::temp_dir();
        assert!(!is_project_root(&dir));

        // 造一个假的 Cargo.toml → 是项目根
        let fake = dir.join("axctl-probe-test");
        std::fs::create_dir_all(&fake).expect("创建临时目录");
        std::fs::write(fake.join("Cargo.toml"), "[package]\n").expect("写入 Cargo.toml");
        assert!(is_project_root(&fake));

        std::fs::remove_dir_all(&fake).ok();
    }

    #[test]
    fn probe_never_panics() {
        // 探测函数应该永远不 panic，无论环境如何
        let report = probe(None);
        assert!(!report.axctl_version.is_empty());
        assert!(!report.os.is_empty());
        assert!(!report.arch.is_empty());
    }
}
