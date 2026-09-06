//! `axctl build` 命令：生产构建。
//!
//! 完整链路：
//! 1. 定位前端根（frontend_root 配置 > workspace 根；`--dir` 覆盖）
//! 2. `vite build` → dist/
//! 3. 写 `dist/.axctl-sentinel`（哨兵：dist 指纹，含 mtime 故每次构建必变）
//!    ——让用户 build.rs 的 rerun-if-changed 命中，触发 server 重编内嵌
//! 4. `cargo build --release -p <backend> --bin <bin>`

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::backend;
use crate::config;
use crate::logging;
use crate::vite;
use crate::workspace::WorkspaceInfo;

/// `axctl build` 命令参数。
#[derive(Debug, Clone, clap::Args)]
pub struct BuildArgs {
    /// 前端项目根目录（含 vite.config.ts / package.json）。
    /// 默认读 frontend_root 配置，否则 workspace 根。
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

/// `axctl build` 主流程。
pub async fn run(args: BuildArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    // ── 0. workspace / 配置 ──
    let workspace = WorkspaceInfo::load(&cwd)?;
    let axctl_config = config::load_from_project(&cwd)?;

    // ── 1. 前端根 ──
    let frontend_root: PathBuf = if let Some(dir) = args.dir {
        dir
    } else {
        axctl_config
            .frontend_root
            .as_ref()
            .map(|r| workspace.root.join(r))
            .unwrap_or_else(|| workspace.root.clone())
    };
    if !frontend_root.join("package.json").is_file() {
        anyhow::bail!(
            "{} has no package.json — run `axctl build` from the frontend root, \
             or pass --dir <frontend-root> / configure frontend_root",
            frontend_root.display()
        );
    }

    // ── 2. 前端构建 ──
    logging::info(format!("building frontend in {}", frontend_root.display()));
    vite::run_vite_build(&frontend_root)
        .await
        .context("frontend build failed")?;
    logging::success("frontend build complete");

    // ── 3. 写哨兵 ──
    // dist 目录：vite 默认 build.outDir = <frontend_root>/dist。
    let dist_dir = frontend_root.join("dist");
    if !dist_dir.is_dir() {
        anyhow::bail!("dist not found at {} — did vite build succeed?", dist_dir.display());
    }
    let sentinel = dist_dir.join(".axctl-sentinel");
    let fingerprint = dir_fingerprint(&dist_dir);
    std::fs::write(&sentinel, format!("{fingerprint}\n"))
        .with_context(|| format!("failed to write sentinel {}", sentinel.display()))?;
    logging::success(format!("sentinel written (dist fingerprint {fingerprint})"));

    // ── 4. 后端 release 构建 ──
    let backend_target = backend::resolve_backend(&workspace, &axctl_config, &cwd)?;
    logging::info(format!(
        "building backend release: {} (bin {})",
        backend_target.package, backend_target.bin
    ));
    backend::cargo_build_release(&workspace, &backend_target).await?;
    let release_bin = backend::release_binary_path(&workspace, &backend_target);
    logging::success(format!(
        "backend release build complete: {}",
        release_bin.display()
    ));

    Ok(())
}

/// 递归计算目录内容指纹（文件名 + 大小 + 修改时间，非内容 hash——
/// 快且足以区分"产物变了没"；哨兵只需稳定区分变化）。
///
/// 哨兵内容 = 此指纹（内容可能相同）+ 调用方追加时间戳（保证每次必变）。
fn dir_fingerprint(dir: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    let mut hasher = DefaultHasher::new();
    let mut files: Vec<_> = walk_files(dir);
    files.sort();
    for f in files {
        hasher.write(f.to_string_lossy().as_bytes());
        if let Ok(meta) = std::fs::metadata(&f) {
            hasher.write_u64(meta.len());
            if let Ok(mtime) = meta.modified() {
                if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    hasher.write_u64(dur.as_nanos() as u64);
                }
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

/// 递归收集目录下所有文件路径。
fn walk_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}
