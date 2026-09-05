//! `axctl serve` 命令：生产产物预览。
//!
//! 纯前端静态预览——封装 `vite preview`（vite 的官方生产预览），
//! 在**前端项目目录**运行，服务 `vite.config.ts` 的 `build.outDir`
//! （通常 `dist/`）。vite preview 自带 SPA fallback / mime / 缓存头，
//! axctl 不重复实现静态服务。
//!
//! - 无后端、无 API 代理：前端请求 `/api/*` 会失败（因为没有后端可转发）。
//! - 典型用途：`axctl build` 出 dist 后，快速过一眼产物。
//! - 端口默认 4173（vite preview 惯例），`--strictPort` 保证端口被占时报错。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::logging;
use crate::process;
use crate::vite;

/// `vite preview` 默认端口（与 vite 官方预览一致）。
pub const DEFAULT_PREVIEW_ADDR: &str = "127.0.0.1:4173";

/// `axctl serve` 命令参数。
#[derive(Debug, Clone, clap::Args)]
pub struct ServeArgs {
    /// 前端项目根目录（含 vite.config.ts / package.json）。
    /// 默认当前目录。
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// 监听地址（默认 127.0.0.1:4173）。
    #[arg(long, default_value = DEFAULT_PREVIEW_ADDR)]
    pub addr: String,

    /// 启动后自动打开浏览器。
    #[arg(long)]
    pub open: bool,
}

/// `axctl serve` 主流程。
pub async fn run(args: ServeArgs) -> Result<()> {
    // 前端根目录：--dir 优先，否则当前目录（vite preview 需在前端项目跑）
    let frontend_root: PathBuf = match args.dir {
        Some(dir) => dir,
        None => std::env::current_dir().context("failed to get current directory")?,
    };

    // 校验是个前端项目（有 package.json；vite 由 vite preview 自己探测）
    if !frontend_root.join("package.json").is_file() {
        anyhow::bail!(
            "{} has no package.json — run `axctl serve` from the frontend project root, \
             or pass --dir <frontend-root>",
            frontend_root.display()
        );
    }

    logging::info(format!("serving production build from {}", frontend_root.display()));
    logging::info(format!("using vite preview at http://{}", args.addr));

    // 启动 vite preview
    let mut preview = vite::spawn_vite_preview(&frontend_root, &args.addr, args.open)
        .await
        .context("failed to start vite preview")?;

    let (host, port) = crate::config::parse_addr(&args.addr)?;

    // 就绪探测：每轮先查 preview 是否已退出（vite 异步启动，端口被占 /
    // dist 缺失时启动后才会报错退出——必须持续监控，不能只 spawn 后查一次），
    // 存活再做 TCP 探测。TCP 通了才真 ready（否则可能连上端口占用者）。
    let preview_ref = &mut preview;
    let outcome = wait_ready(&host, port, Duration::from_secs(15), move || {
        preview_ref
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.code())
    })
    .await;

    match outcome {
        ReadyOutcome::Ready => {}
        ReadyOutcome::ChildExited(code) => {
            let hint = if port_in_use(port).await {
                format!("port {port} is already in use (vite preview --strictPort exited, code {code:?})")
            } else {
                format!("vite preview exited before becoming ready (code {code:?}); make sure `dist/` exists (run `axctl build` first)")
            };
            logging::error(format!("vite preview failed to start: {hint}"));
            anyhow::bail!("vite preview failed to start: {hint}");
        }
        ReadyOutcome::Timeout => {
            logging::error(format!("vite preview did not become ready within 15s on {}", args.addr));
            preview.kill_tree().await;
            anyhow::bail!("vite preview did not become ready within 15s on {}", args.addr);
        }
    }

    logging::success(format!("preview ready at http://{}", args.addr));
    logging::info("Press Ctrl+C to stop.");

    // 等待 Ctrl+C / SIGTERM
    let ctrl_c = tokio::signal::ctrl_c();
    let mut terminate = std::pin::pin!(process::shutdown_signal());
    tokio::select! {
        _ = ctrl_c => {},
        _ = &mut terminate => {},
    }

    logging::info("shutting down axctl serve...");
    preview.kill_tree().await;
    logging::success("axctl serve stopped");
    Ok(())
}

/// 就绪探测结果。
#[derive(Debug)]
enum ReadyOutcome {
    /// 端口可连（服务就绪）。
    Ready,
    /// 子进程在就绪前退出（携带退出码）。
    ChildExited(Option<i32>),
    /// 超时仍未就绪。
    Timeout,
}

