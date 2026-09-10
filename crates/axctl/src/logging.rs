//! 统一日志管道。
//!
//! 本模块是 axctl 所有日志/用户可见输出的唯一出口：
//! - 代码只发 `tracing` 事件（宏或本模块提供的薄函数）
//! - 输出由注册在这里的自定义 layer 负责，写入 stderr
//! - stdout 保留给"数据输出"（`info`/`debug-ws` 的报告、子进程透传）
//!
//! # 两种展示模式
//!
//! 由 `RUST_LOG` 在启动时决定（运行期不变）：
//!
//! - **简洁模式**（默认，`RUST_LOG` 未设置或只有 info/warn/error）：
//!   与 tauri-cli 一致的风格，只显示分级前缀 + 消息：
//!
//!   ```text
//!           Info starting vite dev server...
//!           Done reverse proxy listening on http://127.0.0.1:3000
//!           Error backend restart failed: ...
//!   ```
//!
//! - **详细模式**（`RUST_LOG` 含 `debug` 或 `trace`）：
//!   展开 target 与全部结构化字段，便于诊断：
//!
//!   ```text
//!   INFO axctl: running backend: cargo run event=ui.info
//!   DEBUG axctl.dev: starting backend command="cargo run -p sealantern-server" backend_addr=127.0.0.1:3001
//!   DEBUG axctl.watcher: watching directory dir=D:\project\server\src
//!   ```
//!
//! # 前缀语义
//!
//! | level | event_name | 简洁模式前缀 |
//! | --- | --- | --- |
//! | ERROR | 任意 | `Error`（红） |
//! | WARN | 任意 | `Warn`（黄） |
//! | INFO | `ui.success` | `Done`（绿） |
//! | INFO | 其他 | `Info`（青） |
//! | DEBUG/TRACE | — | 简洁模式下不显示 |
//!
//! 公开 API：`info` / `success` / `warn` / `error` / `blank`（终端 UI 输出）、
//! `ui_message`（通用底层，带 level + event_name）、`init`（初始化管道）。
//!
//! 借鉴 SeaLantern `observability.rs` 的做法：每个子系统有固定 `target`，
//! 每个事件带 `event_name`，让日志可被 RUST_LOG 定向过滤（如 `RUST_LOG=axctl.proxy`）
//! 或按事件聚合。但 axctl 是 CLI，无需 SeaLantern 的事件键注册表——只取
//! target + event_name 的最小模式。

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use console::style;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// 各子系统 target 约定（RUST_LOG 定向过滤用，如 `RUST_LOG=axctl.proxy`）。
///
/// tracing 宏的 `target:` 参数要求编译期字符串字面量，故代码里直接写
/// 下列字符串值，不使用常量间接引用。本模块仅作文档与约定汇聚，
/// 故允许 dead_code。
///
/// - `axctl` —— 用户可见 UI 事件（dev 进度等），简洁模式的主通道
/// - `axctl.dev` —— dev 编排
/// - `axctl.proxy` —— 反向代理
/// - `axctl.process` —— 子进程生命周期
/// - `axctl.watcher` —— 文件监听
/// - `axctl.config` —— 配置读取
/// - `axctl.vite` —— vite 相关
#[allow(dead_code)]
pub mod target {
    /// 用户可见 UI 事件（dev 进度等）。
    pub const UI: &str = "axctl";
    /// dev 编排。
    pub const DEV: &str = "axctl.dev";
    /// 反向代理。
    pub const PROXY: &str = "axctl.proxy";
    /// 子进程生命周期。
    pub const PROCESS: &str = "axctl.process";
    /// 文件监听。
    pub const WATCHER: &str = "axctl.watcher";
    /// 配置读取。
    pub const CONFIG: &str = "axctl.config";
    /// vite 相关。
    pub const VITE: &str = "axctl.vite";
}

/// 详细模式开关（init 后只读）。
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// `ui.success` 事件名：简洁模式下渲染成绿色 `Done` 前缀。
const EVENT_UI_SUCCESS: &str = "ui.success";

/// 初始化全局日志管道。必须在任何输出之前调用一次（main 最前）。
pub fn init() {
    VERBOSE.store(env_is_verbose(), Ordering::SeqCst);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(AxctlLayer)
        .init();
}

/// RUST_LOG 是否要求详细模式（含 debug 或 trace）。
fn env_is_verbose() -> bool {
    std::env::var("RUST_LOG")
        .map(|v| {
            let v = v.to_ascii_lowercase();
            // 注意：`axctl=debug` 与 `debug` 都命中；`info` 不命中。
            v.contains("debug") || v.contains("trace")
        })
        .unwrap_or(false)
}

