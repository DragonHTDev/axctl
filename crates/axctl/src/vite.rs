//! Vite dev server 的探测与启动。
//!
//! dev 模式下优先复用用户已启动的 vite（对应 tauri 的 devUrl 心智），
//! 未启动时才由 axctl 拉起。

use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::process::ManagedChild;

/// vite dev server 的默认地址。
pub const DEFAULT_DEV_HOST: &str = "127.0.0.1";
/// vite dev server 的默认端口。
pub const DEFAULT_DEV_PORT: u16 = 5173;

/// 探测给定 host:port 上是否有 **vite dev server** 就绪。
///
/// 与旧版的关键差异：不只要求 HTTP 200——请求 `/@vite/client`（vite 的
/// 注入模块路径），只有 vite 才返回 200 且内容含 "vite"。这避免 5173 被
/// 无关服务占用时误判"复用成功"。
pub async fn http_ready(host: &str, port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if is_vite(host, port).await {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// 请求 /@vite/client 判断目标是否真是 vite dev server。
async fn is_vite(host: &str, port: u16) -> bool {
    let url = format!("http://{host}:{port}/@vite/client");
    match reqwest::get(&url).await {
        Ok(response) if response.status().is_success() => {
            // vite 的 @vite/client 响应是 JS，通常含 "vite" 字样
            match response.text().await {
                Ok(body) => body.contains("vite") || body.contains("@vite"),
                Err(_) => false,
            }
        }
        _ => false,
    }
}

/// 检查端口是否被占用（尝试建立 TCP 连接）。
#[allow(dead_code)]
pub async fn port_in_use(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

/// 在当前目录探测包管理器，返回可用于启动脚本的命令前缀。
///
/// 返回形如 `["pnpm", "exec"]` 或 `["npm", "run"]` 的片段，
/// 后续拼上子命令即可（如 `pnpm exec vite` / `npm run dev`）。
pub fn package_manager() -> &'static str {
    // TODO: 尊重用户配置（packageManager 字段 / [package.metadata.axctl]）。
    // 先按存在性探测：pnpm > npm。
    if command_exists("pnpm") {
        "pnpm"
    } else {
        "npm"
    }
}

/// 判断命令是否存在（尽力而为）。
fn command_exists(name: &str) -> bool {
    let check = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", &format!("where {name} >nul 2>nul")])
            .status()
    } else {
        std::process::Command::new("sh")
            .args(["-c", &format!("command -v {name} >/dev/null 2>&1")])
            .status()
    };
    matches!(check, Ok(status) if status.success())
}

/// 启动 vite dev server（使用项目的包管理器）。
///
/// 返回受管子进程句柄；调用方负责在退出时 kill。
/// Windows 下 `.cmd` shim 的处理由 `spawn_command` 统一负责。
pub async fn spawn_vite(project_root: &Path) -> Result<ManagedChild> {
    let pm = package_manager();
    // dev 统一用 `exec vite`（npm/pnpm 对称），不经 package.json 的 dev script——
    // 与 pnpm exec vite 语义一致，避免 npm run dev 与 pnpm exec vite 行为分叉。
    let command = match pm {
        "pnpm" => "pnpm exec vite".to_string(),
        _ => "npm exec vite".to_string(),
    };
    ManagedChild::spawn_command("vite", &command, Some(project_root), &[])
}

/// 启动 `vite preview` 服务构建产物（生产预览）。
///
/// vite preview 会读项目的 vite.config.ts，服务其 `build.outDir`（通常
/// dist/），并自带 SPA fallback / mime / 缓存——比 axctl 自研静态服务
/// 更可靠且零新增依赖。
///
/// `addr` 形如 `127.0.0.1:4173`；用 `--strictPort` 让端口被占时直接
/// 报错而非静默换端口。`open` 为 true 时让 vite 自动开浏览器。
pub async fn spawn_vite_preview(
    project_root: &Path,
    addr: &str,
    open: bool,
) -> Result<ManagedChild> {
    let (host, port) = crate::config::parse_addr(addr)?;
    let pm = package_manager();
    // --no-install：本地 node_modules 没有 vite 时立即报错（与文档一致），
    // 不让 npx 联网静默装包。
    let command = match pm {
        "pnpm" => format!(
            "pnpm exec vite preview --host {host} --port {port} --strictPort{}",
            open_flag(open)
        ),
        _ => format!(
            "npx --no-install vite preview --host {host} --port {port} --strictPort{}",
            open_flag(open)
        ),
    };
    ManagedChild::spawn_command("vite-preview", &command, Some(project_root), &[])
}

/// `open` → " --open"，否则空串。
fn open_flag(open: bool) -> &'static str {
    if open {
        " --open"
    } else {
        ""
    }
}

/// 跑一次 `vite build`（生产构建），阻塞到完成。
///
/// 与 dev/serve 的 spawn 不同：vite build 是一次性任务（非常驻），
/// 直接等退出码。stdout/stderr 继承——用户要看到 vite 的构建输出与
/// 产物报告。
///
/// 返回前 `vite.config.ts` 的 `build.outDir`（默认 `dist/`）应已产出。
pub async fn run_vite_build(project_root: &Path) -> Result<()> {
    let pm = package_manager();
    let (program, args) = match pm {
        "pnpm" => ("pnpm", vec!["exec", "vite", "build"]),
        _ => ("npm", vec!["exec", "vite", "build"]),
    };
    // Windows 下 pnpm/npm 是 .cmd shim，须经 cmd /C（spawn_command 的职责）。
    // 这里复用 ManagedChild::spawn_command 的 Windows 处理，但 build 需要
    // 等退出，故先 spawn 再 wait。
    let command = format!("{} {}", program, args.join(" "));
    let mut child = ManagedChild::spawn_command("vite-build", &command, Some(project_root), &[])?;
    let status = child.wait().await?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("vite build failed (exit {:?})", status.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_flag_toggle() {
        assert_eq!(open_flag(true), " --open");
        assert_eq!(open_flag(false), "");
    }
}
