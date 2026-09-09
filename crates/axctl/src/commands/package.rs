//! `axctl package` 命令：打包安装包（Server 线，封装 cargo-packager）。
//!
//! 完整链路：
//! 1. 复用 `axctl build` 的前端构建管线（vite build + 哨兵）——保证打进
//!    安装包的 release 后端内嵌的是最新 dist
//! 2. `cargo build --release` 编译后端
//! 3. 在 backend member 目录跑 `cargo packager --release`（读该 crate
//!    Cargo.toml 的 `[package.metadata.packager]`，或 `Packager.toml`）
//! 4. 产物报告：列出 target 下新生成的安装包（bundle 目录）
//!
//! 配置约定与 SeaLantern 现状对齐：packager 配置仍写在 backend crate 的
//! `[package.metadata.packager]`，由 cargo-packager 原生读取——axctl 不做
//! 第二份配置，只负责编排与产物确认。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::backend;
use crate::commands::build;
use crate::config;
use crate::logging;
use crate::workspace::{MemberInfo, WorkspaceInfo};

/// `axctl package` 命令参数。
#[derive(Debug, Clone, clap::Args)]
pub struct PackageArgs {
    /// 前端项目根目录（含 vite.config.ts / package.json）。
    /// 默认读 frontend_root 配置，否则 workspace 根（同 build）。
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// 覆盖安装包格式（如 nsis / wix / dmg / deb / appimage）。
    /// 可传多次；不传时由 cargo-packager 按配置与平台默认决定。
    #[arg(long = "format", alias = "formats", value_name = "FORMAT")]
    pub formats: Vec<String>,

    /// 指定要打包的 backend package（workspace member 名）。
    /// 默认按 build 的规则解析（backend_package / backend_command / cwd）。
    #[arg(long, value_name = "PACKAGE")]
    pub package: Option<String>,

    /// 安装包输出目录（透传给 cargo-packager --out-dir）。
    /// 默认由 cargo-packager 决定（backend member 的 target 下 bundle）。
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
}

/// `axctl package` 主流程。
pub async fn run(args: PackageArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    // ── 0. workspace / 配置 ──
    let workspace = WorkspaceInfo::load(&cwd)?;
    let mut axctl_config = config::load_from_project(&cwd)?;
    if let Some(name) = &args.package {
        axctl_config.backend_package = Some(name.clone());
    }

    // ── 1. 定位 backend + 预检 packager 配置 ──
    let backend_target = backend::resolve_backend(&workspace, &axctl_config, &cwd)?;
    let member = workspace
        .member_by_name(&backend_target.package)
        .context("resolved backend member not found")?;
    if let Some(hint) = backend::backend_choice_hint(&workspace, &axctl_config, &backend_target) {
        logging::warn(hint);
    }
    ensure_packager_config(member)?;
    ensure_packager_installed().await?;

    // ── 2. 前端构建 + 哨兵（复用 build 管线）──
    let frontend_root =
        build::resolve_frontend_root(&workspace, &axctl_config, args.dir.as_deref())?;
    build::build_frontend_with_sentinel(&frontend_root).await?;

    // ── 3. 后端 release 构建（哨兵触发重编内嵌）──
    logging::info(format!(
        "building backend release: {} (bin {})",
        backend_target.package, backend_target.bin
    ));
    backend::cargo_build_release(&workspace, &backend_target).await?;
    logging::success("backend release build complete");

    // ── 4. 跑 cargo packager（cwd = backend member，读其 packager 配置）──
    run_cargo_packager(&args, member).await?;

    // ── 5. 产物报告 ──
    report_artifacts(&workspace, &backend_target);
    Ok(())
}

/// 预检 backend member 是否具备 cargo-packager 配置。
///
/// 合法来源（与 cargo-packager 自身查找一致）：
/// - `[package.metadata.packager]`（Cargo.toml）
/// - `Packager.toml` / `packager.json`（backend 目录）
fn ensure_packager_config(member: &MemberInfo) -> Result<()> {
    let cargo_toml = &member.manifest_path;
    // 用 TOML 结构解析而非字符串 contains：避免 Cargo.toml 注释里出现
    // `[package.metadata.packager]` 字样时误判"有配置"（预检通过后真实
    // 执行才失败，削弱提前报错的引导价值）。
    let has_metadata_section = if cargo_toml.is_file() {
        let text = std::fs::read_to_string(cargo_toml)
            .with_context(|| format!("failed to read {}", cargo_toml.display()))?;
        let value: toml::Value = toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", cargo_toml.display()))?;
        value
            .get("package")
            .and_then(|p| p.get("metadata"))
            .and_then(|m| m.get("packager"))
            .is_some()
    } else {
        false
    };
    let root_files = ["Packager.toml", "packager.json", "packager.toml"];
    let has_packager_file = root_files.iter().any(|f| member.root.join(f).is_file());

    if has_metadata_section || has_packager_file {
        Ok(())
    } else {
        anyhow::bail!(
            "{} has no cargo-packager configuration. Add either\n  \
             [package.metadata.packager]\n  \
             product-name = \"My App\"\n  \
             identifier = \"com.example.app\"\n  \
             before-packaging-command = \"cargo build --release -p {}\"\nto its Cargo.toml, \
             or a Packager.toml next to it.",
            member.name,
            member.name
        )
    }
}