/// 发一条用户可见 UI 事件（简洁模式显示为 tauri 前缀风格）。
///
/// `event_name` 用于区分 `Done` 前缀等样式；`info` / `success` 等语义函数
/// 是对本函数的封装。
pub fn ui_message(level: Level, event_name: &str, message: &str) {
    // tracing 宏的 target 需要编译期字符串，故直接写常量值 `"axctl"`。
    match level {
        Level::ERROR => tracing::event!(target: "axctl", Level::ERROR, event_name, message),
        Level::WARN => tracing::event!(target: "axctl", Level::WARN, event_name, message),
        Level::INFO => tracing::event!(target: "axctl", Level::INFO, event_name, message),
        Level::DEBUG => tracing::event!(target: "axctl", Level::DEBUG, event_name, message),
        Level::TRACE => tracing::event!(target: "axctl", Level::TRACE, event_name, message),
    }
}

/// 打印 info 级日志：`Info <message>`（tauri 风格，青色前缀）。
pub fn info(message: impl AsRef<str>) {
    ui_message(Level::INFO, "ui.info", message.as_ref());
}

/// 打印成功提示：`Done <message>`（绿色前缀）。
pub fn success(message: impl AsRef<str>) {
    ui_message(Level::INFO, "ui.success", message.as_ref());
}

/// 打印警告：`Warn <message>`（黄色前缀）。
pub fn warn(message: impl AsRef<str>) {
    ui_message(Level::WARN, "ui.warn", message.as_ref());
}

/// 打印错误：`Error <message>`（红色前缀）。
pub fn error(message: impl AsRef<str>) {
    ui_message(Level::ERROR, "ui.error", message.as_ref());
}

/// 输出分隔空行。
///
/// 空行不是日志事件，仅在人类可读终端输出（重定向到文件时保持干净）。
pub fn blank() {
    if console::user_attended_stderr() {
        eprintln!();
    }
}

/// 格式化的 stderr 层。
struct AxctlLayer;

impl<S: Subscriber> Layer<S> for AxctlLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let verbose = VERBOSE.load(Ordering::Relaxed);
        let level = *event.metadata().level();

        // 收集字段：message 与其余 key=value 分开
        let mut fields = Fields::default();
        event.record(&mut fields);

        if verbose {
            // 详细：LEVEL target: message key=value ...
            let level_word = match level {
                Level::ERROR => style("ERROR").red().bold(),
                Level::WARN => style("WARN").yellow().bold(),
                Level::INFO => style("INFO").cyan().bold(),
                Level::DEBUG => style("DEBUG").dim(),
                Level::TRACE => style("TRACE").dim(),
            };
            let mut kv: Vec<String> = fields.kv.iter().map(|(k, v)| format!("{k}={v}")).collect();
            if let Some(name) = &fields.event_name {
                kv.push(format!("event={name}"));
            }
            let mut line =
                format!("{level_word} {}: {}", event.metadata().target(), fields.message);
            if !kv.is_empty() {
                line.push(' ');
                line.push_str(&kv.join(" "));
            }
            eprintln!("{line}");
            return;
        }

        // 简洁：DEBUG/TRACE 静默（EnvFilter 默认已挡，这里双保险）
        if matches!(level, Level::DEBUG | Level::TRACE) {
            return;
        }

        let is_success = fields.event_name.as_deref() == Some(EVENT_UI_SUCCESS);
        let (word, colored) = match level {
            Level::ERROR => ("Error", style("Error").red().bold()),
            Level::WARN => ("Warn", style("Warn").yellow().bold()),
            Level::INFO if is_success => ("Done", style("Done").green().bold()),
            Level::INFO => ("Info", style("Info").cyan().bold()),
            _ => unreachable!(),
        };
        let _ = word; // colored 已含文本，word 仅注释用途
        eprintln!("        {} {}", colored, fields.message);
    }
}

/// 从 event 收集 message / event_name / 其余 kv 字段。
#[derive(Default)]
struct Fields {
    message: String,
    event_name: Option<String>,
    kv: Vec<(String, String)>,
}

impl Fields {
    fn push_kv(&mut self, name: &str, value: String) {
        self.kv.push((name.to_string(), value));
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        match field.name() {
            "message" => self.message = rendered.trim_matches('"').to_string(),
            "event_name" => self.event_name = Some(rendered.trim_matches('"').to_string()),
            name => self.push_kv(name, rendered),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "event_name" => self.event_name = Some(value.to_string()),
            name => self.push_kv(name, format!("{value:?}")),
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push_kv(field.name(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push_kv(field.name(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push_kv(field.name(), value.to_string());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push_kv(field.name(), value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env_is_verbose 对典型 RUST_LOG 的判断。
    #[test]
    fn verbose_detection() {
        // 用临时设置环境变量并恢复
        let set = |val: Option<&str>| match val {
            Some(v) => unsafe { std::env::set_var("RUST_LOG", v) },
            None => unsafe { std::env::remove_var("RUST_LOG") },
        };

        let orig = std::env::var("RUST_LOG").ok();

        set(None);
        assert!(!env_is_verbose());
        set(Some("info"));
        assert!(!env_is_verbose());
        set(Some("warn,axctl=debug"));
        assert!(env_is_verbose());
        set(Some("trace"));
        assert!(env_is_verbose());
        set(Some("axctl.proxy=debug,info"));
        assert!(env_is_verbose());

        // 恢复
        set(orig.as_deref());
    }
}
