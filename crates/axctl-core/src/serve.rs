//! SPA static serving on top of embedded assets.
//!
//! Mount the returned router as the fallback of your API server so that:
//! - real files in the embedded frontend (`dist/`) are served with proper
//!   mime / cache headers / ETag;
//! - the root path and extension-less paths (SPA client-side routes) fall
//!   back to `index.html`, so browser navigation works;
//! - extension-bearing paths that do not exist return a real 404 — `fetch()`
//!   calls for a missing `.json` / `.js` / image never receive HTML.
//!
//! # Usage
//!
//! ```rust,ignore
//! use axctl_core::frontend;
//! use axum::Router;
//!
//! // 前端产物（release 内嵌 / debug 空包装）
//! let assets = frontend!("$CARGO_MANIFEST_DIR/../dist");
//!
//! // API 挂 /api，其余全部交给 SPA 前端
//! let app = Router::new()
//!     .nest("/api", api_routes)                       // 业务 API 优先
//!     .fallback_service(axctl_core::serve::spa(assets)); // Router 组合用 fallback_service
//! ```
//!
//! # Known boundary: dot-in-path heuristic
//!
//! The extension heuristic is a double-edged sword. It makes SPA routes
//! verifiable with `curl` (which sends `Accept: */*`), but the flip side is
//! that an extension-bearing **valid SPA route** is treated as a missing asset
//! and returns 404 — e.g. `/order/2024.12`, `/user/v1.2/profile`. If your
//! client-side routes may contain dots, either keep them dot-free or mount an
//! explicit route / allow-list for those paths in your own router. Do not file
//! this as a bug; it is a deliberate trade-off (see design §6.4).
//!
//! # Debug / release
//!
//! [`FrontendAssets`](crate::embed::FrontendAssets) from
//! [`frontend!`](crate::frontend) is empty in debug builds — `spa` then
//! answers 404 for everything, because in dev the frontend is served by
//! Vite, not by this router.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;

use crate::embed::{File, FrontendAssets};

/// 挂载 SPA fallback 路由。
///
/// 用 catch-all 单 handler：按路径查内嵌文件，命中则带缓存头返回；
/// 未命中时按**路径扩展名**判断——无扩展名（SPA 客户端路由）回退
/// index.html，带扩展名（真实资源请求）返回 404，避免 `fetch()` 误拿
/// HTML。handler 不 extract state，故返回 [`Router`]`<S>` 可并入任意
/// 带 state 的用户 router。
pub fn spa<S>(assets: FrontendAssets<'static>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // assets 是 Copy（内部是 Option<&Dir>），两个 route 各捕获一份。
    // any() 让所有 method 都进（release 实际只有 GET 有意义，但 fallback
    // 收到非 GET 也应走 404 而非 405）。
    let root_handler = move |req: Request| async move { handle_spa(req, &assets).await };
    let catchall_handler = move |req: Request| async move { handle_spa(req, &assets).await };

    Router::new()
        .route("/", any(root_handler))
        .route("/{*path}", any(catchall_handler))
}

/// 处理单个 SPA 请求。
async fn handle_spa(req: Request, assets: &FrontendAssets<'static>) -> Response {
    // debug（empty）或未内嵌：不 serve 任何前端（dev 由 vite 服务）
    if !assets.is_embedded() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // 拆请求：方法、headers、路径
    let method = req.method().clone();
    let headers = req.headers().clone();
    let uri = req.uri().clone();
    let path = uri.path().trim_start_matches('/');

    // 1) 真实文件命中 → 直接返回（带缓存头 / ETag）
    if let Some(file) = assets.get_file(path) {
        return serve_file(file, &headers);
    }

    // 2) 根路径：浏览器输域名访问根 → 永远回 index.html（无条件）
    if path.is_empty() {
        if let Some(index) = assets.get_file("index.html") {
            return serve_file(index, &headers);
        }
        return StatusCode::NOT_FOUND.into_response();
    }

    // 3) 未命中子路径 → 判定该不该回退 index.html：
    //    - 路径**不含扩展名**（SPA 客户端路由，如 /console/123）：回退，
    //      浏览器导航与 curl 验证（Accept 默认 */*）都应拿到 HTML；
    //    - 路径**含扩展名**（fetch 的 /data.json、缺失的 /img.png）：
    //      真 404，绝不返回 HTML（否则 fetch 解析崩）。
    //    附加：非 GET 请求不回退（POST /submit 之类不该得 HTML）。
    let looks_like_navigation = method == axum::http::Method::GET && !has_extension(path);

    if looks_like_navigation {
        if let Some(index) = assets.get_file("index.html") {
            return serve_file(index, &headers);
        }
    }

    StatusCode::NOT_FOUND.into_response()
}

/// 路径末段是否含文件扩展名（`.js` / `.json` / `.png` 等）。
fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|seg| seg.rsplit_once('.').is_some())
}