/// 确保 cargo-packager 子命令可用；缺时给出安装引导（不自动装，
/// `cargo install` 首次要编译数分钟，不适合在命令里静默挂起）。
async fn ensure_packager_installed() -> Result<()> {
    let check = tokio::process::Command::new("cargo")
        .args(["packager", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match check {
        Ok(status) if status.success() => Ok(()),
        _ => anyhow::bail!(
            "cargo-packager is not installed. Run `cargo install cargo-packager --locked` first."
        ),
    }
}

/// 在 backend member 目录执行 `cargo packager --release`。
///
/// - cwd 必须是含 packager 配置的 crate 目录（cargo-packager 按此解析）。
/// - 不传 `-f` 时由 cargo-packager 用配置的 formats / 平台默认，
///   避免 axctl 硬编码平台格式（对齐 config，而非 mjs 时代脚本写死）。
/// - `--out-dir` 仅当用户在 CLI 显式给才传。
async fn run_cargo_packager(args: &PackageArgs, member: &MemberInfo) -> Result<()> {
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("packager")
        .arg("--release")
        .current_dir(&member.root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for format in &args.formats {
        cmd.arg("--formats").arg(format);
    }
    if let Some(out_dir) = &args.out_dir {
        cmd.arg("--out-dir").arg(out_dir);
    }

    logging::info(format!(
        "running cargo packager in {}",
        member.root.display()
    ));
    let status = cmd
        .status()
        .await
        .context("failed to run cargo packager")?;
    if status.success() {
        logging::success("cargo packager finished");
        Ok(())
    } else {
        anyhow::bail!("cargo packager failed (exit {:?})", status.code())
    }
}

/// 列出 backend workspace target 下的安装包产物。
///
/// 探测两个位置（cargo-packager 0.11 在不同场景/平台的行为有差异）：
/// 1. `<target>/release/` profile 根的直接产物（实测 Windows 的 NSIS 安装包
///    `target/release/<name>_<ver>_x64-setup.exe` 就落在这里）——但要排除
///    后端裸二进制本身（`release_binary_path`，同名 exe，非安装包）；
/// 2. `<target>/**/bundle/` 递归（macOS/Linux 的 dmg/deb 及跨平台约定）。
///
/// 只扫这两处、并排除裸二进制，避免把 target 下其它中间产物误报成安装包。
fn report_artifacts(workspace: &WorkspaceInfo, target: &crate::backend::BackendTarget) {
    let target_dir = &workspace.target_dir;
    let mut found: Vec<PathBuf> = Vec::new();

    // 1) profile 根直接产物，排除后端裸二进制
    let profile_dir = target_dir.join("release");
    let backend_exe = crate::backend::release_binary_path(workspace, target);
    if let Ok(entries) = std::fs::read_dir(&profile_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path == backend_exe {
                continue;
            }
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if INSTALLER_EXTENSIONS.contains(&ext) {
                    found.push(path);
                }
            }
        }
    }

    // 2) bundle 目录递归（跨平台约定）
    for bundle in find_bundle_dirs(target_dir) {
        collect_installer_files(&bundle, &mut found);
    }

    found.sort();
    found.dedup();

    if found.is_empty() {
        logging::warn(format!(
            "no installer artifacts found under {} — check the packager output above",
            target_dir.display()
        ));
        return;
    }

    for path in &found {
        let size = std::fs::metadata(path)
            .map(|m| human_size(m.len()))
            .unwrap_or_else(|_| "?".into());
        logging::success(format!("artifact: {} ({size})", path.display()));
    }
}

/// 常见安装包扩展名（cargo-packager 支持格式的产物）。
const INSTALLER_EXTENSIONS: &[&str] = &[
    "msi", "dmg", "deb", "rpm", "AppImage", "appimage", "exe", "app", "pkg", "tar", "gz",
];

/// 递归找出 target 下所有名为 `bundle` 的目录（cargo-packager 产物根）。
fn find_bundle_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("bundle") {
            out.push(path);
        } else {
            out.extend(find_bundle_dirs(&path));
        }
    }
    out
}

