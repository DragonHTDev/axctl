//! `axctl build` 命令：生产构建（前端部分）。
//!
//! 当前实现阶段：只做**前端 vite build**——产出 `dist/`。
//! 后端 release 构建与哨兵写入（§6.1/§6.3）为后续步骤，届时本命令会把
//! `vite build → 写 .axctl-sentinel → cargo build --release` 串成一条链。
//!
//! 前端根定位（与 dev 一致）：`frontend_root` 配置优先（相对 workspace
//! 根），否则 workspace 根本身；`--dir` 显式覆盖。

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::logging;
use crate::vite;

/// `axctl build` 命令参数。
#[derive(Debug, Clone, clap::Args)]
pub struct BuildArgs {
    /// 前端项目根目录（含 vite.config.ts / package.json）。
    /// 默认读 frontend_root 配置，否则 workspace 根。
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

/// `axctl build` 主流程（前端部分）。
pub async fn run(args: BuildArgs) -> Result<()> {
    // workspace 根：cargo metadata 定位（build 是 workspace 级操作）
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let workspace_root = workspace_root_of(&cwd)?;

    // 前端根：--dir > frontend_root 配置 > workspace 根
    let frontend_root: PathBuf = if let Some(dir) = args.dir {
        dir
    } else {
        let config = crate::config::load_from_project(&cwd)?;
        config
            .frontend_root
            .as_ref()
            .map(|r| workspace_root.join(r))
            .unwrap_or(workspace_root.clone())
    };

    // 校验是前端项目
    if !frontend_root.join("package.json").is_file() {
        anyhow::bail!(
            "{} has no package.json — build the frontend from its project root, \
             or pass --dir <frontend-root> / configure frontend_root",
            frontend_root.display()
        );
    }

    logging::info(format!("building frontend in {}", frontend_root.display()));
    vite::run_vite_build(&frontend_root)
        .await
        .context("frontend build failed")?;

    logging::success("frontend build complete");
    Ok(())
}

/// 用 cargo metadata 找 workspace 根。
fn workspace_root_of(start: &std::path::Path) -> Result<PathBuf> {
    let meta = cargo_metadata::MetadataCommand::new()
        .current_dir(start)
        .no_deps()
        .exec()
        .context("failed to run `cargo metadata`")?;
    Ok(meta.workspace_root.as_std_path().to_path_buf())
}
