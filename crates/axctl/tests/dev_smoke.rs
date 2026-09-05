//! axctl dev 黑盒冒烟测试（集成骨架）。
//!
//! 通过 spawn 真实的 `axctl` 二进制做端到端验证：
//! 起 dev → 代理 200 → 改 fixture 源码 → backend 重启（PID 变化）→ 清理。
//!
//! # 为什么 #[ignore]
//!
//! 需要 node + vite 环境（fixture 的 node_modules 需先 `pnpm install`），
//! 且会真实起进程、占端口。CI 常规流水线跳过，本地冒烟手动跑：
//!
//! ```powershell
//! cargo test -p axctl --test dev_smoke -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// fixture 的定位：workspace 根下的 tests/fixtures/minimal-app。
///
/// CARGO_MANIFEST_DIR = crates/axctl，fixture 在 ../../tests/fixtures/minimal-app。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("..")
        .join("tests").join("fixtures").join("minimal-app")
}

/// axctl 二进制路径：workspace target/debug/axctl(.exe)。
fn axctl_bin() -> PathBuf {
    let mut bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("..")
        .join("target").join("debug").join("axctl");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    bin
}

/// 轮询直到 `check` 返回 true 或超时。
async fn wait_until(timeout: Duration, what: &str, check: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    eprintln!("timed out waiting for {what}");
    false
}

/// 请求 URL 返回状态码；失败返回 None。
async fn http_status(url: &str) -> Option<u16> {
    // 用 reqwest（已是 axctl 的依赖，dev-dependencies 里补上即可）。
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    Some(resp.status().as_u16())
}

/// 请求 /api/status 取 backend pid。
async fn backend_pid(base: &str) -> Option<u32> {
    let client = reqwest::Client::new();
    let resp = client.get(format!("{base}/api/status")).send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body["pid"].as_u64().map(|p| p as u32)
}

/// 黑盒冒烟：起 dev → HTTP 200 → 改源码触发重启 → PID 变化 → 清理。
#[tokio::test]
#[ignore = "需要 node+vite 环境（fixture 需 pnpm install），本地冒烟手动跑"]
async fn dev_smoke_end_to_end() {
    let fixture = fixture_dir();
    assert!(fixture.join("Cargo.toml").is_file(), "fixture 不存在: {}", fixture.display());

    // 1. 起 axctl dev（继承 stdout/stderr 便于诊断）
    let axctl = axctl_bin();
    assert!(axctl.is_file(), "axctl 二进制不存在，先 cargo build: {}", axctl.display());
    let child: Child = Command::new(&axctl)
        .arg("dev")
        .current_dir(&fixture)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn axctl dev");

    // 清理 guard：杀 axctl 进程树 + 恢复 fixture main.rs 原文
    let main_rs = fixture.join("src").join("main.rs");
    let original_src = std::fs::read_to_string(&main_rs).expect("read fixture main.rs");
    struct Guard(Option<Child>, PathBuf, String);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // 恢复 fixture（避免多次运行累积污染 git diff）
            let _ = std::fs::write(&self.1, &self.2);
        }
    }
    let _guard = Guard(Some(child), main_rs.clone(), original_src);

    // 2. 等代理 3000 就绪（首次含 cargo build，给足时间）
    let proxy_ready = wait_until(Duration::from_secs(60), "proxy :3000", || {
        // 同步阻塞式探测（测试里简化处理）
        std::net::TcpStream::connect_timeout(
            &"127.0.0.1:3000".parse().unwrap(),
            Duration::from_millis(500),
        )
        .is_ok()
    })
    .await;
    assert!(proxy_ready, "axctl dev 代理未在 60s 内就绪");

    // 3. 后端代理 200 + 拿 pid
    let status = http_status("http://127.0.0.1:3000/api/status").await;
    assert_eq!(status, Some(200), "GET /api/status 应 200");
    let pid_before = backend_pid("http://127.0.0.1:3000").await;
    assert!(pid_before.is_some(), "应能读到 backend pid");

    // 4. 改 fixture src/main.rs 触发重启
    let main_rs = fixture.join("src").join("main.rs");
    let mut src = std::fs::read_to_string(&main_rs).expect("read fixture main.rs");
    src.push_str(&format!("\n// smoke trigger {}\n", std::process::id()));
    std::fs::write(&main_rs, src).expect("write fixture main.rs");

    // 5. 等 pid 变化（重编译 + 重启）
    let pid_before = pid_before.unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut restarted = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(pid_after) = backend_pid("http://127.0.0.1:3000").await {
            if pid_after != pid_before {
                restarted = true;
                break;
            }
        }
    }
    assert!(restarted, "backend 应因源码变更而重启（PID 变化）");
}
