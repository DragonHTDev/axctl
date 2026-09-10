//! 后端（backend）的解析、编译与产物路径推导。
//!
//! # 启动策略：先 build 后 spawn 产物（对齐 tauri-cli）
//!
//! 不直接 `cargo run`，而是：
//! 1. 解析出 backend 对应的 workspace member 与 binary target；
//! 2. `cargo build -p <pkg> --bin <bin>` 编译（阻塞到成功）；
//! 3. spawn `target/debug/<bin>(.exe)` 编译产物。
//!
//! 好处：
//! - 产物是**叶子进程**，终止时 kill 根进程即干净，无"cargo 父进程 +
//!   孤儿 server"残留；
//! - 编译期间旧后端仍在运行（build 成功才替换进程），API 不中断；
//! - build 失败时旧后端继续服务，错误信息清晰。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::AxctlConfig;
use crate::workspace::{MemberInfo, WorkspaceInfo};

/// 解析出的后端目标：哪个 member 的哪个 binary。
#[derive(Debug, Clone)]
pub struct BackendTarget {
    /// workspace member 名。
    pub package: String,
    /// binary target 名。
    pub bin: String,
}

/// 选择 backend target：member 解析（4 级）+ binary 确定。
///
/// member 解析优先级（与 watcher 的监听范围一致）：
/// 1. `backend_package` 配置
/// 2. `backend_command` 的 `-p` / `--bin`
/// 3. cwd 所在 member
/// 4. workspace 内唯一 binary package
///
/// binary 确定：显式 `--bin` 优先；否则 member 里同名 bin；否则唯一 bin。
pub fn resolve_backend(
    ws: &WorkspaceInfo,
    cfg: &AxctlConfig,
    start_dir: &Path,
) -> Result<BackendTarget> {
    let member = resolve_backend_member(ws, cfg, start_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "cannot determine backend package: configure either\n  \
             [workspace.metadata.axctl.dev].backend_package = \"<name>\"\n  \
             or backend_command = \"cargo run -p <name>\""
        )
    })?;

    let bin = resolve_bin_name(member, cfg)?;
    Ok(BackendTarget { package: member.name.clone(), bin })
}

/// 多 binary workspace 且未显式指定 backend 时，返回一行引导提示。
///
/// 供 dev/build/package 在解析出 backend 后打印，让"项目根运行选中了谁"
/// 一目了然；无歧义或已显式指定时返回 `None`。
///
/// 判定"显式指定"见 [`is_backend_explicitly_configured`]；歧义统计为
/// workspace 内**有 bin target** 的成员数 >1。
pub fn backend_choice_hint(
    ws: &WorkspaceInfo,
    cfg: &AxctlConfig,
    target: &BackendTarget,
) -> Option<String> {
    if is_backend_explicitly_configured(cfg) {
        return None;
    }
    let binary_count = ws
        .members
        .iter()
        .filter(|m| {
            m.targets.iter().any(|t| {
                t.kind
                    .iter()
                    .any(|k| matches!(k, cargo_metadata::TargetKind::Bin))
            })
        })
        .count();
    if binary_count > 1 {
        Some(format!(
            "workspace has {binary_count} binary crates; selected `{}` for backend. \
             If not intended, set [workspace.metadata.axctl.dev].backend_package = \"{}\"",
            target.package, target.package
        ))
    } else {
        None
    }
}

/// backend 是否已**显式**指定：`backend_package` 配置，或 `backend_command`
/// 里带 `-p`/`--bin`（此时选中是用户明确意图，无需歧义提示）。
pub(crate) fn is_backend_explicitly_configured(cfg: &AxctlConfig) -> bool {
    if cfg.backend_package.is_some() {
        return true;
    }
    cfg.backend_command
        .as_ref()
        .map(|c| parse_package_flag(c).is_some() || parse_bin_flag(c).is_some())
        .unwrap_or(false)
}

/// 解析 backend member（4 级优先级，见模块文档）。
///
/// 供本模块 `resolve_backend` 与 watcher 的 WatchSet 计算共用，
/// 保证"监听谁"与"重启谁"始终是同一个 member。
pub(crate) fn resolve_backend_member<'a>(
    ws: &'a WorkspaceInfo,
    cfg: &AxctlConfig,
    start_dir: &Path,
) -> Option<&'a MemberInfo> {
    // 1. 显式 backend_package
    if let Some(name) = &cfg.backend_package {
        if let Some(m) = ws.member_by_name(name) {
            return Some(m);
        }
        tracing::warn!(
            target: "axctl.dev",
            name = %name,
            "configured backend_package not found in workspace members"
        );
    }

    // 2. backend_command 中提取 -p / --bin
    if let Some(cmd) = &cfg.backend_command {
        if let Some(pkg) = parse_package_flag(cmd) {
            if let Some(m) = ws.member_by_name(&pkg) {
                return Some(m);
            }
            tracing::warn!(
                target: "axctl.dev",
                package = %pkg,
                "package from backend_command -p not found in workspace"
            );
        }
        if let Some(bin) = parse_bin_flag(cmd) {
            if let Some(m) = ws.package_for_bin(&bin) {
                return Some(m);
            }
            tracing::warn!(
                target: "axctl.dev",
                bin = %bin,
                "binary from backend_command --bin not found in workspace"
            );
        }
    }

    // 3. cwd 所在 member
    if let Some(m) = ws.member_for_dir(start_dir) {
        return Some(m);
    }

    // 4. workspace 内唯一 binary package
    let bins: Vec<_> = ws
        .members
        .iter()
        .filter(|m| {
            m.targets.iter().any(|t| {
                t.kind
                    .iter()
                    .any(|k| matches!(k, cargo_metadata::TargetKind::Bin))
            })
        })
        .collect();
    if bins.len() == 1 {
        return Some(bins[0]);
    }

    None
}