/// 合并"子进程存活探测 + HTTP 就绪探测"的轮询。
///
/// 为什么用 HTTP 而非 TCP：vite preview 异步启动，`--strictPort` 遇端口
/// 被占时启动后才会报错退出。若端口恰被别的服务占用，纯 TCP 探测会连上
/// "占用者"误报 Ready（占用者 accept 但未必是 HTTP 服务）。HTTP 探测发
/// `GET /`，只有真 vite preview（返回 index.html）才响应 200；裸 TCP
/// 占用者不响应，探测失败继续轮询——此时 vite 已因端口冲突退出，被
/// `child_status` 捕获。
///
/// 每轮顺序：
/// 1. `child_status()` 返回 `Some(code)` → 子进程已退出，判 ChildExited；
/// 2. HTTP GET 成功 → 疑似就绪，隔 300ms 复检（子进程仍存活 + HTTP 仍通）
///    才判 Ready，覆盖"首轮连上占用者、vite 随后退出"的竞态窗口；
/// 3. 超时 → Timeout。
async fn wait_ready(
    host: &str,
    port: u16,
    timeout: Duration,
    mut child_status: impl FnMut() -> Option<Option<i32>>,
) -> ReadyOutcome {
    // 给 vite 初判时间：端口冲突 / dist 缺失通常在启动后几百 ms 内退出
    tokio::time::sleep(Duration::from_millis(300)).await;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        // a) 子进程是否已退出（退出即失败，不继续探测）
        if let Some(code) = child_status() {
            return ReadyOutcome::ChildExited(code);
        }
        // b) HTTP 就绪探测（vite preview 就绪后 GET / 返回 200）
        if http_ok(host, port).await {
            tokio::time::sleep(Duration::from_millis(300)).await;
            // 稳定确认：子进程仍存活且 HTTP 仍通才算真 Ready
            if child_status().is_none() && http_ok(host, port).await {
                return ReadyOutcome::Ready;
            }
            // 复检发现子进程退出 → 下一轮循环顶部返回 ChildExited
            continue;
        }
        // c) 超时
        if std::time::Instant::now() >= deadline {
            return ReadyOutcome::Timeout;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// HTTP GET / 是否返回成功（vite preview 就绪的标志）。
///
/// 带短超时：端口占用者若 accept 但不响应 HTTP，reqwest 默认会无限等待，
/// 卡死就绪轮询（走不到子进程退出检查）。2s 超时足够区分"真 vite
/// （立即 200）"与"哑占用者（挂起→超时→false）"。
async fn http_ok(host: &str, port: u16) -> bool {
    let url = format!("http://{host}:{port}/");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok();
    let Some(client) = client else { return false };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// 端口是否已被占用（尝试建立 TCP 连接）。
async fn port_in_use(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 起一个响应 HTTP 200 的本地监听（模拟 vite preview 就绪）。
    async fn spawn_http_ok_listener() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                        .await;
                });
            }
        });
        port
    }

    /// 真 vite（HTTP 200 响应）+ 子进程存活 → Ready。
    #[tokio::test]
    async fn wait_ready_success() {
        let port = spawn_http_ok_listener().await;
        let outcome = wait_ready("127.0.0.1", port, Duration::from_secs(2), || None).await;
        assert!(matches!(outcome, ReadyOutcome::Ready));
    }

    /// 无监听 + 短超时 → Timeout。
    #[tokio::test]
    async fn wait_ready_timeout() {
        let port = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let outcome =
            wait_ready("127.0.0.1", port, Duration::from_millis(600), || None).await;
        assert!(matches!(outcome, ReadyOutcome::Timeout));
    }

    /// 子进程退出（即使端口被占用者监听）→ ChildExited，不误报 Ready。
    #[tokio::test]
    async fn wait_ready_child_exit_beats_port_occupier() {
        // 裸 TCP 占用者：accept 但不响应 HTTP（模拟别的服务占住端口）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                if listener.accept().await.is_err() {
                    break;
                }
            }
        });
        // 子进程"已退出"（code=1）→ 应立即 ChildExited
        let outcome = wait_ready("127.0.0.1", port, Duration::from_secs(2), || {
            Some(Some(1))
        })
        .await;
        assert!(
            matches!(outcome, ReadyOutcome::ChildExited(Some(1))),
            "子进程退出应优先判失败, got {outcome:?}"
        );
    }
}
