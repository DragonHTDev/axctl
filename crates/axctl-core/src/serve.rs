//! Runtime static file serving with SPA fallback.
//!
//! User servers mount this on `.fallback(...)` so that requests that do
//! not match a static file fall back to `index.html`, supporting frontend
//! history-mode routing.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

// TODO: 实现 `spa()` 路由函数，替代 axum-vite：
// - mime 推断（mime_guess）
// - SPA fallback：找不到文件 → index.html
// - 缓存头控制
// - dev / release 切换帮手