/// 确定 binary target 名。
///
/// 找不到明确 bin 时报错并列出可选 bin，引导用户配置
/// `backend_command` 的 `--bin` 或 `backend_package`——而不是返回一个
/// 注定 `cargo build --bin` 失败的包名（那会把错误推迟成晦涩的 cargo 输出）。
fn resolve_bin_name(member: &MemberInfo, cfg: &AxctlConfig) -> Result<String> {
    let all_bins: Vec<String> = member
        .targets
        .iter()
        .filter(|t| {
            t.kind
                .iter()
                .any(|k| matches!(k, cargo_metadata::TargetKind::Bin))
        })
        .map(|t| t.name.clone())
        .collect();

    // 1. backend_command 的 --bin 显式指定
    if let Some(cmd) = &cfg.backend_command {
        if let Some(bin) = parse_bin_flag(cmd) {
            if all_bins.contains(&bin) {
                return Ok(bin);
            }
        }
    }

    // 2. 与 package 同名的 bin（cargo 默认约定）
    if all_bins.contains(&member.name) {
        return Ok(member.name.clone());
    }

    // 3. 唯一 bin
    if all_bins.len() == 1 {
        return Ok(all_bins[0].clone());
    }

    // 4. 无法确定：显式报错，列出可选项
    anyhow::bail!(
        "cannot determine binary target for backend package `{}`: \
         available binaries are [{}]. \
         Configure `[workspace.metadata.axctl.dev].backend_command = \
         \"cargo run -p {} --bin <name>\"` or `backend_package`.",
        member.name,
        all_bins.join(", "),
        member.name
    )
}

/// 编译 backend（阻塞到 cargo 退出）。
pub async fn cargo_build(ws: &WorkspaceInfo, target: &BackendTarget) -> Result<()> {
    cargo_build_profile(ws, target, "dev").await
}

/// 以 release profile 编译后端（`cargo build --release -p <pkg> --bin <bin>`）。
///
/// 供 `axctl build` 生产构建用；产物在 `target/release/<bin>(.exe)`。
pub async fn cargo_build_release(ws: &WorkspaceInfo, target: &BackendTarget) -> Result<()> {
    cargo_build_profile(ws, target, "release").await
}

/// 按 profile 编译后端，阻塞到 cargo 退出。
async fn cargo_build_profile(
    ws: &WorkspaceInfo,
    target: &BackendTarget,
    profile: &str,
) -> Result<()> {
    let mut cmd = tokio::process::Command::new("cargo");
    if profile == "release" {
        cmd.arg("build").arg("--release");
    } else {
        cmd.arg("build");
    }
    cmd.args(["-p", &target.package, "--bin", &target.bin])
        .current_dir(&ws.root)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .await
        .with_context(|| format!("failed to run cargo build for {}", target.package))?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo build failed for {} (exit {:?})", target.package, status.code())
    }
}

/// dev 产物路径：`<target_dir>/debug/<bin>(.exe)`。
pub fn binary_path(ws: &WorkspaceInfo, target: &BackendTarget) -> PathBuf {
    binary_path_for(ws, target, "debug")
}

/// release 产物路径：`<target_dir>/release/<bin>(.exe)`。
pub fn release_binary_path(ws: &WorkspaceInfo, target: &BackendTarget) -> PathBuf {
    binary_path_for(ws, target, "release")
}

