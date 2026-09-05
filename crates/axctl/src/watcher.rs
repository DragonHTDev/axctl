//! 基于 notify 的源码目录监听。
//!
//! 使用 `notify-debouncer-full` 去抖（默认 300ms 合并一批事件）。
//! 与旧版的关键差异：
//!
//! - 不再监听整个项目根——只监听 `WatchSet` 给出的精确目录与文件
//! - 事件通过 `std::sync::mpsc` 投递，调用方在主线程串行处理，
//!   避免多个重启任务并发堆积（notify 回调运行在自己的线程，
//!   也不再直接 `tokio::spawn`——避免 panic on `there is no reactor running`）

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{DebounceEventResult, Debouncer, FileIdMap, new_debouncer};

use crate::watch_set::WatchSet;

/// 一批变化事件（去抖后）。
#[derive(Debug, Clone)]
pub struct ChangeBatch {
    pub paths: Vec<PathBuf>,
}

/// 受管的 workspace watcher。
///
/// drop 时自动停止 notify 后台线程。
pub struct WorkspaceWatcher {
    _debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
}

/// 启动监听，回调在调用 `start` 的线程（建议主线程）被同步调用。
///
/// `on_change` 接收一批变化路径，由 caller 决定如何处理（spawn task / 重启等）。
pub fn start<F>(watch_set: WatchSet, on_change: F) -> Result<WorkspaceWatcher>
where
    F: Fn(ChangeBatch) + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<ChangeBatch>();
    let ignored_names: HashSet<String> =
        watch_set.ignored_dir_names.iter().cloned().collect();
    let auto_ignored: Vec<PathBuf> = watch_set.auto_ignored.clone();

    let mut debouncer: Debouncer<RecommendedWatcher, FileIdMap> = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                let paths: Vec<PathBuf> = events
                    .iter()
                    .flat_map(|e| e.paths.iter().cloned())
                    .filter(|p| !is_ignored(p, &ignored_names, &auto_ignored))
                    .collect();
                if !paths.is_empty() {
                    // 通道关闭（watcher drop）时 send 会失败，忽略即可
                    let _ = tx.send(ChangeBatch { paths });
                }
            }
        },
    )
    .context("failed to create file watcher")?;

    // 递归目录：member src/（rust_member）+ extra_watch_dirs（trigger_all）
    for dir in watch_set
        .recursive_dirs
        .iter()
        .chain(watch_set.trigger_all_dirs.iter())
    {
        debouncer
            .watcher()
            .watch(dir, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch directory {}", dir.display()))?;
        tracing::debug!(target: "axctl.watcher", dir = %dir.display(), "watching directory");
    }

    // 单文件：监听父目录的 NonRecursive，再在事件过滤里精确匹配
    let mut nonrecursive_parents: HashSet<PathBuf> = HashSet::new();
    for file in &watch_set.files {
        if let Some(parent) = file.parent() {
            if nonrecursive_parents.insert(parent.to_path_buf()) {
                debouncer
                    .watcher()
                    .watch(parent, RecursiveMode::NonRecursive)
                    .with_context(|| {
                        format!("failed to watch parent of {}", file.display())
                    })?;
            }
        }
    }
    for f in &watch_set.files {
        tracing::debug!(target: "axctl.watcher", file = %f.display(), "watching file");
    }

    // 事件投递线程：notify 线程 → mpsc → on_change（同步）
    // on_change 内部可以 tokio::spawn 自己处理异步逻辑
    std::thread::Builder::new()
        .name("axctl-watcher".into())
        .spawn(move || {
            while let Ok(batch) = rx.recv() {
                on_change(batch);
            }
        })
        .context("failed to spawn watcher thread")?;

    Ok(WorkspaceWatcher { _debouncer: debouncer })
}

/// 判断路径是否命中忽略规则。
fn is_ignored(path: &Path, names: &HashSet<String>, auto: &[PathBuf]) -> bool {
    if auto.iter().any(|p| path.starts_with(p)) {
        return true;
    }
    path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        names.contains(name.as_ref())
    })
}
