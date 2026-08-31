//! Embedded frontend assets (release mode).
//!
//! User projects use the derive provided here (based on rust-embed) to
//! compile the `dist/` folder into the binary. In debug mode rust-embed
//! reads from disk by default, so frontend changes do not require a
//! rebuild.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

// TODO: 提供 `include_frontend!` 宏与 Assets derive 的封装。
// 参考设计文档 §6.2：rust-embed + 哨兵机制。
