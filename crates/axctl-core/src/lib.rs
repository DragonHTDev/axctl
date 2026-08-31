//! Embed and serve frontend assets.
//!
//! This module provides the tooling to embed a Vite build output (`dist/`)
//! into the server binary at compile time (release mode) via rust-embed,
//! and to serve it at runtime with SPA fallback.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

pub mod embed;
pub mod serve;