/// 返回一个内嵌文件（mime / 缓存头 / ETag / 304）。
///
/// `File<'static>` 由 `FrontendAssets<'static>::get_file` 产出（内嵌数据
/// 生命周期为 'static），`contents()` 才能零拷贝借出 &'static [u8]。
fn serve_file(file: &'static File<'static>, request_headers: &axum::http::HeaderMap) -> Response {
    let path_buf = PathBuf::from(file.path());
    let resolved_mime = mime_guess::from_path(&path_buf)
        .first_or_octet_stream()
        .to_string();

    // 缓存头分层（对齐 axum-vite 验证过的策略）：
    //   - HTML：no-store（入口恒重验）
    //   - assets/ 下：1 年 immutable（Vite 内容 hash 文件名）
    //   - 其余（favicon / robots / public 根）：no-cache（未 hash 需重验）
    let cache_header = if resolved_mime.contains("text/html") {
        "no-store"
    } else if path_buf.components().any(|c| c.as_os_str() == "assets") {
        "public, max-age=31536000, immutable"
    } else {
        "public, no-cache"
    };

    // ETag：内容 hash。客户端带 If-None-Match 且匹配 → 304
    let etag = etag_of(file.contents());
    if let Some(if_none_match) = request_headers.get(header::IF_NONE_MATCH) {
        if if_none_match.as_bytes() == etag.as_bytes() {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(Body::empty())
                .unwrap();
        }
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, resolved_mime)
        .header(header::CACHE_CONTROL, cache_header)
        .header(header::ETAG, etag)
        // file.contents() 是 &'static [u8]（include_dir 内嵌），零拷贝
        .body(Body::from(file.contents()))
        .unwrap()
}

/// 由文件内容算 ETag（不引额外依赖，用 std 的 DefaultHasher 简化——
/// 只要"内容变 → etag 变"即可，不要求跨进程一致或防碰撞）。
fn etag_of(contents: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hasher};
    let mut h = DefaultHasher::new();
    h.write(contents);
    format!("\"{:016x}\"", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{header, Request as HttpRequest};
    use tower::ServiceExt;

    /// 用 fixture 的 static/ 构造 embedded assets（测试不依赖 release cfg）。
    fn test_assets() -> FrontendAssets<'static> {
        static DIR: crate::embed::Dir<'static> = crate::embed::include_dir::include_dir!(
            "$CARGO_MANIFEST_DIR/../../tests/fixtures/embed-app/static"
        );
        FrontendAssets::embedded(&DIR)
    }

    /// 空 assets（模拟 debug 的 frontend!）。
    fn empty_assets() -> FrontendAssets<'static> {
        FrontendAssets::empty()
    }

    async fn get(router: Router<()>, uri: &str) -> Response {
        router
            .oneshot(
                HttpRequest::builder()
                    .uri(uri)
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// 根路径 → index.html + no-store。
    #[tokio::test]
    async fn root_serves_index() {
        let router = spa::<()>(test_assets());
        let resp = get(router, "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("axctl embed fixture"));
    }

    /// assets/ 下文件 → 200 + immutable + JS mime。
    #[tokio::test]
    async fn asset_file_served_immutable() {
        let router = spa::<()>(test_assets());
        let resp = get(router, "/assets/app.js").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        let ct = resp.headers()[header::CONTENT_TYPE].to_str().unwrap();
        assert!(
            ct.starts_with("application/javascript") || ct.starts_with("text/javascript"),
            "ct={ct}"
        );
    }

    /// SPA 路由（无扩展名路径，如 /some/client/route）未命中 → 回退 index.html。
    /// 不依赖 Accept 头——curl / 测试工具默认 */* 也应拿到 HTML。
    #[tokio::test]
    async fn spa_navigation_falls_back_to_index() {
        let router = spa::<()>(test_assets());
        let resp = get(router, "/some/client/route").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("axctl embed fixture"));
    }

    /// 带扩展名的缺失路径（fetch /data.json、缺失图片）→ 404，绝不返回 HTML。
    #[tokio::test]
    async fn fetch_missing_returns_404_not_html() {
        let router = spa::<()>(test_assets());
        let resp = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/api-ish/missing.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// 根路径无论 Accept 都回 index.html（浏览器输域名访问根）。
    #[tokio::test]
    async fn root_serves_index_without_html_accept() {
        let router = spa::<()>(test_assets());
        let resp = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header(header::ACCEPT, "*/*")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// 带匹配 If-None-Match → 304。
    #[tokio::test]
    async fn etag_returns_304() {
        let router = spa::<()>(test_assets());
        // 先取一次拿 etag
        let resp = get(router.clone(), "/assets/app.js").await;
        let etag = resp.headers()[header::ETAG].to_str().unwrap().to_string();
        drop(resp);

        // 带 If-None-Match 再请求
        let resp = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/assets/app.js")
                    .header(header::IF_NONE_MATCH, &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    /// debug（empty assets）→ 一律 404。
    #[tokio::test]
    async fn empty_assets_all_404() {
        let router = spa::<()>(empty_assets());
        assert_eq!(get(router.clone(), "/").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get(router, "/assets/app.js").await.status(), StatusCode::NOT_FOUND);
    }
}
