//! axctl serve 黑盒冒烟测试（集成骨架）。
//!
//! spawn 真实 `axctl serve`，验证 vite preview 封装：
//! 静态 200 + SPA fallback 200。
//!
//! # 为什么 #[ignore]
//!
//! 需要 node + vite 环境（fixture 需 `pnpm install` 且先 `vite build`
//! 产出 dist）。CI 常规流水线跳过，本地手动跑：
//!
//! ```powershell
//! # 先构建 fixture 前端（产出 dist/）
//! cd tests\fixtures\minimal-app
//! pnpm exec vite build
//! # 再跑冒烟
//! cargo test -p axctl-rs --test preview_smoke -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// fixture 的定位：workspace 根下的 tests/fixtures/minimal-app。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("minimal-app")
}

/// axctl 二进制路径。
fn axctl_bin() -> PathBuf {
    let mut bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
        .join("axctl");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    bin
}

/// 起 `axctl serve` 到 127.0.0.1:4173（vite preview 默认），返回子进程 guard。
fn spawn_serve() -> Child {
    let fixture = fixture_dir();
    assert!(fixture.join("dist").is_dir(), "fixture 缺 dist/——先跑 `pnpm exec vite build`");
    let axctl = axctl_bin();
    assert!(axctl.is_file(), "axctl 二进制不存在，先 cargo build");
    Command::new(&axctl)
        .arg("serve")
        .current_dir(&fixture)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn axctl serve")
}

/// HTTP GET 返回状态码；失败 None。
async fn http_status(url: &str) -> Option<u16> {
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.ok()?;
    Some(resp.status().as_u16())
}

/// 等端口可连（vite preview 就绪）。
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

/// serve 冒烟：静态 200 + SPA fallback 200 + 清理。
#[tokio::test]
#[ignore = "需要 node+vite 环境且 fixture 先 vite build 产出 dist；本地冒烟手动跑"]
async fn serve_smoke_static_and_spa() {
    let child = spawn_serve();

    // 清理 guard：杀 axctl 的**整棵进程树**（axctl serve 经 cmd /C 起 vite
    // preview → node；只 kill 根会残留孤儿 node）。
    struct Guard(Option<Child>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                // std::process::Child::id() 返回 u32（非 Option）
                kill_tree(child.id());
                let _ = child.wait();
            }
        }
    }
    let _guard = Guard(Some(child));

    // 等 4173 就绪
    assert!(wait_port(4173, Duration::from_secs(20)).await, "axctl serve 未在 20s 内就绪");

    // 静态：根路径 200
    assert_eq!(http_status("http://127.0.0.1:4173/").await, Some(200));

    // SPA fallback：假路由也 200（vite preview 回退 index.html）
    assert_eq!(http_status("http://127.0.0.1:4173/some/spa/route").await, Some(200));
}

/// 终止 pid 所在进程树（测试清理用；集成测试不便依赖 axctl crate 内部
/// 函数，故在此写平台分支小辅助）。
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        // taskkill 连树强杀（cmd /C → node 整链）
        let _ = Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/t", "/f"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // Unix：kill 根进程（子进程通常随父退出；测试场景足够）
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}
