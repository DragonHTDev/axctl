//! embed fixture 主程序：验证 axctl-core::frontend! 宏 + FrontendAssets。
//!
//! - release：frontend! 展开为 FrontendAssets::embedded，static/ 编译期内嵌
//! - debug：展开为 FrontendAssets::empty，不内嵌不读盘（dev 前端归 vite）

use axctl_core::embed::FrontendAssets;

/// 绑定前端产物（release 内嵌 static/，debug 空）。
///
/// 路径相对本 crate 的 Cargo.toml（CARGO_MANIFEST_DIR = embed-app/），
/// static/ 就在本目录下。
fn assets() -> FrontendAssets<'static> {
    axctl_core::frontend!("$CARGO_MANIFEST_DIR/static")
}

fn main() {
    let assets = assets();

    if !assets.is_embedded() {
        // debug：不内嵌，直接打印说明并正常退出（serve 层不会走到这里）
        println!("axctl-embed-fixture ok: debug mode, not embedded (dev uses vite)");
        return;
    }

    // release：验证内嵌内容
    let index = assets
        .get_file("index.html")
        .expect("index.html should be embedded");
    let text = String::from_utf8_lossy(index.contents());
    assert!(text.contains("<title>axctl embed fixture</title>"), "index.html 内容不符");

    let app_js = assets
        .get_file("assets/app.js")
        .expect("assets/app.js should be embedded");
    assert!(
        String::from_utf8_lossy(app_js.contents()).contains("console.log"),
        "app.js 内容不符"
    );

    assert!(assets.get_file("nope.txt").is_none(), "不存在文件应返回 None");

    let names: Vec<String> = assets.files().map(|f| f.path().display().to_string()).collect();
    assert!(names.iter().any(|n| n == "index.html"), "files 应含 index.html, got {names:?}");
    assert!(names.iter().any(|n| n == "assets/app.js"), "files 应含 assets/app.js, got {names:?}");

    println!("axctl-embed-fixture ok: embedded {} files", names.len());
}
