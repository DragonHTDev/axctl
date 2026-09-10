//! axctl build 黑盒冒烟测试（集成骨架）。
//!
//! 覆盖两个核心机制：
//! 1. **哨兵传导**：改 dist/.axctl-sentinel → cargo 重跑 build.rs → 主 crate
//!    重编（build-stamp 变化）——这是"改前端后 release 内嵌新产物"的基础。
//! 2. **release 内嵌**：cargo build --release 后跑 exe，`/` 返回内嵌的
//!    index.html、`/api/status` 返回 200。
//!
//! # 为什么 #[ignore]
//!
//! 需要 node + vite 环境（fixture 需先 `pnpm install` + `vite build` 产出
//! dist）。CI 常规流水线跳过，本地手动跑：
//!
//! ```powershell
//! # 先构建 fixture 前端（产出 dist/）
//! cd tests\fixtures\minimal-app
//! pnpm exec vite build
//! # 再跑冒烟
//! cargo test -p axctl --test build_smoke -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// minimal-app fixture 目录（workspace 根下）。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("minimal-app")
}

/// 递归找 build-stamp.txt（target/debug/build/*/out/build-stamp.txt）。
fn find_build_stamp(fixture: &Path) -> Option<PathBuf> {
    let build_dir = fixture.join("target").join("debug").join("build");
    let entries = std::fs::read_dir(&build_dir).ok()?;
    for entry in entries.flatten() {
        let out = entry.path().join("out").join("build-stamp.txt");
        if out.is_file() {
            return Some(out);
        }
    }
    None
}

/// 读 build-stamp 内容（build.rs 每次重跑写入不同 PID）。
fn read_stamp(stamp: &Path) -> Option<String> {
    std::fs::read_to_string(stamp).ok()
}

/// 在 fixture 跑一次 cargo build（阻塞），返回是否成功。
fn cargo_build(fixture: &Path) -> bool {
    Command::new("cargo")
        .arg("build")
        .current_dir(fixture)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// HTTP GET 返回 body 文本；失败 None。
async fn http_body(url: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.ok()?;
    resp.text().await.ok()
}

/// 等端口可连。
async fn wait_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

/// 终止进程树（测试清理用）。
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/t", "/f"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}

/// 哨兵传导：改哨兵 → build.rs 重跑 → build-stamp 变。
#[tokio::test]
#[ignore = "需 node+vite 且 fixture 先 vite build 产出 dist；本地冒烟手动跑"]
async fn sentinel_rebuild_triggers_recompile() {
    let fixture = fixture_dir();
    assert!(fixture.join("dist").is_dir(), "fixture 缺 dist/——先 `pnpm exec vite build`");

    // 1. 基线 build + 读 stamp
    assert!(cargo_build(&fixture), "baseline cargo build 失败");
    let stamp_file = find_build_stamp(&fixture).expect("找不到 build-stamp.txt");
    let stamp1 = read_stamp(&stamp_file).expect("读 stamp1");

    // 2. 改哨兵内容（追加随机行保证变化）
    let sentinel = fixture.join("dist").join(".axctl-sentinel");
    let original = std::fs::read_to_string(&sentinel).unwrap_or_default();
    std::fs::write(&sentinel, format!("{original}trigger {}\n", std::process::id()))
        .expect("改哨兵");

    // 清理 guard：恢复哨兵
    struct Guard(PathBuf, String);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, &self.1);
        }
    }
    let _guard = Guard(sentinel, original);

    // 3. 再次 build → build.rs 应重跑（stamp 变）
    assert!(cargo_build(&fixture), "第二次 cargo build 失败");
    let stamp2 = read_stamp(&stamp_file).expect("读 stamp2");

    assert_ne!(stamp1, stamp2, "哨兵变后 build.rs 应重跑（build-stamp 应变）");
}

/// release 内嵌：release exe 起服务后 / 返回内嵌 index.html、/api 200。
#[tokio::test]
#[ignore = "需 node+vite 且 fixture 先 vite build 产出 dist；本地冒烟手动跑"]
async fn release_embed_serves_frontend() {
    let fixture = fixture_dir();
    assert!(fixture.join("dist").is_dir(), "fixture 缺 dist/——先 `pnpm exec vite build`");

    // release 编译（若哨兵机制正常，内嵌的是当前 dist）
    let release_ok = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&fixture)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(release_ok, "cargo build --release 失败");

    // 起 release exe（默认 3001）
    let exe = fixture
        .join("target")
        .join("release")
        .join("axctl-fixture-server");
    let mut exe = exe;
    if cfg!(windows) {
        exe.set_extension("exe");
    }
    assert!(exe.is_file(), "release exe 不存在: {}", exe.display());
    let child = Command::new(&exe)
        .current_dir(&fixture)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn release exe");

    struct Guard(std::process::Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            let pid = self.0.id();
            kill_tree(pid);
            let _ = self.0.wait();
        }
    }
    let _guard = Guard(child);

    // 等 3001 就绪 + HTTP 断言
    assert!(wait_port(3001, Duration::from_secs(15)).await, "release server 未就绪");
    // 页面含 fetch 占位（验证内嵌前端确实带 fetch 逻辑被 serve）
    let page = http_body("http://127.0.0.1:3001/")
        .await
        .expect("GET / 无 body");
    assert!(page.contains("id=\"backend\""), "页面应含 backend 挂载点（内嵌前端）");
    // /api/status 应返回可用 JSON（前端 fetch 的数据源）
    let status = http_body("http://127.0.0.1:3001/api/status")
        .await
        .expect("GET /api/status 无 body");
    assert!(status.contains("\"ok\":true"), "GET /api/status 应含 ok:true，实际: {status}");
}
