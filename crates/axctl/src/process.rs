//! 跨平台子进程管理。
//!
//! 负责启动 vite / backend 等子进程，并在结束时正确终止进程。
//!
//! # 进程终止策略（对齐 tauri-cli）
//!
//! - backend 采用"先 `cargo build`、再直接 spawn 编译产物"的方式启动，
//!   产物是**叶子进程**，直接 kill 根进程即干净。
//! - vite 等经包管理器（pnpm/npm → node）启动的进程是**进程树**，
//!   杀树策略分平台：
//!   - Windows：`taskkill /pid <pid> /t /f` 连树强制终止。
//!   - Unix：递归 `pgrep -P <pid>` 找全部后代，先 SIGTERM 后补 SIGKILL，
//!     与 tauri-cli 的 kill-children.sh 同思路（不依赖进程组）。

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};

/// Windows `CREATE_NEW_PROCESS_GROUP`（0x00000200）：
/// 让子进程进入**新进程组**，脱离与控制台前台组共享的 CTRL_C。
///
/// 不设这个标志时，用户 Ctrl+C 会广播给控制台前台组的**所有**进程——
/// 包括我们经 `cmd /C` 拉起的 vite/backend 子进程，它们收到后各自退出，
/// 甚至 cmd 会弹 "Terminate batch job (Y/N)?"，清理时序完全失控。
/// 设了之后 Ctrl+C 只到 axctl 自己，由我们统一 `taskkill` 收尾。
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[cfg(unix)]
use std::time::Duration;

/// 一个受管的后台子进程。
pub struct ManagedChild {
    child: Child,
    /// 命令名（用于日志）。
    pub name: String,
}

/// 把命令字符串按 shell 风格分词（支持单双引号）。
///
/// 例如 `cargo run --bin "my server"` → `["cargo", "run", "--bin", "my server"]`。
/// 不做变量展开、管道等高级处理——够用即可，避免引入 shell 注入风险。
pub fn split_command(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for c in input.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        parts.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(c),
            },
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

impl ManagedChild {
    /// 后台启动一条命令，继承标准输出/错误（便于用户看到 vite/cargo 输出）。
    pub fn spawn(
        name: &str,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        envs: &[(&str, String)],
    ) -> Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in envs {
            cmd.env(k, v);
        }

        // Windows：子进程进新进程组，避免 Ctrl+C 被控制台广播给整棵
        // 子进程树（否则 vite 的 cmd 会弹 "Terminate batch job?"、
        // 清理时序失控）。Ctrl+C 只到 axctl，由 kill_tree 统一收尾。
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);

        let child = cmd.spawn().with_context(|| format!("failed to spawn {name} ({program})"))?;
        tracing::debug!(
            target: "axctl.process",
            name,
            program,
            "spawned child process"
        );
        Ok(Self { child, name: name.to_string() })
    }

    /// 按命令字符串后台启动（先分词再 spawn）。
    ///
    /// Windows 上包管理器等是 `.cmd` shim，直接 `Command::new` 找不到，
    /// 因此 Windows 分支统一经 `cmd /C` 执行完整命令字符串。
    pub fn spawn_command(
        name: &str,
        command: &str,
        cwd: Option<&Path>,
        envs: &[(&str, String)],
    ) -> Result<Self> {
        if cfg!(windows) {
            let wrapped = format!("cmd /C {command}");
            let parts = split_command(&wrapped);
            let (program, args) = parts
                .split_first()
                .with_context(|| format!("empty command for {name}"))?;
            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            return Self::spawn(name, program, &args_ref, cwd, envs);
        }
        let parts = split_command(command);
        let (program, args) = parts
            .split_first()
            .with_context(|| format!("empty command for {name}"))?;
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Self::spawn(name, program, &args_ref, cwd, envs)
    }

    /// 终止进程（含其派生的后代进程树）。
    pub async fn kill_tree(&mut self) {
        if self.child.id().is_none() {
            return;
        }
        let pid = self.child.id().expect("child id should be set while running");
        let result = kill_process_tree(pid).await;
        match result {
            Ok(()) => tracing::debug!(
                target: "axctl.process",
                pid,
                name = self.name,
                "child process tree terminated"
            ),
            Err(error) => tracing::warn!(
                target: "axctl.process",
                pid,
                name = self.name,
                error = %error,
                "failed to terminate child process"
            ),
        }
        // 等待进程真正退出，避免僵尸
        let _ = self.child.wait().await;
    }
}

