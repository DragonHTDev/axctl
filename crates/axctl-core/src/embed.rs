//! Frontend asset embedding.
//!
//! Provides a compile-time baked copy of the built frontend (`dist/`) plus a
//! tiny wrapper ([`FrontendAssets`]) that can be passed around and consumed by
//! [`crate::serve`].
//!
//! # Usage
//!
//! ```rust,ignore
//! use axctl_core::embed::FrontendAssets;
//!
//! fn main() {
//!     // release: 编译期内嵌 dist/，二进制自包含
//!     // debug:   不内嵌（前端由 vite dev server 服务）
//!     let assets = axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");
//!
//!     if let Some(file) = assets.get_file("index.html") {
//!         println!("{} bytes", file.contents().len());
//!     }
//! }
//! ```
//!
//! # How it works
//!
//! [`frontend!`](crate::frontend) expands to [`FrontendAssets::embedded`] in
//! release builds (which reads the folder at compile time via `include_dir!`)
//! and [`FrontendAssets::empty`] in debug builds. Debug builds therefore never
//! require `dist/` to exist and never read it at runtime — the dev frontend is
//! served by Vite instead.
//!
//! # Dependency note
//!
//! `include_dir!` expands to code referencing the bare `include_dir::` path,
//! so the consuming crate must depend on `include_dir` directly (same
//! constraint as `axum-vite`'s `embedded_dir!`). Add `include_dir = "0.7"`
//! next to `axctl-core`.
//!
//! 模块级注释面向 docs.rs 使用英文；实现内部的过程性注释使用中文。

/// include_dir 的宏与类型 re-export（供宏展开与类型标注用）。
pub use include_dir::{self, Dir, File};

/// 内嵌前端产物的包装值。
///
/// - release：`Some(dir)`——编译期内嵌目录，运行时不依赖磁盘。
/// - debug：`None`——不内嵌（前端由 vite 服务），`get_file`/`files` 返回空。
///
/// 包装成值而不是裸 `Dir`，是为了：
/// 1. 屏蔽 debug/release 的差异（调用方拿到同一个类型）；
/// 2. 可整体传给 [`crate::serve`] 等消费方，不用各自处理 cfg。
#[derive(Clone, Copy, Debug)]
pub struct FrontendAssets<'a> {
    dir: Option<&'a Dir<'a>>,
}

impl<'a> FrontendAssets<'a> {
    /// release：包一个编译期内嵌的目录。
    pub fn embedded(dir: &'a Dir<'a>) -> Self {
        Self { dir: Some(dir) }
    }

    /// debug：空包装（前端由 vite 服务，无内嵌资源）。
    pub fn empty() -> Self {
        Self { dir: None }
    }

    /// 是否持有内嵌资源（release 为 true，debug 为 false）。
    pub fn is_embedded(&self) -> bool {
        self.dir.is_some()
    }

    /// 取内嵌文件（按完整路径递归查找）；debug 模式或无此文件返回 None。
    pub fn get_file(&self, path: impl AsRef<std::path::Path>) -> Option<&'a File<'a>> {
        self.dir.and_then(|d| d.get_file(path))
    }

    /// 递归迭代所有内嵌文件（含子目录）；debug 模式返回空。
    pub fn files(&self) -> impl Iterator<Item = &'a File<'a>> {
        self.dir.into_iter().flat_map(|d| walk_files(d))
    }
}

/// 递归收集 Dir 下所有文件（include_dir 的 `Dir::files` 只列顶层文件）。
///
/// 已知取舍：每次 collect 成 Vec——对 dist（几百到几千文件）是一次性小
/// 分配，可接受；若未来嵌入数万文件可改惰性递归（手写迭代器 / 栈式
/// 生成器），当前不阻塞。
fn walk_files<'a>(dir: &'a Dir<'a>) -> Vec<&'a File<'a>> {
    let mut out: Vec<&'a File<'a>> = dir.files().collect();
    for sub in dir.dirs() {
        out.extend(walk_files(sub));
    }
    out
}

/// 编译期内嵌前端产物的宏。
///
/// 参数是产物目录路径，支持 `$CARGO_MANIFEST_DIR` 环境变量展开：
///
/// ```rust,ignore
/// let assets = axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");
/// ```
///
/// - release 编译：展开为 [`FrontendAssets::embedded`]，目录在编译期被
///   `include_dir!` 读入并烙进二进制。
/// - debug 编译：展开为 [`FrontendAssets::empty`]，不触发 include_dir!
///   展开（因此 debug 下目录不存在也能编译）。
#[macro_export]
macro_rules! frontend {
    ($path:tt) => {{
        // release：真正内嵌（include_dir! 在编译期读目录）
        #[cfg(not(debug_assertions))]
        {
            static DIR: $crate::embed::Dir<'static> =
                $crate::embed::include_dir::include_dir!($path);
            $crate::embed::FrontendAssets::embedded(&DIR)
        }
        // debug：不内嵌（dev 前端由 vite 服务）
        #[cfg(debug_assertions)]
        {
            $crate::embed::FrontendAssets::empty()
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静态目录存在（fixture 未删）。include_dir! 只接受字面量路径，
    /// 故不能经 const 间接传入——每个测试直接内联。
    fn fixture_dir() -> Dir<'static> {
        include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../tests/fixtures/embed-app/static")
    }

    #[test]
    fn get_file_reads_content() {
        let dir = fixture_dir();
        let index = dir.get_file("index.html").expect("index.html 应存在");
        let text = String::from_utf8_lossy(index.contents());
        assert!(text.contains("axctl embed fixture"), "index.html 内容不符");
    }

    #[test]
    fn get_file_recurses_into_subdir() {
        let dir = fixture_dir();
        let app_js = dir
            .get_file("assets/app.js")
            .expect("assets/app.js 应可递归找到");
        assert!(
            String::from_utf8_lossy(app_js.contents()).contains("console.log"),
            "app.js 内容不符"
        );
    }

    #[test]
    fn get_file_missing_returns_none() {
        let dir = fixture_dir();
        assert!(dir.get_file("nope.txt").is_none());
    }

    #[test]
    fn walk_files_counts_all_files_recursively() {
        let dir = fixture_dir();
        let all = walk_files(&dir);
        // fixture static/ 下有 index.html + assets/app.js 两个文件
        assert_eq!(all.len(), 2, "应递归数到全部文件, got {}", all.len());
        let names: Vec<String> = all.iter().map(|f| f.path().display().to_string()).collect();
        assert!(names.iter().any(|n| n == "index.html"), "got {names:?}");
        assert!(names.iter().any(|n| n == "assets/app.js"), "got {names:?}");
    }
}
