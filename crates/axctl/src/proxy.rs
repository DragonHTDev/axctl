//! 开发模式反向代理。
//!
//! axctl dev 在统一入口端口（默认 3000）起一个 axum 服务：
//! - `/api/*` 请求转发到后端（dev 监听 3001）
//! - 其余请求（含 HMR WebSocket）转发到 vite（5173）
//!
//! WebSocket 隧道是硬需求：vite 的 HMR client 会连"页面加载端口"（即 3000），
//! 不做 WS 转发则 HMR 断裂、前端改动触发整页刷新。
//!
//! 路由策略：所有路径都进统一 handler，内部按 `Upgrade` 头手动分流——
//! 带 WS 升级头 → 尝试提取 `WebSocketUpgrade` 做隧道；普通请求 → HTTP 转发。
//!
//! # HTTP 转发要点
//!
//! - reqwest Client 设为**不跟随重定向**（redirect::Policy::none()）：
//!   后端 302/301 的 `Location` 必须原样透传给浏览器（登录跳转 / OAuth /
//!   表单重定向场景），由浏览器自己决定是否跟随；reqwest 跟随会丢失
//!   Location 头、且 POST 跟随 302 会被改写为 GET。
//! - Client 在 `ProxyState` 中共享（连接复用，keep-alive 生效）。

use std::sync::Arc;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::{SinkExt, StreamExt};
use reqwest::redirect::Policy;
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, http::Uri};

/// 代理后端目标的连接配置。
#[derive(Clone)]
pub struct ProxyTarget {
    /// 目标主机（如 `127.0.0.1`）。
    pub host: String,
    /// 目标端口（如 3001 或 5173）。
    pub port: u16,
}

/// 代理应用状态：共享 reqwest 客户端 + 后端/vite 两个目标。
#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    backend: Arc<ProxyTarget>,
    vite: Arc<ProxyTarget>,
}

/// 构造代理用的 reqwest 客户端。
///
/// - 不跟随重定向：30x 必须原样透传（见模块注释）。
fn new_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("failed to build reqwest client")
}

/// 构建反向代理路由。
pub fn build_proxy_router(backend: ProxyTarget, vite: ProxyTarget) -> Router {
    let state = ProxyState {
        // 不跟随重定向：30x 必须原样透传（见模块注释）
        client: new_client(),
        backend: Arc::new(backend),
        vite: Arc::new(vite),
    };

    Router::new()
        // /api/* 走 HTTP 转发到后端
        .route("/api/{*rest}", any(api_http_handler))
        // 其余路径（含根路径 /）：WS 升级 → 隧道；HTTP → 转发 vite
        .route("/", any(vite_handler))
        .route("/{*rest}", any(vite_handler))
        .with_state(state)
}

/// hop-by-hop 请求头：不应转发给上游（连接管理由 reqwest 负责）。
const HOP_BY_HOP_REQUEST: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// hop-by-hop 响应头：不应透传给浏览器。
const HOP_BY_HOP_RESPONSE: &[&str] = &["connection", "keep-alive", "transfer-encoding"];

/// /api/* 的 HTTP 转发 handler。
async fn api_http_handler(
    State(state): State<ProxyState>,
    req: Request<axum::body::Body>,
) -> Response {
    proxy_http(&state.client, req, &state.backend).await
}