/// 按 profile 目录拼产物路径（`debug` / `release`）。
fn binary_path_for(ws: &WorkspaceInfo, target: &BackendTarget, profile: &str) -> PathBuf {
    let mut path = ws.target_dir.join(profile).join(&target.bin);
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

/// 从 `cargo run -p name` 或 `cargo run --package=name` 解析出 "name"。
pub fn parse_package_flag(cmd: &str) -> Option<String> {
    let mut parts = cmd.split_whitespace();
    while let Some(p) = parts.next() {
        if p == "-p" || p == "--package" {
            return parts.next().map(|s| s.to_string());
        }
        if let Some(rest) = p
            .strip_prefix("-p=")
            .or_else(|| p.strip_prefix("--package="))
        {
            return Some(rest.to_string());
        }
    }
    None
}

/// 从 `cargo run --bin name` 或 `cargo run --bin=name` 解析出 "name"。
pub fn parse_bin_flag(cmd: &str) -> Option<String> {
    let mut parts = cmd.split_whitespace();
    while let Some(p) = parts.next() {
        if p == "--bin" {
            return parts.next().map(|s| s.to_string());
        }
        if let Some(rest) = p.strip_prefix("--bin=") {
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::MemberInfo;
    use cargo_metadata::PackageId;

    /// 构造一个指定 targets 的 MemberInfo（测试用，避免依赖真实 workspace）。
    ///
    /// 注意：`cargo_metadata::Target` 是 non-exhaustive 结构，无法在外部
    /// 手工构造，因此无法直接构造"含多个异名 bin"的假 member。这里用
    /// 空 targets 验证"无 bin 可确定 → 显式报错"分支；多 bin 歧义分支的
    /// 逻辑与空 targets 走同一错误路径（都到第 4 步 bail），由该测试覆盖。
    fn member_no_bins(pkg_name: &str) -> MemberInfo {
        MemberInfo {
            package_id: PackageId { repr: format!("{pkg_name} v0.1.0") },
            name: pkg_name.to_string(),
            root: PathBuf::from("/proj"),
            manifest_path: PathBuf::from("/proj/Cargo.toml"),
            targets: vec![],
            path_dep_ids: vec![],
        }
    }

    /// 无法确定 bin（无同名、无唯一、无 --bin）→ 应显式报错并给引导，
    /// 而非返回一个注定 cargo build --bin 失败的包名。
    #[test]
    fn resolve_bin_name_errors_when_indeterminate() {
        let member = member_no_bins("my-server");
        let cfg = AxctlConfig::default();
        let err = resolve_bin_name(&member, &cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("cannot determine binary target"),
            "应显式报错而非伪兜底, got: {msg}"
        );
        assert!(msg.contains("--bin"), "错误信息应引导 --bin, got: {msg}");
    }

    #[test]
    fn parse_package_flag_long() {
        assert_eq!(parse_package_flag("cargo run --package foo-bar"), Some("foo-bar".into()));
    }
    #[test]
    fn parse_package_flag_short() {
        assert_eq!(parse_package_flag("cargo run -p foo"), Some("foo".into()));
    }
    #[test]
    fn parse_package_flag_eq() {
        assert_eq!(parse_package_flag("cargo run -p=foo"), Some("foo".into()));
        assert_eq!(parse_package_flag("cargo run --package=foo"), Some("foo".into()));
    }
    #[test]
    fn parse_package_flag_none() {
        assert_eq!(parse_package_flag("cargo run"), None);
        assert_eq!(parse_package_flag(""), None);
    }
    #[test]
    fn test_parse_bin_flag() {
        assert_eq!(parse_bin_flag("cargo run --bin my-server"), Some("my-server".into()));
        assert_eq!(parse_bin_flag("cargo run --bin=my-server"), Some("my-server".into()));
        assert_eq!(parse_bin_flag("cargo run"), None);
    }

    /// 配置了 backend_package → 视为显式指定，无需歧义提示。
    #[test]
    fn explicit_via_backend_package() {
        let cfg = AxctlConfig {
            backend_package: Some("my-server".into()),
            ..Default::default()
        };
        assert!(is_backend_explicitly_configured(&cfg));
    }

    /// backend_command 带 -p / --package → 显式指定。
    #[test]
    fn explicit_via_backend_command_package() {
        let cfg = AxctlConfig {
            backend_command: Some("cargo run -p my-server".into()),
            ..Default::default()
        };
        assert!(is_backend_explicitly_configured(&cfg));
        let cfg = AxctlConfig {
            backend_command: Some("cargo run --package=my-server".into()),
            ..Default::default()
        };
        assert!(is_backend_explicitly_configured(&cfg));
    }

    /// backend_command 带 --bin → 显式指定。
    #[test]
    fn explicit_via_backend_command_bin() {
        let cfg = AxctlConfig {
            backend_command: Some("cargo run --bin my-server".into()),
            ..Default::default()
        };
        assert!(is_backend_explicitly_configured(&cfg));
    }

    /// 无 backend_package、backend_command 也不带 -p/--bin → 未显式指定。
    #[test]
    fn not_explicit_when_unconfigured() {
        let cfg = AxctlConfig {
            backend_command: Some("cargo run".into()),
            ..Default::default()
        };
        assert!(!is_backend_explicitly_configured(&cfg));
        let cfg = AxctlConfig::default();
        assert!(!is_backend_explicitly_configured(&cfg));
    }
}
