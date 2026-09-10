//! `axctl dev` 命令：开发模式。
//!
//! 流程：
//! 1. 加载 workspace 信息（cargo metadata），解析配置
//! 2. 探测/拉起 vite dev server（5173）
//! 3. 编译并启动后端（3001）
//! 4. 起反向代理（3000）：/api → 后端，其余 → vite，WS 隧道
//! 5. 监听"后端依赖闭包"内源码变化 → 重编译重启（代理与 vite 不动）

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::backend::{self, BackendTarget};
use crate::config;
use crate::logging;
use crate::process::{self, ManagedChild};
use crate::proxy::{self, ProxyTarget};
use crate::vite;
use crate::watch_set::WatchSet;
use crate::watcher;
use crate::workspace::WorkspaceInfo;

/// 默认后端 dev 监听地址（端口来自配置）。
const DEFAULT_BACKEND_HOST: &str = "127.0.0.1";

/// 判断一个路径是否属于"需要重启后端"的文件。
///
/// watcher 现在只监听精确目录（member/src 递归 + Cargo.toml 等单文件），
/// 但 member/src 下仍可能有非 .rs 文件被捕获，这里做最终过滤。
fn is_backend_source(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    name.ends_with(".rs") || name == "Cargo.toml" || name == "build.rs"
}

/// 重启状态机的三个状态（AtomicU8）。
///
/// - [`IDLE`]：无重启在跑；
/// - [`RESTARTING`]：一次重启进行中；
/// - [`PENDING`]：重启期间来了新变更，结束后需补一轮。
const IDLE: u8 = 0;
const RESTARTING: u8 = 1;
const PENDING: u8 = 2;

/// 一次后端重启**结束之后**要做的动作。
#[derive(Debug, PartialEq, Eq)]
enum RoundAction {
    /// 本轮运行期间有新变更（状态为 PENDING）→ 转回 RESTARTING 再跑一轮。
    Rerun,
    /// 无新变更（状态为 RESTARTING，或异常态）→ 收尾。
    Stop,
}

/// 根据当前重启状态决定一轮结束后的动作（纯函数，便于单测）。
fn round_action(state: u8) -> RoundAction {
    if state == PENDING {
        RoundAction::Rerun
    } else {
        RoundAction::Stop
    }
}

/// 先编译后端，成功后 spawn 编译产物。
///
/// 流程（对齐 tauri-cli）：
/// 1. `cargo build -p <pkg> --bin <bin>` 编译（阻塞到成功/失败）
/// 2. spawn `target/debug/<bin>(.exe)`
///
/// 地址经环境变量 `AXCTL_BACKEND_ADDR` 传入后端。
/// build 失败返回 Err（此时不会 spawn，调用方决定是否保留旧进程）。
async fn build_and_start_backend(
    workspace: &WorkspaceInfo,
    target: &BackendTarget,
    backend_addr: &str,
) -> Result<ManagedChild> {
    backend::cargo_build(workspace, target).await?;
    spawn_backend_binary(workspace, target, backend_addr).await
}

/// spawn 后端编译产物（假定已 build 成功）。
async fn spawn_backend_binary(
    workspace: &WorkspaceInfo,
    target: &BackendTarget,
    backend_addr: &str,
) -> Result<ManagedChild> {
    let binary = backend::binary_path(workspace, target);
    if !binary.is_file() {
        anyhow::bail!(
            "backend binary not found at {} (did cargo build succeed?)",
            binary.display()
        );
    }
    let binary_str = binary.to_string_lossy().to_string();
    tracing::debug!(target: "axctl.dev", binary = %binary_str, %backend_addr, "starting backend binary");
    ManagedChild::spawn(
        "backend",
        &binary_str,
        &[],
        Some(&workspace.root),
        &[("AXCTL_BACKEND_ADDR", backend_addr.to_string())],
    )
}

