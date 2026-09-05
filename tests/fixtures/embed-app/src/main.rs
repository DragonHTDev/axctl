//! embed + serve fixture：release 内嵌 + SPA fallback 的集成验证。
//!
//! 起一个最小 axum server：`/api/status` + `spa` fallback（前端来自
//! frontend! 内嵌）。release 跑起来后手动 curl 验证：
//!
//!   GET /              → index.html（200）
//!   GET /assets/app.js → JS（200, immutable）
//!   GET /some/route    → index.html（SPA fallback, 200）
//!   GET /api/status    → JSON（API 正常）
//!
//! debug（cargo run）：frontend! 给空包装 → spa 全 404，仅 API 可用。

use axctl_core::embed::FrontendAssets;
use axum::routing::get;
use axum::Router;

fn main() {
    // 绑定 frontend!（release 内嵌 static/，debug 空）
    let assets: FrontendAssets<'static> =
        axctl_core::frontend!("$CARGO_MANIFEST_DIR/static");

    let app = Router::new()
        .route("/api/status", get(|| async { "{\"ok\":true}" }))
        // Router 之间组合用 fallback_service（fallback() 只收 Handler 函数）
        .fallback_service(axctl_core::serve::spa(assets));

    let addr = "127.0.0.1:3947"; // 固定端口便于手动 curl
    println!("axctl-embed-fixture listening on http://{addr}");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });
}
