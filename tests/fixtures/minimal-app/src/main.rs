//! 最小 axum 后端，用于验证 axctl dev 的代理与热重载。

use std::net::SocketAddr;

use axum::{Router, Json, routing::get};

/// 简单的 API 响应。
#[derive(serde::Serialize)]
struct Status {
    ok: bool,
    message: &'static str,
    pid: u32,
}

#[tokio::main]
async fn main() {
    // build.rs 编译期注入的构建标记：哨兵传导实证用（每次重编变化）
    println!(
        "[axctl-fixture] build stamp = {}",
        env!("AXCTL_FIXTURE_BUILD_STAMP")
    );
    // 从环境变量读取监听地址（axctl 传入），默认 127.0.0.1:3001
    let addr: SocketAddr = std::env::var("AXCTL_BACKEND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3001".to_string())
        .parse()
        .expect("invalid backend addr");

    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/hello", get(hello))
        // release：内嵌 dist 服务前端（SPA fallback）；debug：frontend! 空包装
        .fallback_service(axctl_core::serve::spa(axctl_core::frontend!(
            "$CARGO_MANIFEST_DIR/dist"
        )));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    // 启动横幅。不用 axctl 的 "Info" 前缀风格——那是 axctl 自己的输出约定，
    // fixture 作为被测程序照抄会让人在混合日志里分不清来源（尤其
    // RUST_LOG=debug 时两者同屏）。改用带自身标识的独立横幅（品红加粗）。
    eprintln!(
        "{}",
        console::style(format!("[axctl-fixture] backend listening on http://{addr}"))
            .bold()
            .magenta()
    );
    axum::serve(listener, app).await.expect("server error");
}

/// 返回进程 PID，便于验证"重启后 PID 变化"。
async fn status() -> Json<Status> {
    Json(Status {
        ok: true,
        message: "hello from axctl fixture",
        pid: std::process::id(),
    })
}

/// 一个简单的文本接口。
async fn hello() -> &'static str {
    "hello axctl"
}