/// `axctl dev` 主流程。
pub async fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    // ── 0. workspace / 配置 ──
    // 只跑一次 `cargo metadata`（不带 no_deps——workspace.rs 需要 resolve
    // 依赖图算闭包），同时喂给 WorkspaceInfo 与配置读取
    let meta = cargo_metadata::MetadataCommand::new()
        .current_dir(&cwd)
        .exec()
        .context("failed to run `cargo metadata`")?;
    let workspace = WorkspaceInfo::from_metadata(&meta)?;
    let axctl_config = config::load_from_metadata(&meta, &cwd)?;
    let workspace_root = workspace.root.clone();
    let backend_target = backend::resolve_backend(&workspace, &axctl_config, &cwd)?;
    if let Some(hint) = backend::backend_choice_hint(&workspace, &axctl_config, &backend_target) {
        logging::warn(hint);
    }

    logging::info(format!("axctl dev in {}", workspace_root.display()));

    // ── 1. 前端 dev server ──
    //
    // 目标地址 (vite_host, vite_port)：
    // - 配置了 frontend_dev_url → 用它解析出的 host:port（tauri 心智：
    //   devUrl 指向的已运行前端由用户负责，axctl 只探测复用，不拉起）
    // - 未配置 → 默认 5173，axctl 可自行拉起
    let mut vite_host = vite::DEFAULT_DEV_HOST.to_string();
    let mut vite_port = vite::DEFAULT_DEV_PORT;
    let mut vite_child: Option<ManagedChild> = None;

    if let Some(url) = &axctl_config.frontend_dev_url {
        let (host, port) = config::parse_addr(url.trim_start_matches("http://"))?;
        vite_host = host;
        vite_port = port;
        logging::info(format!("using configured frontend dev url: http://{vite_host}:{vite_port}"));
    }

    // 前端工作目录：frontend_root 配置优先，否则 workspace 根
    let frontend_root: PathBuf = axctl_config
        .frontend_root
        .as_ref()
        .map(|r| workspace_root.join(r))
        .unwrap_or_else(|| workspace_root.clone());

    let vite_configured = axctl_config.frontend_dev_url.is_some();
    if vite::http_ready(&vite_host, vite_port, Duration::from_millis(800)).await {
        logging::info(format!("reusing running vite dev server at http://{vite_host}:{vite_port}"));
    } else if vite_configured {
        // 配置了 devUrl 但尚未就绪：等待用户自行启动的前端（tauri 同款等待）。
        // 注意：不主动拉起——devUrl 语义是"前端由外部管理"。
        logging::info(format!(
            "waiting for frontend dev server at http://{vite_host}:{vite_port}..."
        ));
        if !vite::http_ready(&vite_host, vite_port, Duration::from_secs(60)).await {
            logging::error(format!(
                "frontend dev server did not become ready on {vite_host}:{vite_port}"
            ));
            anyhow::bail!("frontend dev server at {vite_host}:{vite_port} is not reachable");
        }
    } else {
        logging::info("starting vite dev server...");
        vite_child = Some(vite::spawn_vite(&frontend_root).await?);
        if !vite::http_ready(&vite_host, vite_port, Duration::from_secs(15)).await {
            logging::error(format!(
                "vite dev server did not become ready on {vite_host}:{vite_port}"
            ));
            anyhow::bail!("vite dev server did not become ready on {vite_host}:{vite_port}");
        }
    }

    // ── 2. 后端 ──
    let backend_port = axctl_config.backend_port_or();
    let backend_addr = format!("{DEFAULT_BACKEND_HOST}:{backend_port}");
    logging::info(format!(
        "building backend: {} (bin {})",
        backend_target.package, backend_target.bin
    ));
    let backend = build_and_start_backend(&workspace, &backend_target, &backend_addr).await?;

    // ── 3. 反向代理 ──
    let proxy_addr = axctl_config.proxy_addr_or();
    let (proxy_host, proxy_port) = config::parse_addr(&proxy_addr)?;

    // 安全提示：非 loopback 监听会把无鉴权的后端与 vite dev server
    // 暴露到局域网（vite 的 @fs 与 HMR 有历史 RCE：CVE-2025-30221 等），
    // 只应在受信的本机开发环境使用。
    if !config::is_loopback_host(&proxy_host) {
        logging::warn(format!(
            "proxy_addr {proxy_addr} is not loopback: the unauthenticated backend and \
             vite dev server will be exposed to the network. For local development only."
        ));
    }

    let proxy_router = proxy::build_proxy_router(
        ProxyTarget {
            host: DEFAULT_BACKEND_HOST.to_string(),
            port: backend_port,
        },
        ProxyTarget { host: vite_host.clone(), port: vite_port },
    );
    let listener = TcpListener::bind((proxy_host.as_str(), proxy_port))
        .await
        .with_context(|| format!("failed to bind proxy on {proxy_addr}"))?;

    // 代理在后台任务运行；serve 正常只会在监听 socket 出错时返回 Err
    let mut proxy_task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, proxy_router).await {
            tracing::error!(target: "axctl.proxy", error = %error, "proxy server error");
        }
    });
    logging::success(format!("reverse proxy listening on http://{proxy_addr}"));

    // ── 4. 文件监听：后端依赖闭包内源码变化 → 重编译重启 ──
    let watch_set = WatchSet::from_config(&workspace, &axctl_config, &cwd)?;
    if watch_set.is_empty() {
        logging::warn("watch set is empty: no backend sources will trigger restart");
    } else {
        for dir in &watch_set.recursive_dirs {
            logging::info(format!("watching {} for changes...", dir.display()));
        }
        for dir in &watch_set.trigger_all_dirs {
            logging::info(format!(
                "watching {} for changes (any change triggers restart)...",
                dir.display()
            ));
        }
    }

    // 重启状态机（状态常量见模块级 IDLE / RESTARTING / PENDING）
    let restart_state = Arc::new(AtomicU8::new(IDLE));
    let backend_lock = Arc::new(Mutex::new(backend));
    let backend_target_owned = Arc::new(backend_target);

    let watcher = {
        let restart_state = restart_state.clone();
        let backend_lock = backend_lock.clone();
        let ws = workspace.clone();
        let addr = backend_addr.clone();
        let target = backend_target_owned.clone();
        let watch_set_cb = watch_set.clone();
        let runtime_handle = tokio::runtime::Handle::current();

        watcher::start(watch_set, move |batch| {
            // 事件投递线程同步调用；只过滤是否后端相关，异步重启交给 runtime。
            // extra_watch_dirs 内的变化不过滤（任何文件都触发），
            // 其余（member src/）按 Rust 源码过滤。
            let changed = batch
                .paths
                .iter()
                .any(|p| watch_set_cb.is_in_trigger_all(p) || is_backend_source(p));
            if !changed {
                return;
            }

            // 抢占：idle → restarting 才真正触发一轮重启；
            // 已在 restarting → 置 PENDING，当前轮结束后补跑（不丢变更）。
            match restart_state.compare_exchange(
                IDLE,
                RESTARTING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {}
                Err(_) => {
                    restart_state.store(PENDING, Ordering::SeqCst);
                    return;
                }
            }

            let restart_state = restart_state.clone();
            let backend_lock = backend_lock.clone();
            let ws = ws.clone();
            let addr = addr.clone();
            let target = target.clone();

            runtime_handle.spawn(async move {
                // 循环处理：一轮重启结束后若期间有新变更（PENDING），
                // 再补一轮，直到没有新变更到达。
                loop {
                    logging::info("source changed, rebuilding...");
                    let result = restart_backend(&backend_lock, &ws, &target, &addr).await;
                    match result {
                        Ok(_) => logging::success("backend restarted"),
                        Err(error) => {
                            logging::error(format!("backend restart failed: {error}"));
                            // 重启失败：释放状态，允许下次变更再试
                            restart_state.store(IDLE, Ordering::SeqCst);
                            return;
                        }
                    }
                    // 一轮结束：按状态决定"补跑"还是"收尾"（见 round_action）。
                    match round_action(restart_state.load(Ordering::SeqCst)) {
                        RoundAction::Rerun => {
                            // 有新变更：转回 RESTARTING 再跑一轮（CAS 失败说明状态
                            // 又变了，交给下一轮重新判断）。
                            let _ = restart_state.compare_exchange(
                                PENDING,
                                RESTARTING,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            );
                            continue;
                        }
                        RoundAction::Stop => {
                            // 无新变更：用 CAS 收尾（RESTARTING → IDLE）。
                            // 关键：绝不能 store(IDLE)——若此刻回调恰好置入
                            // PENDING（新变更），store 会把它覆盖掉，导致该变更
                            // 不触发补跑（用户存盘后无反应）。
                            if restart_state
                                .compare_exchange(
                                    RESTARTING,
                                    IDLE,
                                    Ordering::SeqCst,
                                    Ordering::SeqCst,
                                )
                                .is_ok()
                            {
                                return;
                            }
                            // CAS 失败：状态不是 RESTARTING（应是 PENDING，即此刻
                            // 新到的变更）→ 转回 RESTARTING 补跑一轮；若为其它
                            // 异常态则直接收尾，避免空转。
                            if restart_state.load(Ordering::SeqCst) == PENDING {
                                let _ = restart_state.compare_exchange(
                                    PENDING,
                                    RESTARTING,
                                    Ordering::SeqCst,
                                    Ordering::SeqCst,
                                );
                                continue;
                            }
                            return;
                        }
                    }
                }
            });
        })?
    };

    logging::blank();
    logging::info(format!("proxy:    http://{proxy_addr}"));
    logging::info(format!("frontend: http://{vite_host}:{vite_port} (vite)"));
    logging::info(format!("backend:  http://{backend_addr}"));
    logging::blank();
    logging::info("Press Ctrl+C to stop.");
    logging::blank();

    // ── 等待 Ctrl+C / SIGTERM / 代理异常退出 ──
    let ctrl_c = tokio::signal::ctrl_c();
    let mut terminate = std::pin::pin!(process::shutdown_signal());
    let mut proxy_task_mut = &mut proxy_task;
    tokio::select! {
        _ = ctrl_c => {},
        _ = &mut terminate => {},
        proxy_result = &mut proxy_task_mut => {
            // 代理任务结束：正常只会因 serve 出错。记录后进入清理流程。
            match proxy_result {
                Ok(()) => tracing::warn!(target: "axctl.proxy", "proxy server stopped unexpectedly"),
                Err(join_error) => tracing::warn!(target: "axctl.proxy", error = %join_error, "proxy task panicked"),
            }
        }
    }

    // ── 清理 ──
    logging::info("shutting down axctl dev...");
    drop(watcher);
    proxy_task.abort();
    let mut backend = backend_lock.lock().await;
    backend.kill_tree().await;
    if let Some(mut vite) = vite_child {
        vite.kill_tree().await;
    }
    logging::success("axctl dev stopped");
    Ok(())
}

