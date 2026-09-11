# 05 · 设计要点

记录 axctl 的关键决策与踩坑。每条都对应一个"看起来能用、实际有问题"的朴素做法。

## HMR 必须走 WebSocket 隧道

**现象**：vite 配置未固定 `hmr` 端口时，HMR client 使用**页面加载端口**（即
代理端口 3000）建立 WebSocket。若代理不做 WS 转发，HMR 断裂，前端改动触发整页刷新。

**关键细节**（曾漏掉导致握手失败）：

1. **子协议回选**：vite HMR server 只接受带 `Sec-WebSocket-Protocol: vite-hmr`
   的升级请求。RFC 6455 要求：客户端带了子协议，服务器响应必须回选其一，否则
   浏览器判定握手失败。因此代理在浏览器↔代理段需 `requested_protocols()` +
   `protocols()` 让 axum 回选，在代理↔vite 段需带上子协议。
2. **path + query 透传**：vite 用 `?token=` 鉴权，代理连 vite 时必须保留原始
   path 与 query，否则鉴权失败。

结论：WS 隧道是硬需求，且必须完整透传子协议与 path/query。

## 哨兵机制（前端新鲜度）

**现象**：cargo 增量编译感知不到 `dist/` 变化，`frontend!` 宏读 dist 后 cargo
认为"源码没变"不重编 → 内嵌旧产物。

**解法**：`axctl build` 在 `vite build` 后、`cargo release` 前写
`dist/.axctl-sentinel`，内容为 **dist 指纹**（文件名 + 大小 + mtime 的 hash，
每次构建必变）。用户 server 的 `build.rs` 声明契约：

```rust
fn main() {
    println!("cargo:rerun-if-changed=dist/.axctl-sentinel");
    println!("cargo:rerun-if-changed=dist"); // 可选兜底
}
```

哨兵变 → `build.rs` 重跑 → 主 crate 重编 → `frontend!` 重读 dist。

> 不用"内容相同则不写"的策略：哨兵职责是让 cargo 感知"发生了构建"，若内容
> 相同就不写，cargo 可能跳过重编。指纹含 mtime 保证每次必变。

## 进程模型：先 build 后 spawn

**为什么不用 `cargo run`**：cargo 是父进程、server 是其子进程。杀掉 cargo 后
server 变孤儿继续占端口，下次重启 bind 失败；且编译期间旧进程已杀、新进程未起，
API 完全不可用。

**解法**：`cargo build -p <pkg> --bin <bin>` 编译到成功 → spawn 编译产物
（`target/debug/<bin>`，**叶子进程**）→ 终止时 kill 根进程即干净。

**重启语义（平台差异）**：

- Unix：先编译成功，再杀旧进程——编译期旧后端继续服务，编译失败旧进程保留。
- Windows：先杀旧进程再编译——Windows 锁定运行中的 exe，不杀则 cargo 无法覆盖
  产物（链接失败）。因此 Windows 放弃"编译期 API 可用"。

**进程树终止**：Unix 用 `pgrep -P` 递归收集后代再 `SIGTERM`/`SIGKILL`；
Windows 用 `taskkill /t /f`。backend 虽是叶子，但 vite（pnpm → node）是树，需杀树。

## 监听范围：workspace 感知

**为什么不能监听整个项目根**：

- Linux `inotify` 每目录一个 watch descriptor，递归注册整个根会连
  `node_modules` / `target` / `.git` 一起带上，撞穿 `fs.inotify.max_user_watches`
  默认上限（8192）→ watch 失败或静默丢事件。
- 语义错误：改 `src-tauri`（另一个二进制）不该重启 backend。

**解法**：`watch_set.rs` 计算 backend 的 **path dependency 闭包**（BFS，只走
workspace 内的 path 依赖），只注册闭包内每个 member 的 `src/`（递归，按 Rust
源码过滤）、`Cargo.toml`、`build.rs`。`extra_watch_dirs` 作为 **trigger-all
目录**（其中任何文件变化都触发，不过滤 `.rs`）。

依赖关系取自 `cargo metadata` 的 **resolve.nodes**（已解析的依赖图），而非
Cargo.toml 声明——这样 rename 依赖能正确解析，且只统计 normal + build 依赖，
排除 dev-dependencies。

## 内嵌方案：include_dir 而非 rust-embed

`axctl-core` 的 `frontend!` 宏底层用 `include_dir`。选它而非 `rust-embed` 的
原因：需要的是 **`let assets = macro!(...)` 表达式值**（可传给 serve 层）；
rust-embed 是 derive 形态，资源挂在类型上只能静态调用，无法作值传递。

约束：`include_dir!` 展开引用裸 `include_dir::`，所以**调用方 crate 必须直接
依赖 `include_dir = "0.7"`**。

release 内嵌、debug 空包装——debug 下不内嵌、不读盘，dev 前端由 vite 服务。

## SPA 回退：扩展名启发式

`spa()` 的回退判定用**路径扩展名**（无扩展名 = SPA 客户端路由 → 回退
`index.html`；带扩展名未命中 = 真实资源缺失 → 404），而非 `Accept` 头。

理由：curl / 测试工具默认 `Accept: */*` 也能命中 SPA 路由；带扩展名的缺失
`fetch()` 仍 404，不会拿到 HTML。

**已知边界（勿当 bug 报）**：带扩展名的**合法 SPA 路由**会被当缺失资源返回 404
（如 `/order/2024.12`）。含点号的客户端路由需自行处理：改成无点号路径，或在业务
router 里显式挂路由 / 白名单。

缓存头分层：HTML `no-store`；`assets/` 下 1 年 `immutable`（Vite 内容 hash）；
其余 `public, no-cache`；ETag（内容 hash）+ `If-None-Match` → 304。

## 统一日志管道

- tracing 是**唯一通道**；默认显示与 tauri-cli 一致的最简风格，`RUST_LOG` 展开细节。
- stdout 留给数据输出；日志一律写 **stderr**。
- 两种模式：默认（未设 / `info` / `warn`）为简洁模式（`Info` / `Done` / `Warn` /
  `Error` 前缀）；含 `debug` / `trace` 为详细模式（`LEVEL target: message key=value`）。
- target 约定：`axctl`（UI）/ `axctl.dev` / `axctl.proxy` / `axctl.process` /
  `axctl.watcher` / `axctl.config`。可定向过滤：`RUST_LOG=axctl.proxy=debug`。
- level → 前缀：ERROR→`Error`、WARN→`Warn`、INFO+`ui.success`→`Done`、INFO→`Info`。

## 已知边界与限制

- **SPA 扩展名启发式**：含点号的合法客户端路由返回 404（见上）。
- **Windows 无法做到"编译期 API 可用"**：exe 文件锁决定必须先杀后编。
- **serve 需本地 vite**：项目 `node_modules` 就绪，否则报错。
- **`init` 未实现**。
