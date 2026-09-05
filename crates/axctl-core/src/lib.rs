//! Embed and serve frontend assets.
//!
//! This crate provides the tooling to embed a Vite build output (`dist/`)
//! into the server binary at compile time (release mode) via
//! [`frontend!`](crate::frontend), and to serve it at runtime with SPA
//! fallback.
//!
//! # Modules
//!
//! - [`embed`] — release-mode asset embedding: [`frontend!`](crate::frontend)
//!   macro + [`embed::FrontendAssets`] wrapper value.
//! - [`serve`] — SPA static serving on top of embedded assets.
//! - [`probe`] — environment probing (Rust / Node / package manager).
//!
//! # Frontend embedding in one line
//!
//! ```rust,ignore
//! use axctl_core::embed::FrontendAssets;
//!
//! // release: 编译期内嵌 dist/；debug: 空包装（前端由 vite 服务）
//! let assets: FrontendAssets<'static> =
//!     axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");
//! ```
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

/// Release-mode frontend asset embedding: `frontend!` macro + [`embed::FrontendAssets`].
pub mod embed;
/// Environment probing (Rust / Node / package manager / platform).
pub mod probe;
/// Runtime static file serving with SPA fallback.
pub mod serve;