/// 在给定目录（bundle 根）下递归收集扩展名命中的安装包文件。
fn collect_installer_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_installer_files(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if INSTALLER_EXTENSIONS.contains(&ext) {
                out.push(path);
            }
        }
    }
}

/// 字节数 → 人类可读大小。
fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个 MemberInfo（测试用，避免依赖真实 workspace）。
    fn member(root: &Path, name: &str) -> MemberInfo {
        MemberInfo {
            package_id: cargo_metadata::PackageId { repr: format!("{name} v0.1.0") },
            name: name.to_string(),
            root: root.to_path_buf(),
            manifest_path: root.join("Cargo.toml"),
            targets: vec![],
            path_dep_ids: vec![],
        }
    }

    /// 写了 [package.metadata.packager] 的 Cargo.toml → 预检通过。
    #[test]
    fn packager_config_accepts_metadata_section() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n\n[package.metadata.packager]\nproduct-name = \"Demo\"\n",
        )
        .unwrap();
        let m = member(dir.path(), "demo");
        assert!(ensure_packager_config(&m).is_ok());
    }

    /// 没有 packager 配置 → 预检报错并引导。
    #[test]
    fn packager_config_rejects_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        let m = member(dir.path(), "demo");
        let err = ensure_packager_config(&m).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("package.metadata.packager"), "应引导配置: {msg}");
    }

    /// 注释里出现 `[package.metadata.packager]` 字样但无真实 section →
    /// 不应误判"有配置"（TOML 结构解析，而非字符串 contains）。
    #[test]
    fn packager_config_ignores_comment_mention() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n# 示例：[package.metadata.packager]\n",
        )
        .unwrap();
        let m = member(dir.path(), "demo");
        assert!(
            ensure_packager_config(&m).is_err(),
            "注释里的同名段不应被当成真实 packager 配置"
        );
    }

    /// Packager.toml 单独存在 → 预检通过。
    #[test]
    fn packager_config_accepts_standalone_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Packager.toml"),
            "product-name = \"Demo\"\n",
        )
        .unwrap();
        let m = member(dir.path(), "demo");
        assert!(ensure_packager_config(&m).is_ok());
    }

    /// 扫描器能找到 bundle 里的安装包并忽略无关文件。
    #[test]
    fn artifact_scan_finds_installers_only() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("target").join("release").join("bundle");
        std::fs::create_dir_all(bundle.join("nsis")).unwrap();
        std::fs::write(bundle.join("nsis").join("demo_0.1.0_x64-setup.exe"), "x").unwrap();
        // 无关文件：bundle 外的裸二进制 / 源码 / 非安装包扩展名，都不该被收集
        std::fs::write(dir.path().join("target").join("release").join("demo.exe"), "x").unwrap();
        std::fs::write(bundle.join("nsis").join("readme.txt"), "x").unwrap();

        let mut found = Vec::new();
        for bundle_dir in find_bundle_dirs(&dir.path().join("target")) {
            collect_installer_files(&bundle_dir, &mut found);
        }
        let names: Vec<&str> = found
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert_eq!(names, vec!["demo_0.1.0_x64-setup.exe"]);
    }

    /// bundle 目录之外的同名 tar.gz 源码包不会被误报成安装包。
    #[test]
    fn artifact_scan_excludes_non_bundle_tarballs() {
        let dir = tempfile::tempdir().unwrap();
        // bundle 内是真实安装包
        let bundle = dir.path().join("target").join("release").join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("demo.deb"), "x").unwrap();
        // target 别处（非 bundle）的源码包 / 裸可执行，都不该被收集
        std::fs::create_dir_all(dir.path().join("target").join("src-pkgs")).unwrap();
        std::fs::write(
            dir.path().join("target").join("src-pkgs").join("demo-1.0.0.tar.gz"),
            "x",
        )
        .unwrap();
        std::fs::write(dir.path().join("target").join("release").join("demo.exe"), "x").unwrap();

        let mut found = Vec::new();
        for bundle_dir in find_bundle_dirs(&dir.path().join("target")) {
            collect_installer_files(&bundle_dir, &mut found);
        }
        let names: Vec<&str> = found
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert_eq!(names, vec!["demo.deb"]);
    }

    /// 没有产物 → 扫描为空（report 会打 warn，这里只验扫描空）。
    #[test]
    fn artifact_scan_empty_on_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut found = Vec::new();
        for bundle_dir in find_bundle_dirs(&dir.path().join("target")) {
            collect_installer_files(&bundle_dir, &mut found);
        }
        assert!(found.is_empty());
    }

    /// 人类可读大小换算。
    #[test]
    fn human_size_formatting() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
