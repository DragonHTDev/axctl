//! `axctl build` 命令：生产构建。
//!
//! 完整链路：
//! 1. 定位前端根（frontend_root 配置 > workspace 根；`--dir` 覆盖）
//! 2. `vite build` → dist/
//! 3. 写 `dist/.axctl-sentinel`（哨兵：dist 指纹，含 mtime 故每次构建必变）
//!    ——让用户 build.rs 的 rerun-if-changed 命中，触发 server 重编内嵌
//! 4. `cargo build --release -p <backend> --bin <bin>`
//!
//! 第 1–3 步抽成 [`resolve_frontend_root`] + [`build_frontend_with_sentinel`]
//! 供 `axctl package` 复用（同一套"前端新鲜 → 后端重编内嵌"保障）。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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
    let frontend_root = resolve_frontend_root(&workspace, &axctl_config, args.dir.as_deref())?;

    // ── 2–3. 前端构建 + 哨兵 ──
    build_frontend_with_sentinel(&frontend_root).await?;

    // ── 4. 后端 release 构建 ──
    let backend_target = crate::backend::resolve_backend(&workspace, &axctl_config, &cwd)?;
    if let Some(hint) =
        crate::backend::backend_choice_hint(&workspace, &axctl_config, &backend_target)
    {
        logging::warn(hint);
    }
    logging::info(format!(
        "building backend release: {} (bin {})",
        backend_target.package, backend_target.bin
    ));
    crate::backend::cargo_build_release(&workspace, &backend_target).await?;
    let release_bin = crate::backend::release_binary_path(&workspace, &backend_target);
    if release_bin.is_file() {
        logging::success(format!("backend release build complete: {}", release_bin.display()));
    } else {
        // 极端情况：cargo 成功但产物缺失（target 被外部清理等）
        logging::warn(format!(
            "cargo build succeeded but binary not found at {}",
            release_bin.display()
        ));
    }

    Ok(())
}

/// 解析前端项目根：`--dir` 覆盖 > `frontend_root` 配置 > workspace 根。
///
/// 返回前校验目录含 `package.json`，否则给出引导（供 build/package 共用）。
pub(crate) fn resolve_frontend_root(
    workspace: &WorkspaceInfo,
    axctl_config: &config::AxctlConfig,
    dir_override: Option<&Path>,
) -> Result<PathBuf> {
    let frontend_root: PathBuf = if let Some(dir) = dir_override {
        dir.to_path_buf()
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
    Ok(frontend_root)
}

/// 构建前端（vite build）并写哨兵文件。
///
/// 供 build/package 共用；`axctl package` 内部也先走这里，保证打进
/// 安装包的 release 二进制内嵌的是最新 dist。
pub(crate) async fn build_frontend_with_sentinel(frontend_root: &Path) -> Result<()> {
    logging::info(format!("building frontend in {}", frontend_root.display()));
    vite::run_vite_build(frontend_root)
        .await
        .context("frontend build failed")?;
    logging::success("frontend build complete");

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
    Ok(())
}

/// 递归计算目录内容指纹（文件名 + 大小 + mtime 的 hash）。
///
/// 含 mtime（纳秒精度）：产物重写即 mtime 变化 → 指纹变化 → 哨兵内容变
/// → 触发 build.rs 的 rerun-if-changed。产物未变但 mtime 未动时指纹稳定
/// （确定性），无需外部时间戳。
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 建一个临时目录 + 若干文件，返回 (dir, 文件路径列表)。
    fn make_tree() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let files = ["a.txt", "sub/b.js", "sub/deep/c.css"];
        for rel in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, format!("content of {rel}")).unwrap();
        }
        let paths = files.iter().map(|f| dir.path().join(f)).collect();
        (dir, paths)
    }

    /// 同一目录算两次 → 指纹相同（确定性）。
    #[test]
    fn fingerprint_is_stable_for_identical_dir() {
        let (dir, _) = make_tree();
        let a = dir_fingerprint(dir.path());
        let b = dir_fingerprint(dir.path());
        assert_eq!(a, b);
    }

    /// 目录里文件内容变化 → 指纹变化。
    #[test]
    fn fingerprint_differs_when_content_changes() {
        let (dir, files) = make_tree();
        let before = dir_fingerprint(dir.path());
        fs::write(&files[0], "changed content").unwrap();
        let after = dir_fingerprint(dir.path());
        assert_ne!(before, after);
    }

    /// 新增文件（不改现有内容）→ 指纹也变。
    #[test]
    fn fingerprint_differs_when_file_added() {
        let (dir, _) = make_tree();
        let before = dir_fingerprint(dir.path());
        fs::write(dir.path().join("new-file.txt"), "extra").unwrap();
        let after = dir_fingerprint(dir.path());
        assert_ne!(before, after);
    }

    /// 空目录 → 不 panic，返回确定值。
    #[test]
    fn fingerprint_empty_dir_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir_fingerprint(dir.path());
        let b = dir_fingerprint(dir.path());
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