/// 其余路径的统一 handler：按 `Upgrade` 头手动分流。
///
/// 带 `Upgrade: websocket` → 手动提取 `WebSocketUpgrade` 做隧道；
/// 普通请求 → HTTP 转发到 vite。
async fn vite_handler(
    State(state): State<ProxyState>,
    req: Request<axum::body::Body>,
) -> Response {
    // 判断是否 WS 升级请求
    let is_ws = req
        .headers()
        .get("upgrade")
        .map(|v| v.to_str().map(|s| s.eq_ignore_ascii_case("websocket")).unwrap_or(false))
        .unwrap_or(false);

    if is_ws {
        // 手动从请求 parts 提取 WebSocketUpgrade（extractor 方式）
        let (parts, body) = req.into_parts();
        match WebSocketUpgrade::from_request_parts(&mut parts.clone(), &()).await {
            Ok(upgrade) => {
                let target = state.vite.clone();
                // 原请求的路径+query（vite HMR 靠 ?token= 鉴权）与子协议
                // 必须透传给 vite（vite 服务端只接受带 `Sec-WebSocket-Protocol:
                // vite-hmr` 的升级请求），否则 HMR 初次连代理端口失败、被迫
                // 回退直连 vite 端口（dev 时 vite 未暴露给浏览器则整段断裂）。
                let ws_path_query = parts
                    .uri
                    .path_and_query()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_else(|| "/".to_string());
                let ws_subprotocol = parts
                    .headers
                    .get("sec-websocket-protocol")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                // RFC 6455：客户端带子协议请求时，服务器必须回选其一（响应头
                // `Sec-WebSocket-Protocol`）。axum 默认不回选（protocol=None），
                // 浏览器会据此判定握手失败。这里把客户端请求的子协议全部注册，
                // 让 axum 回选第一个，实现任意子协议的透传代理。
                let echo_protocols: Vec<String> = upgrade
                    .requested_protocols()
                    .map(|v| v.to_str().unwrap_or("").to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let upgrade = upgrade.protocols(echo_protocols);
                return upgrade.on_upgrade(move |socket| async move {
                    tunnel_ws(socket, target, ws_path_query, ws_subprotocol).await;
                });
            }
            Err(_) => {
                // 提取失败（缺少关键头），但请求本身还是要转给 vite
                let req = Request::from_parts(parts, body);
                return proxy_http(&state.client, req, &state.vite).await;
            }
        }
    }

    proxy_http(&state.client, req, &state.vite).await
}

/// WebSocket 隧道：浏览器侧 axum socket ↔ vite 侧 tungstenite 双向转发。
async fn tunnel_ws(
    socket: axum::extract::ws::WebSocket,
    target: Arc<ProxyTarget>,
    path_query: String,
    subprotocol: Option<String>,
) {
    let uri: Uri = format!(
        "ws://{}:{}{}",
        target.host, target.port, path_query
    )
    .parse()
    .unwrap_or_else(|_| {
        // 路径/query 含非法字符等极端情况：退化为裸连接
        format!("ws://{}:{}", target.host, target.port)
            .parse()
            .expect("invalid fallback ws uri")
    });

    // 用带请求头的 client 请求（透传子协议）；vite 服务端只在子协议命中
    // HMR_HEADER("vite-hmr") 时才 handleUpgrade，缺了它握手永不完成。
    let mut builder = ClientRequestBuilder::new(uri.clone());
    if let Some(proto) = subprotocol {
        // 上游可能同时声明多个子协议，逐个带过去由 vite 挑选
        for p in proto.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            builder = builder.with_sub_protocol(p.to_string());
        }
    }
    let conn_result = tokio_tungstenite::connect_async(builder).await;

    match conn_result {
        Ok((vite_ws, _)) => {
            tracing::debug!(target: "axctl.proxy", url = %uri, "websocket tunnel established");
            // futures 的 split() 返回 (SplitSink, SplitStream)：
            // 前者是写端（send），后者是读端（next）
            let (mut browser_tx, mut browser_rx) = socket.split();
            let (mut vite_tx, mut vite_rx) = vite_ws.split();
            let to_vite = async {
                while let Some(Ok(msg)) = browser_rx.next().await {
                    let Some(vite_msg) = axum_to_tungstenite(msg) else { break };
                    if vite_tx.send(vite_msg).await.is_err() {
                        break;
                    }
                }
            };
            let to_browser = async {
                while let Some(Ok(msg)) = vite_rx.next().await {
                    let Some(browser_msg) = tungstenite_to_axum(msg) else { break };
                    if browser_tx.send(browser_msg).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! {
                _ = to_vite => {},
                _ = to_browser => {},
            }
            tracing::debug!(target: "axctl.proxy", "websocket tunnel closed");
        }
        Err(error) => {
            tracing::error!(target: "axctl.proxy", url = %uri, error = %error, "websocket connect to vite failed");
        }
    }
}

/// HTTP 转发：用共享 reqwest client 请求目标，流式转发 body。
///
/// 独立函数便于单测（传入已构造的 client 与 target）。
async fn proxy_http(
    client: &reqwest::Client,
    req: Request<axum::body::Body>,
    target: &ProxyTarget,
) -> Response {
    let uri = req.uri().clone();
    let url = format!(
        "http://{}:{}{}",
        target.host,
        target.port,
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("")
    );

    let mut builder = client.request(req.method().clone(), &url);
    for (name, value) in req.headers() {
        // 跳过 hop-by-hop 头（连接管理由 reqwest 负责）
        if HOP_BY_HOP_REQUEST
            .iter()
            .any(|h| name.as_str().eq_ignore_ascii_case(h))
        {
            continue;
        }
        builder = builder.header(name, value);
    }

    // 请求体流式转发（不整体读进内存）。
    // BodyDataStream 的 Item 是 Result<Bytes, axum::Error>，其 Error 可
    // Into<Box<dyn Error + Send + Sync>>，满足 wrap_stream 的约束。
    let body_stream = req.into_body().into_data_stream();
    builder = builder.body(reqwest::Body::wrap_stream(body_stream));

    match builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let stream = resp.bytes_stream();
            let mut response = Response::builder().status(status);
            for (name, value) in &headers {
                // 响应侧 hop-by-hop 头不透传；content-length 由 axum 从
                // body 流重新计算，避免与流式 body 冲突。
                if HOP_BY_HOP_RESPONSE
                    .iter()
                    .any(|h| name.as_str().eq_ignore_ascii_case(h))
                    || name == "content-length"
                {
                    continue;
                }
                response = response.header(name, value);
            }
            response
                .body(axum::body::Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(error) => {
            tracing::error!(target: "axctl.proxy", %url, error = %error, "proxy request failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// axum Message → tungstenite Message。
fn axum_to_tungstenite(msg: Message) -> Option<tokio_tungstenite::tungstenite::Message> {
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    match msg {
        Message::Text(t) => Some(WsMessage::Text(t.to_string())),
        Message::Binary(b) => Some(WsMessage::Binary(b.to_vec())),
        Message::Ping(p) => Some(WsMessage::Ping(p.to_vec())),
        Message::Pong(p) => Some(WsMessage::Pong(p.to_vec())),
        Message::Close(_) => None,
    }
}

/// tungstenite Message → axum Message。
fn tungstenite_to_axum(msg: tokio_tungstenite::tungstenite::Message) -> Option<Message> {
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    match msg {
        WsMessage::Text(t) => Some(Message::Text(t.into())),
        WsMessage::Binary(b) => Some(Message::Binary(b.into())),
        WsMessage::Ping(p) => Some(Message::Ping(p.into())),
        WsMessage::Pong(p) => Some(Message::Pong(p.into())),
        WsMessage::Close(_) => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header;
    use axum::response::Redirect;

    /// 起一个本地测试服务并返回其 (地址, target)。
    ///
    /// 服务包含两条路由：
    /// - `/redirect` → 307（axum Redirect::temporary）到 `/target`
    /// - `/target` → 200 "landed"
    async fn spawn_test_server() -> (String, ProxyTarget) {
        let app = Router::new()
            .route("/redirect", axum::routing::get(|| async {
                Redirect::temporary("/target")
            }))
            .route("/target", axum::routing::get(|| async { "landed" }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = addr.ip().to_string();
        let port = addr.port();
        (format!("{host}:{port}"), ProxyTarget { host, port })
    }

    /// 核心：30x 必须原样透传（Location 头保留），不能被 reqwest 跟随。
    ///
    /// axum 的 `Redirect::temporary` 产生 307；302/307 同属 30x 重定向，
    /// 断言重点是"透传重定向本身 + Location"，而非具体 30x 码。
    #[tokio::test]
    async fn proxy_http_passes_through_redirect() {
        let (_addr, target) = spawn_test_server().await;
        let client = new_client();

        // 构造发往代理的请求：GET /redirect
        let req = Request::builder()
            .uri("/redirect")
            .body(Body::empty())
            .unwrap();

        let resp = proxy_http(&client, req, &target).await;
        assert!(resp.status().is_redirection(), "30x 应原样透传, got {}", resp.status());
        assert_eq!(
            resp.headers().get(header::LOCATION).map(|v| v.to_str().unwrap()),
            Some("/target"),
            "Location 头应保留"
        );
    }

    /// 正常 200 响应能流式透传 body。
    #[tokio::test]
    async fn proxy_http_forwards_body() {
        let (_addr, target) = spawn_test_server().await;
        let client = new_client();

        let req = Request::builder()
            .uri("/target")
            .body(Body::empty())
            .unwrap();

        let resp = proxy_http(&client, req, &target).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], b"landed");
    }

    /// WS 隧道端到端：验证子协议回选 + token 透传 + 消息往返。
    ///
    /// 复刻 vite HMR 的握手约束：上游服务只接受带 `Sec-WebSocket-Protocol:
    /// vite-hmr` 的升级（缺它不 handleUpgrade），且要求 path 带 ?token。
    /// 此前代理两处丢信息导致浏览器握手失败（design P1-7 未实测）：
    /// 1. 连上游不带原始 path/query → token 丢失
    /// 2. 浏览器↔代理段 axum 默认不回选子协议 → RFC 6455 违规，浏览器拒绝
    #[tokio::test]
    async fn proxy_ws_tunnel_preserves_protocol_and_path() {
        // 1. 起一个模拟 vite HMR 的上游：接受 vite-hmr 子协议的 WS，echo 文本。
        async fn ws_echo_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
            ws.protocols(["vite-hmr"]).on_upgrade(move |mut socket| async move {
                use axum::extract::ws::Message as AxMsg;
                while let Some(Ok(AxMsg::Text(t))) = socket.recv().await {
                    let _ = socket.send(AxMsg::Text(format!("echo:{}", t.as_str()).into())).await;
                }
            })
        }
        let upstream_app = Router::new().route("/", axum::routing::get(ws_echo_handler));
        let up_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_addr = up_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(up_listener, upstream_app).await.unwrap();
        });
        let upstream = ProxyTarget { host: up_addr.ip().to_string(), port: up_addr.port() };
        // 另一个不重要的 HTTP 目标（proxy 路由需要两个 target）
        let dummy = ProxyTarget { host: "127.0.0.1".to_string(), port: 1 };

        // 2. 起 proxy
        let router = build_proxy_router(dummy, upstream);
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, router).await.unwrap();
        });
        let proxy_ws = format!("ws://{proxy_addr}/?token=testtoken123");

        // 3. 用 tungstenite client 连 proxy，带子协议 vite-hmr
        use tokio_tungstenite::tungstenite::ClientRequestBuilder;
        let builder = ClientRequestBuilder::new(proxy_ws.parse().unwrap())
            .with_sub_protocol("vite-hmr");
        let (mut ws, resp) = tokio_tungstenite::connect_async(builder).await.expect("握手应成功");

        // 断言上游回选的子协议被透传给浏览器侧客户端
        let selected = resp.headers().get("sec-websocket-protocol").map(|v| v.to_str().unwrap().to_string());
        assert_eq!(selected, Some("vite-hmr".into()), "应回选 vite-hmr 子协议");

        // 断言消息往返（token 经 path/query 透传后上游正常 echo）
        use futures_util::SinkExt;
        use futures_util::StreamExt;
        ws.send(tokio_tungstenite::tungstenite::Message::Text("ping".into())).await.unwrap();
        if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(reply))) = ws.next().await {
            assert_eq!(reply, "echo:ping", "echo 应往返成功");
        } else {
            panic!("未收到 echo 回复");
        }
    }
}