/// 重启后端：编译成功后替换进程。
///
/// 平台差异（关键）：
/// - Unix：先编译，成功后才 kill 旧进程——编译期间旧后端继续服务，
///   API 不中断，编译失败旧进程保留。
/// - Windows：必须先 kill 旧进程再编译——Windows 锁定正在运行的 exe
///   文件，旧进程不退出则 cargo 无法覆盖产物（链接失败）。
async fn restart_backend(
    lock: &Arc<Mutex<ManagedChild>>,
    workspace: &WorkspaceInfo,
    target: &BackendTarget,
    addr: &str,
) -> Result<()> {
    let mut child = lock.lock().await;

    // Windows 先杀（释放 exe 锁），Unix 留到 build 成功后再杀
    #[cfg(windows)]
    child.kill_tree().await;

    backend::cargo_build(workspace, target).await?;

    #[cfg(not(windows))]
    child.kill_tree().await;

    let new_child = spawn_backend_binary(workspace, target, addr).await?;
    *child = new_child;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PENDING（本轮运行期间有新变更）→ 补跑一轮（不丢变更）。
    #[test]
    fn round_action_rerun_on_pending() {
        assert_eq!(round_action(PENDING), RoundAction::Rerun);
    }

    /// RESTARTING（无新变更）→ 收尾。
    #[test]
    fn round_action_stop_on_restarting() {
        assert_eq!(round_action(RESTARTING), RoundAction::Stop);
    }

    /// IDLE（异常态，正常循环不应出现）→ 收尾，避免空转。
    #[test]
    fn round_action_stop_on_idle() {
        assert_eq!(round_action(IDLE), RoundAction::Stop);
    }
}
