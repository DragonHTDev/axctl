//! axctl 哨兵契约：让 cargo 感知 dist 变化（axctl build 每次写哨兵）。
//!
//! 这是真实用户项目应写的 build.rs 契约——axctl build 在 vite build 后
//! 写 dist/.axctl-sentinel，本文件声明 rerun-if-changed 监控它：
//! 哨兵变 → build.rs 重跑 → 主 crate 重编 → frontend! 宏重读 dist。
//!
//! server 经 frontend! 内嵌 dist（见 main.rs 的 spa fallback）；本 build.rs
//! 实证哨兵传导：哨兵变 → 本文件重跑 → 主 crate 重编。

fn main() {
    // 哨兵文件：dist 在 Cargo.toml 同目录
    println!("cargo:rerun-if-changed=dist/.axctl-sentinel");
    // 兜底：dist 目录本身（部分构建工具只改目录 mtime）
    println!("cargo:rerun-if-changed=dist");

    // 运行标记：每次 build.rs 重跑都写入 OUT_DIR，便于验证传导
    let out = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let stamp = std::process::id().to_string();
    std::fs::write(std::path::Path::new(&out).join("build-stamp.txt"), &stamp)
        .expect("write build stamp");
    // 让主 crate 能读到 stamp：编译期环境变量
    println!("cargo:rustc-env=AXCTL_FIXTURE_BUILD_STAMP={stamp}");
}
