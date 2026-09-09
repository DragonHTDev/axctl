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

    // 为 Windows 可执行文件嵌入版本/公司/产品等文件信息（供打包验证）。
    // 非 Windows 平台无操作。版本号从 Cargo.toml 动态读，避免双份维护。
    #[cfg(target_os = "windows")]
    embed_windows_resources();
}

/// 嵌入 Windows 资源：文件版本、产品、公司、版权等元数据。
#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    let mut resource = winres::WindowsResource::new();
    let version = parse_version(env!("CARGO_PKG_VERSION"));
    resource.set("FileVersion", &version);
    resource.set("ProductVersion", &version);
    resource.set("FileDescription", "Axctl Fixture Server");
    resource.set("ProductName", "Axctl Fixture Server");
    resource.set("CompanyName", "axctl");
    resource.set("LegalCopyright", "Copyright (c) 2026 axctl");
    resource
        .compile()
        .expect("failed to embed Windows resources");
}

/// 将 `x.y.z` 规范化为 Windows 的四段 `x,y,z,0` 版本号。
#[cfg(target_os = "windows")]
fn parse_version(version: &str) -> String {
    let mut parts: Vec<String> = version
        .split('.')
        .map(|part| part.chars().filter(|c| c.is_ascii_digit()).collect())
        .filter(|part: &String| !part.is_empty())
        .collect();
    while parts.len() < 4 {
        parts.push("0".to_string());
    }
    parts.truncate(4);
    parts.join(",")
}