/// 终止 pid 进程树。
///
/// Windows：taskkill /t（已能整树终止）。退出码 128 = 进程不存在
/// （已被外部杀掉 / 刚自己退出），视作成功而非错误。
/// Unix：递归收集后代，先 SIGTERM、宽限后 SIGKILL，最后杀根进程。
async fn kill_process_tree(pid: u32) -> std::result::Result<(), std::io::Error> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/t", "/f"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
        match status.code() {
            // 0 = 成功；128 = 进程不存在（taskkill 找不到目标）。
            // 后者在 Ctrl+C 竞态里常见（控制台已先杀掉部分子进程），
            // 目标已死正是想要的结果，不算失败。
            Some(0) | Some(128) => Ok(()),
            code => Err(std::io::Error::other(format!(
                "taskkill exited with {code:?}"
            ))),
        }
    }

    #[cfg(unix)]
    {
        // 1) 递归收集所有后代 pid
        let descendants = collect_descendants(pid);
        // 2) 先 SIGTERM 后代 + 根
        for p in descendants.iter().chain(std::iter::once(&pid)) {
            unsafe {
                libc::kill(*p as i32, libc::SIGTERM);
            }
        }
        // 3) 宽限轮询：所有进程是否已退出
        for _ in 0..20 {
            let mut alive = false;
            for p in descendants.iter().chain(std::iter::once(&pid)) {
                if process_exists(*p) {
                    alive = true;
                    break;
                }
            }
            if !alive {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // 4) 超时：SIGKILL 补刀
        for p in descendants.iter().chain(std::iter::once(&pid)) {
            unsafe {
                libc::kill(*p as i32, libc::SIGKILL);
            }
        }
        Ok(())
    }
}

/// Unix：递归收集 pid 的所有后代（含孙进程）。
#[cfg(unix)]
fn collect_descendants(pid: u32) -> Vec<u32> {
    let mut result = Vec::new();
    let mut stack = vec![pid];
    while let Some(current) = stack.pop() {
        if let Some(children) = child_pids(current) {
            for child in children {
                result.push(child);
                stack.push(child);
            }
        }
    }
    result
}

/// Unix：用 `pgrep -P <pid>` 找 pid 的直接子进程。
#[cfg(unix)]
fn child_pids(pid: u32) -> Option<Vec<u32>> {
    let output = std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(Vec::new()); // 无子进程（pgrep 退出码 1）
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect(),
    )
}

/// Unix：进程是否存在（kill 0 探测）。
#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn split_command_basic() {
        assert_eq!(
            split_command("cargo run --bin my-server"),
            vec!["cargo", "run", "--bin", "my-server"]
        );
    }

    #[test]
    fn split_command_with_quotes() {
        assert_eq!(
            split_command(r#"cargo run --bin "my server""#),
            vec!["cargo", "run", "--bin", "my server"]
        );
        assert_eq!(
            split_command("node 'a b.js' arg"),
            vec!["node", "a b.js", "arg"]
        );
    }

    #[test]
    fn split_command_empty_and_whitespace() {
        assert_eq!(split_command(""), Vec::<String>::new());
        assert_eq!(split_command("   "), Vec::<String>::new());
        assert_eq!(split_command("  cargo  run  "), vec!["cargo", "run"]);
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    /// 验证 collect_descendants 能收集多层后代。
    ///
    /// 起 `sh -c 'sleep 60 & sleep 60'`（sh 是根，两个 sleep 是后代），
    /// 用 /proc 检查 collect_descendants 结果包含两个 sleep。
    /// 默认跳过，设 AXCTL_KILL_TREE_TEST=1 启用。
    #[test]
    fn collect_descendants_finds_grandchildren() {
        if std::env::var("AXCTL_KILL_TREE_TEST").is_err() {
            return;
        }
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 60 & sleep 60")
            .spawn()
            .expect("spawn sh");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(300));

        let desc = collect_descendants(pid);
        assert_eq!(desc.len(), 2, "应找到两个 sleep 子进程, got {desc:?}");

        // 清理
        let _ = child.kill();
        let _ = child.wait();
    }
}
