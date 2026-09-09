# axctl 设计文档

面向 axum + Vite 项目的一体化开发工具（对标 tauri-cli 的开发体验）。
本文档记录架构决策、模块边界与关键实现方案。

## 1. 定位与目标

- 为 axum + Vite 项目提供 `dev` / `build` / `serve` / `package` / `info` 等一体化命令。
- 对标 tauri-cli：一条命令拉起完整开发环境，产物打包成单文件分发。
- 通用工具，不绑定具体业务；以 Cargo.toml metadata（`[workspace.metadata.axctl]`
  优先，`[package.metadata.axctl]` 覆盖）为唯一配置入口。

## 2. 语言边界约定

| 范畴 | 语言 | 说明 |
| --- | --- | --- |
| 用户可见 | 英文 | CLI 输出、`--help`、README、docs.rs 文档注释、发布说明 |
| 开发内部 | 中文 | 源码内部注释、配置文件注释、git 提交信息、本文档 |

> 注：`crates/axctl`（二进制）的 `//!` / `///` 不进 docs.rs，统一中文；
> `crates/axctl-core`（库）的公开文档注释用英文。
> tracing 日志属于 CLI 运行时输出，归"用户可见"，用英文。

## 3. 总体架构：workspace 双 crate

```
axctl/                     # workspace 根（虚拟 manifest）
├── Cargo.toml             # [workspace] members = ["crates/axctl", "crates/axctl-core"]
├── crates/
│   ├── axctl/             # CLI：clap 定义 + 命令编排
│   │   └── src/
│   │       ├── main.rs          # clap 入口、dispatch、debug-ws 调试子命令（隐藏）
│   │       ├── logging.rs       # 统一日志管道（tracing layer，两种展示模式）
│   │       ├── backend.rs       # backend 解析/编译/spawn（先 build 后跑产物）
│   │       ├── config.rs        # 双层配置读取（workspace + package merge）
│   │       ├── workspace.rs     # cargo_metadata → WorkspaceInfo / MemberInfo
│   │       ├── watch_set.rs     # WorkspaceInfo + 配置 → WatchSet
│   │       ├── watcher.rs       # notify 监听（只注册 WatchSet 精确路径）
│   │       ├── process.rs       # 跨平台子进程管理（进程树终止）
│   │       ├── proxy.rs         # 反向代理（HTTP 转发 + WS 隧道）
│   │       ├── vite.rs          # vite 探测 / 启动（dev）/ 预览（serve）/ 包管理器选择
│   │       └── commands/
│   │           ├── info.rs      # 已实现
│   │           ├── dev.rs       # 已实现
│   │           ├── build.rs     # 已实现（vite build + 哨兵 + cargo release）
│   │           ├── serve.rs     # 已实现（封装 vite preview）
│   │           └── package.rs   # 已实现（封装 cargo-packager）
│   └── axctl-core/        # 库：给用户项目直接依赖，负责内嵌/服务/探测
│       ├── embed.rs       #   前端内嵌（include_dir + frontend! 宏 + FrontendAssets）——已实现
│       ├── serve.rs       #   SPA fallback 路由（spa()/缓存头/ETag）——已实现
│       └── probe.rs       #   环境探测（已实现）
└── docs/design.md         # 本文档
```

### 3.1 为什么拆成双 crate

"把 web 构建产物内嵌进 server 二进制"的能力必须在**用户项目**里编译，
所以它天然是库 crate，而不是 CLI 逻辑。CLI 只负责编排命令，
内嵌 / 服务 / 探测等能力由 `axctl-core` 以库形式提供给用户项目。

### 3.2 模块依赖方向（CLI 内部）

```
dev.rs ──► workspace.rs ──► config.rs（读取 metadata）
   │            │
   │            └──────► watch_set.rs ──► watcher.rs
   ├──► vite.rs / process.rs / proxy.rs / logging.rs
```

`workspace.rs` 与 `watch_set.rs` 是纯数据模块（解析 + 计算），无 IO 副作用，
可独立单元测试；`dev.rs` 只做编排。

## 4. 子命令设计

| 命令 | 职责 | 状态 |
| --- | --- | --- |
| `init` | 初始化项目（探测环境、生成配置） | 未实现 |
| `dev` | 开发模式（见 §5） | 已实现 |
| `build` | 生产构建（见 §6） | 已实现 |
| `serve` | 静态预览：封装 vite preview（纯前端，无后端） | 已实现（§11） |
| `package` | 打包：对接 cargo-packager（见 §7） | 已实现 |
| `info` | 环境诊断：输出 Rust / 前端 / 系统信息 | 已实现 |
| `debug-ws` | 打印 workspace 解析结果与 WatchSet（隐藏调试命令） | 已实现 |

## 5. dev 模式：统一入口反向代理

### 5.1 目标

用户访问入口统一为 **3000**，dev 与 release 心智一致；对用户 vite.config 零侵入。

### 5.2 架构

```
用户访问 http://127.0.0.1:3000
        │
        ▼
  axctl 代理 (3000)  ← axctl dev 启动
  ├── /api/* ─────────► 后端 server (dev 时监听 3001)
  ├── 其他 ───────────► vite dev server (5173，前端 + HMR)
  └── WebSocket ──────► vite (HMR 隧道)
```

### 5.3 端口规划

| 端口 | 用途 | 说明 |
| --- | --- | --- |
| 3000 | axctl 代理（用户唯一入口） | release 时后端直接监听 3000，入口不变 |
| 5173 | vite dev server | 前端 + HMR |
| 3001 | 后端 dev 监听 | 必须让开 3000（被代理占用），可配置 |

### 5.4 代理实现要点

| 能力 | 实现方式 |
| --- | --- |
| HTTP 转发 | axum handler 接请求，reqwest 转发（**共享 Client**，连接复用），body 流式转发（不整体读入内存） |
| 重定向透传 | reqwest `redirect(Policy::none())`——30x 原样透传，`Location` 头保留（登录跳转 / OAuth / 表单重定向必需）；reqwest 跟随会丢 Location、POST 跟随 302 被改 GET |
| hop-by-hop 头 | 请求侧过滤 `connection`/`keep-alive`/`upgrade`/`te`/`trailer`/`transfer-encoding` 等；响应侧过滤 `connection`/`keep-alive`/`transfer-encoding`/`content-length`（流式 body 由 axum 重算） |
| WebSocket 隧道 | axum `WebSocketUpgrade` 接浏览器，`tokio-tungstenite` 连 vite，双向帧转发 |
| 路径分流 | `/api` 前缀 → 后端；其余（含 `/@vite/`、`/node_modules/` 等）→ vite |

### 5.5 HMR 必须走隧道（关键坑）

用户的 vite 配置 `hmr` 未固定端口时，HMR client 会使用**页面加载端口**
（即 3000）连 WebSocket。因此 axctl 代理**必须**做 WS 隧道，否则 HMR 断裂、
前端改动触发整页刷新。WS 隧道是硬需求，不是可选项。

### 5.6 dev 完整链路

```
axctl dev
├─ 0. 跑一次 cargo metadata → WorkspaceInfo + 双层配置 + backend target
├─ 1. 前端：frontend_dev_url 配置了 → 探测复用该地址（用户自管前端，
│      不拉起；可等 60s）；未配置 → 探测 5173，未就绪则由 axctl 拉起
├─ 2. 编译并启动后端：cargo build -p <pkg> --bin <bin> → spawn 产物
├─ 3. 起反向代理 (3000)：/api → 3001，其余 → 5173，WS 隧道
├─ 4. 计算 WatchSet（backend 依赖闭包 + extra dirs）并监听
└─ 5. 后端依赖源码变化 → 重编译重启（代理和 vite 全程不动）
```

### 5.7 与 tauri 的对比

tauri 前端经 webview IPC 桥访问后端，不走 HTTP，因此无代理需求，对 vite.config
零侵入、只认 `devUrl`。axctl 的前端在浏览器里必须走 HTTP，因此用反向代理方案
统一入口，同样对 vite.config 零侵入，且兼容 tauri 项目的 `devUrl` 心智。

### 5.8 监听范围：workspace 感知（关键架构决策）

**为什么不能监听整个项目根**：

- Linux inotify 每目录一个 watch descriptor，递归监听整个 workspace 根会连
  `node_modules` / `target` / `.git` 一起注册，撞穿 `fs.inotify.max_user_watches`
  默认上限（8192）→ watch 失败或静默丢事件。
- 事件层过滤来不及：notify 6.x 不支持注册时排除子目录，watch descriptor 已建立。
- 语义错误：改 `src-tauri`（另一个二进制）不该重启 `sealantern-server`。

**解法**（已实现）：

1. `workspace.rs` 用 `cargo_metadata` 解析出所有 member 及各自的
   binary target、path 依赖。依赖关系取自 **resolve.nodes**（已解析的依赖图），
   而非 Cargo.toml 的声明——rename 依赖（`foo = { path, package = "bar" }`）
   能正确解析到 bar；且只统计 normal + build 依赖，**排除 dev-dependencies**
   （构建 backend 不需要 dev-dep 的源码）。
2. 确定 backend member（优先级见 §7.4）。
3. `watch_set.rs` 计算 backend 的 **path dependency 闭包**（BFS，只走
   workspace 内的 path 依赖）。
4. `watcher.rs` 只注册闭包内每个 member 的：
   - `src/`（递归，按 Rust 源码过滤触发）
   - `Cargo.toml`（单文件，监听父目录 NonRecursive）
   - `build.rs`（若存在）
5. `extra_watch_dirs`（配置）注册为 **trigger-all 目录**：其中**任何文件变化
   都触发重启**（模板 / 迁移等资源目录），不过滤 .rs——与 member src/ 的
   Rust 源码过滤语义区分。
6. 事件层仍保留忽略兜底：`target` / `node_modules` / `.git` / `dist` / `.axctl`
   （`ignored_dir_names`）与 `target_dir` 前缀（`auto_ignored`）。

**事件投递模型**：notify debouncer 回调运行在自己的线程（无 Tokio reactor），
且不应直接在里面 spawn 重启任务（会产生并发堆积、顺序不可控）。改为：
notify 回调只把 `ChangeBatch` 经 `std::sync::mpsc` 发给 watcher 线程，
`on_change` 在 watcher 线程同步执行，由调用方决定是否/如何提交 Tokio 任务。

### 5.9 后端生命周期：先编译后 spawn（对齐 tauri-cli）

**为什么不用 `cargo run`**：

- `cargo run` 下 cargo 是父进程、server 二进制是 cargo 的子进程。杀掉 cargo
  后 server 变孤儿继续占用端口，下次重启 bind 失败、永远起不来。
- 编译期间 API 完全不可用（旧进程已杀、新进程没起）。

**解法**（已实现，模块 `backend.rs`）：

1. `resolve_backend`：解析 backend 的 member + binary target（4 级优先级见 §7.4）。
2. `cargo_build`：`cargo build -p <pkg> --bin <bin>` 编译到成功。
3. `spawn` 编译产物（`<target_dir>/debug/<bin>(.exe)`）——**叶子进程**，
   终止时 kill 根进程即干净，无孤儿。

**重启语义**（平台差异）：

- Unix：先编译，成功后才 kill 旧进程——编译期间旧后端继续服务（API 不断），
  编译失败旧进程保留。
- Windows：必须先 kill 旧进程再编译——Windows 锁定正在运行的 exe 文件，
  旧进程不退出则 cargo 无法覆盖产物（链接失败）。Windows 因此放弃
  "编译期 API 可用"（平台限制，做不到）。

**进程树终止**（对齐 tauri-cli kill-children.sh）：Unix 用 `pgrep -P <pid>`
递归收集全部后代再 SIGTERM/SIGKILL；Windows 保留 `taskkill /t /f`。backend
虽是叶子，但 vite（pnpm → node）仍是树，axctl 拉起的 vite 需要杀树。

**重启防抖**（三态状态机）：300ms 去抖只合并写入风暴，挡不住编译窗口
（编译 30s 内又改一处会被旧版丢弃）。dev.rs 用 `AtomicU8` 三态
`idle / restarting / pending`：restart 进行中收到新变更置 `pending`，
当前轮结束后若 pending 置位则再补一轮；Ctrl+C 清理直接结束、忽略 pending。
重启失败回到 `idle`，允许下次变更再试。

### 5.10 监听安全提示

`proxy_addr` 配成非 loopback（如 `0.0.0.0`）时打印一条 warn——会把无鉴权的
后端与 vite dev server 暴露到网络（vite 的 @fs 与 HMR 有历史 RCE，
CVE-2025-30221 等），仅受信的本机开发环境可用。

## 6. build 模式：内嵌 web 产物

### 6.1 链路（全部已实现）

```
axctl build
├─ 1. 定位前端根 + 校验 package.json         ✅
├─ 2. vite build（pnpm/npm exec）→ dist/      ✅
├─ 3. 写入 dist/.axctl-sentinel（dist 指纹）  ✅ 哨兵（§6.3）
└─ 4. cargo build --release -p <backend>      ✅
       ├─ build.rs rerun-if-changed 命中 → 重编（已实证）
       └─ frontend! 宏展开 → 新 dist 烙进二进制 ✅（§6.2/§6.4）
```

实测：minimal-app 上 `axctl build` 一次跑通（vite build 37ms → 哨兵写入 →
release 编译 17s → 产物 exe）；哨兵指纹每次构建变化（含 mtime）。

### 6.2 内嵌方案：include_dir + frontend! 宏（已实现，axctl-core::embed）

用户 server crate 里用 `frontend!` 宏绑定一个 `FrontendAssets` 包装值：

```rust
use axctl_core::embed::FrontendAssets;

// release：编译期内嵌 "$CARGO_MANIFEST_DIR/../dist"；debug：空包装
let assets: FrontendAssets<'static> =
    axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");
// assets.get_file("index.html") → Option<&File>（递归按路径查）
// assets.files()               → 递归迭代所有文件
// assets.is_embedded()         → release true / debug false
```

- **release**：`include_dir!` proc-macro 编译期读目录烙进二进制 → 单文件
  分发，运行时不依赖磁盘 dist。
- **debug**：`frontend!` 展开为 `FrontendAssets::empty`，**不内嵌、不读盘**
  ——dev 前端由 vite(5173) 服务，serve 层只在 release 挂载（§6.4）。
- 依赖约束：`include_dir!` 展开引用裸 `include_dir::`，调用方须直接依赖
  `include_dir = "0.7"`（与 axum-vite `embedded_dir!` 同款要求）。
- 形态取舍：选 include_dir 而非 rust-embed，因为要的是 **`let assets =
  macro!(...)` 表达式值**（能传给 serve 层）；rust-embed 是 derive 形态，
  资源挂在类型上只能静态调用，无法作值传递。
- 验证：`tests/fixtures/embed-app/`（独立 crate，static/ 版本化假前端）。
  实测 release 内嵌后删 static/ 目录二进制仍能读到文件；debug 输出
  "not embedded"。

### 6.3 哨兵机制（已实现，axctl build 写入）

**坑**：cargo 增量编译感知不到 dist 目录变化，`frontend!` 宏读 dist 后 cargo
认为"源码没变"不重编 → 内嵌旧产物。

**解法**（已实现）：
- `axctl build` 在 vite build 后、cargo release 前，写 `dist/.axctl-sentinel`，
  内容 = **dist 指纹**（文件名+大小+mtime 的 hash，每次构建必变——
  已实测同一产物重复 build 指纹仍变化）。
- 用户 server 的 build.rs 声明契约：
  ```rust
  fn main() {
      println!("cargo:rerun-if-changed=../dist/.axctl-sentinel");
      println!("cargo:rerun-if-changed=../dist"); // 可选兜底
  }
  ```
- **传导实证**（reruntest）：哨兵内容变 → build.rs 重跑 → 主 crate 重编
  （`BUILD_STAMP` 变化 + "Compiling" 出现）。无需 OUT_DIR 桥接——
  build.rs rerun 命中即可传导。

**为何不用 tauri 的 `write_if_changed`（内容相同不写）**：哨兵职责是"每次
构建都让 cargo 感知发生了构建"，若产物相同就不写，cargo 可能跳过重编。
指纹含 mtime 保证每次必变，天然规避该问题。

### 6.4 运行时服务（已实现，axctl-core::serve::spa）

用户 server 用 `fallback_service` 挂载（Router 组合语义，非 fallback）:

```rust
use axctl_core::frontend;

let assets = frontend!("$CARGO_MANIFEST_DIR/../dist");
let app = Router::new()
    .nest("/api", api_routes)
    .fallback_service(axctl_core::serve::spa(assets));
```

`spa` 返回 `Router<S>`（state 泛型，handler 不 extract state）——可并入任意
带 state 的用户 router。

**行为矩阵**（release / 内嵌时）：

| 请求 | 行为 |
|---|---|
| `GET /`（根） | 无条件回 index.html（200, no-store） |
| `GET /assets/xxx.js`（命中） | 200 + `immutable` 缓存 + mime + ETag |
| `GET /some/route`（无扩展名未命中） | SPA 客户端路由 → 回 index.html（200） |
| `GET /missing.json`（带扩展名未命中） | 真 404（fetch 缺失资源绝不返回 HTML） |
| debug（frontend! 空包装） | 全 404（dev 前端由 vite 服务，不读盘） |

缓存头分层：HTML `no-store` / `assets/` 下 1 年 immutable（Vite 内容 hash）
/ 其余 `public, no-cache`；ETag（内容 hash）+ If-None-Match → 304。

SPA 回退判定用**扩展名启发式**（无扩展名路径 = 客户端路由）而非 Accept 头
——curl / 测试工具默认 `Accept: */*` 也能命中 SPA 路由；带扩展名的缺失
fetch 仍 404。参考 axum-vite（§6.4 设计来源），但去掉其 dev proxy / HMR /
manifest 等 vite 专属逻辑。

**已知边界（勿当 bug 报）**：扩展名启发式是双刃——带扩展名的**合法 SPA
路由**会被当缺失资源返回 404（如 `/order/2024.12`、`/user/v1.2/profile`）。
纯客户端路由若路径含点号，需自行处理：改成无点号路径，或在业务 router
里对这类路径显式挂路由 / 白名单。

## 7. 配置设计

### 7.1 双层配置入口

**为什么必须支持 workspace 层**：真实项目（如 SeaLantern）根 `Cargo.toml` 是
虚拟 manifest（只有 `[workspace]`，无 `[package]`），若只认
`[package.metadata.axctl]` 则配置完全失效。因此：

```toml
# workspace 根（虚拟 manifest 项目的主配置）
[workspace.metadata.axctl.dev]
backend_package = "sealantern-server"
backend_command = "cargo run -p sealantern-server"
backend_port = 3001
proxy_addr = "127.0.0.1:3000"
frontend_dev_url = "http://127.0.0.1:5173"
frontend_root = "."

# 某个 member 内的可选覆盖（如 server/Cargo.toml）
[package.metadata.axctl.dev]
backend_port = 3999        # 只覆盖这一个字段，其余仍取 workspace 层
```

**优先级**（从高到低，低层缺失字段由高层补齐）：

```
package.metadata.axctl.dev   （当前 member 的 dev 嵌套段）
package.metadata.axctl       （当前 member 的扁平字段）
workspace.metadata.axctl.dev （workspace 根的 dev 嵌套段）
workspace.metadata.axctl     （workspace 根的扁平字段）
默认值
```

读取时用 `cargo metadata` 解析（避免手撸 Cargo.toml 格式），先读 workspace 层，
再找"包含 cwd 的 member"（`find_current_package`），读其 package 层，
后者字段级覆盖前者（`merge_configs`）。

### 7.2 字段定义

所有字段扁平铺在 `AxctlConfig`，`.dev` / `.build` 嵌套段读取时被合并进来：

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `backend_port` | `u16?` | 3001 | dev 后端监听端口（让开代理） |
| `frontend_dev_url` | `string?` | 无 | 已运行的 vite 地址（对应 tauri devUrl）；配置了则复用 |
| `proxy_addr` | `string?` | 127.0.0.1:3000 | 代理监听地址 |
| `frontend_root` | `string?` | workspace 根 | 前端目录（相对 workspace 根） |
| `frontend_build_command` | `string?` | pnpm/npm 探测 | 前端构建命令 |
| `backend_command` | `string?` | 见 §7.3 | 兼容写法：从中解析 `-p`/`--bin` 定位 backend member |
| `backend_package` | `string?` | 见 §7.4 | 后端 package 名（workspace 多 binary 时必填） |
| `extra_watch_dirs` | `string[]` | [] | 额外监听目录（相对 workspace 根），**任何变化都触发重启**（见 §5.8）；两层配置**拼接去重** |

### 7.3 后端启动（backend_command / backend_package）

后端启动策略是"先 `cargo build -p <pkg> --bin <bin>`、再 spawn 编译产物"
（§5.9），**不直接执行 backend_command**。`backend_command` 保留为**兼容
写法**——仅用于解析出 `-p <pkg>` / `--bin <bin>` 以定位 backend member；
也兼容老用户 `cargo run -p sealantern-server` 的直觉（解析出 server 包）。

- 监听地址经环境变量 `AXCTL_BACKEND_ADDR` 传入后端（后端读它决定绑定地址）。
- 未配置 `backend_command` 时：配了 `backend_package` → build 该包；否则
  走 §7.4 的自动解析。
- 真正的二进制启动由 `backend.rs` 管理：`cargo_build` + `binary_path` +
  `spawn`，产物是叶子进程。

### 7.4 backend member 解析优先级

确定"谁是需要监听/重启的后端"：

1. `backend_package` 配置
2. `backend_command` 里的 `-p` / `--package`（或 `--package=`）
3. `backend_command` 里的 `--bin`（经 `package_for_bin` 反查）
4. cwd 所在 member（`member_for_dir`）
5. workspace 内唯一的 binary package
6. 都失败 → 报错，提示配置 `backend_package` 或 `backend_command`

## 8. 依赖清单

### crates/axctl（CLI）

| 依赖 | 用途 |
| --- | --- |
| clap | 参数解析 |
| anyhow | 错误处理 |
| tokio | 异步运行时 |
| tracing / tracing-subscriber | 统一日志管道（logging.rs，见 §10） |
| serde / toml | 配置反序列化 |
| cargo_metadata | workspace 结构解析（root / members / targets / path 依赖） |
| notify-debouncer-full | 文件监听（防事件风暴） |
| reqwest | 代理 HTTP 转发 |
| tokio-tungstenite | 代理 WS 隧道 |
| axum | 代理服务器框架（features = ["ws"]） |
| console | 终端着色（logging layer） |

### crates/axctl-core（库）

| 依赖 | 用途 |
| --- | --- |
| include_dir | 前端目录编译期内嵌（frontend! 宏底层） |
| axum | 静态服务 / SPA fallback |
| mime_guess | mime 推断 |

## 9. 待办 / 已知问题（按优先级）

### 当前实现缺陷

- **P1-4**：`proxy.rs` 混杂 HTTP 转发 / WS 隧道 / 消息转换，
  建议拆成 `proxy/{mod,http,ws,headers,target}.rs`。
- **P1-6**：`dev.rs` 编排逻辑过厚（进程生命周期 / 重启状态 / 清理混在一起），
  建议抽 `DevSession`。
- **P1-7**：~~WebSocket / HMR 隧道从未实测~~；已修复：`tunnel_ws` 透传
  path/query + 子协议（ClientRequestBuilder），`vite_handler` 用
  `requested_protocols`/`protocols` 回选子协议（RFC 6455），新增单测
  `proxy_ws_tunnel_preserves_protocol_and_path`。
- **P2**：`ManagedChild` 无 `Drop`，异常路径依赖显式 kill_tree，易漏（vite
  启动后 backend 失败等场景会残留子进程）。
- **P2**：`vite.rs::package_manager()` 硬编码 pnpm > npm，未读配置的
  `frontend_build_command` / package.json `packageManager` 字段。
- **P2**：编译失败时旧后端保留（Windows 因 exe 锁须先杀再编，见 §5.9）。

### 未实现功能

- [ ] `axctl init`
- [ ] 集成测试 CI 接入（dev_smoke / preview_smoke / build_smoke 骨架已就绪，均 #[ignore] 需 node+vite）
- [ ] `debug-ws` 调试子命令未来移除或并入 `info`（决策后执行）

### 已关闭项（历史）

- ~~`build` 未实现~~：vite build + 哨兵 + cargo release 全链已实现（§6），
  含 `dir_fingerprint` 单测 + build_smoke 黑盒冒烟。
- ~~`package` 未实现~~：封装 cargo-packager 已实现（commands/package.rs），
  复用 build 前端管线（哨兵保证内嵌最新 dist）、预检 packager 配置、
  产物扫描收窄到 bundle/ 目录。已端到端实测（fixture minimal-app +
  winres + packager 配置）：Windows NSIS 安装包产出成功；实测暴露并修复
  两处编排问题——(1) fixture 的 before-packaging-command 与 axctl 前置
  release 构建重复 → fixture 移除该 hook；(2) cargo-packager 产物实际落在
  `target/release/` 根（非 bundle/）→ report_artifacts 改为 profile 根排除
  裸二进制 + bundle/ 递归双位置探测；另补 fixture build.rs 的 winres
  Windows 文件信息嵌入（版本/公司/产品，右键属性可验证）。
- ~~P1-7 WS 隧道未实测~~：见上（已知问题区已标注解决）。
- ~~监听整个项目根~~：改为 workspace 感知 WatchSet（§5.8）。
- ~~配置只读 `[package.metadata.axctl]`~~：改为双层 workspace+package（§7.1）。
- ~~Windows 进程树终止~~：`process::kill_tree`（taskkill /t）已实现。
- ~~迁移根级 src/~~：已删除，逻辑全在双 crate 内。
- ~~tracing subscriber 未初始化~~：logging.rs 统一管道已实现（§10）。
- ~~Unix 进程树终止不完整~~：改为 tauri 式"递归 pgrep -P 找后代 + 杀树"，
  不依赖进程组（§5.9）。
- ~~后端 cargo run 残留孤儿~~：改为"先 `cargo build` 成功再 spawn 产物"，
  backend 叶子进程化（§5.9）。
- ~~代理 reqwest 跟随 30x~~：`redirect(Policy::none())` + 共享 Client +
  hop-by-hop 头过滤（§5.4）。
- ~~extra_watch_dirs 二次过滤失效~~：WatchSet 区分 rust member 目录与
  trigger-all 目录（§5.8）。
- ~~frontend_dev_url 半实现~~：devUrl 完整接入探测/复用/代理转发，并加
  vite 特征判断防误复用（§5.6）。
- ~~依赖闭包漏 rename / 含 dev-dep~~：改用 resolve.nodes 解析（§5.8）。
- ~~resolve_bin_name 伪兜底~~：改为显式报错并列出可用 bin（§7.4）。
- ~~重启抖动丢失~~：restarting 改为三态状态机（idle/restarting/pending），
  编译期间新变更置 pending，结束后补一轮（§5.9）。
- 集成测试骨架：`crates/axctl/tests/dev_smoke.rs`（#[ignore]，需 node+vite，
  黑盒冒烟：起 dev → 代理 200 → 改源码 → PID 变化）。
- ~~`axctl serve` 未实现~~：改为封装 `vite preview`（§11），已实测
  静态 200 + SPA fallback 200。
- ~~serve 就绪探测误报~~：HTTP 探测 + 子进程退出监控 + 稳定确认（§11.3），
  实测端口占用场景不再误报 ready。
- ~~npx 隐式下载 / host 注入面~~：`--no-install`；`parse_addr` host 白名单
  校验（dev/serve 共用，§7）。
- 重复代码：`shutdown_signal` 抽到 `process.rs`（dev/serve 共用）。
- ~~axctl-core::embed 未实现~~：include_dir + `frontend!` 宏 + FrontendAssets
  已实现（§6.2），fixture `tests/fixtures/embed-app/` 实测 release 内嵌。
- ~~axctl-core::serve::spa 未实现~~：SPA fallback 路由已实现（§6.4）——
  扩展名启发式回退 + 缓存头分层 + ETag/304；fixture 升级为 axum server
  实测 release HTTP（根/静态/SPA 路由/API 全 200，缺失 fetch 404）。
- ~~axctl build 未实现~~：完整链路已实现（§6.1）——前端 vite build +
  哨兵写入（dist 指纹）+ 后端 release 构建；哨兵传导经 reruntest 实证，
 并有 `crates/axctl/tests/build_smoke.rs` 自动化（#[ignore]，需 node+vite）。

## 10. 统一日志管道

### 10.1 设计目标

- tracing 是**唯一日志通道**；所有用户可见输出与内部诊断都发 tracing 事件。
- 默认展示与 tauri-cli 一致的最简风格；需要诊断时经 `RUST_LOG` 展开细节。
- stdout 留给"数据输出"（`info` / `debug-ws` 报告、子进程 cargo/vite 透传）；
  日志一律写 **stderr**，重定向时两者可分离。

### 10.2 两种展示模式

由 `RUST_LOG` 在启动时决定（`logging::init` 读一次，运行期不变）：

| RUST_LOG | 模式 | 效果 |
| --- | --- | --- |
| 未设置 / `info` / `warn` | **简洁** | `Info`/`Done`/`Warn`/`Error` 前缀 + 消息（tauri 风格） |
| 含 `debug` 或 `trace` | **详细** | `LEVEL target: message key=value`，展开全部字段 |

简洁模式示例：

```text
        Info axctl dev in D:\project
        Info starting vite dev server...
        Done reverse proxy listening on http://127.0.0.1:3000
```

详细模式示例（`RUST_LOG=debug`）：

```text
INFO axctl: building backend: sealantern-server (bin sealantern-server) event=ui.info
DEBUG axctl.dev: starting backend binary binary=D:\project\target\debug\sealantern-server.exe backend_addr=127.0.0.1:3001
DEBUG axctl.process: spawned child process name="backend" program="D:\...\sealantern-server.exe"
DEBUG axctl.watcher: watching directory dir=D:\project\server\src
```

### 10.3 target 约定

tracing 宏的 `target:` 参数只接受编译期字符串字面量，故代码里直接写值。
`logging.rs` 内 `target` 模块仅为文档汇聚（带 `#[allow(dead_code)]`）：

| target | 用途 |
| --- | --- |
| `axctl` | UI 事件（dev 进度等），简洁模式主通道 |
| `axctl.dev` | dev 编排 |
| `axctl.proxy` | 反向代理 |
| `axctl.process` | 子进程生命周期 |
| `axctl.watcher` | 文件监听 |
| `axctl.config` | 配置读取 |

`RUST_LOG` 支持定向过滤：`RUST_LOG=axctl.proxy=debug` 只看代理调试日志；
不设时第三方 crate 的 debug 不显示（EnvFilter 默认 info）。

### 10.4 level → 前缀映射

| level | event_name | 简洁模式前缀 |
| --- | --- | --- |
| ERROR | 任意 | `Error`（红） |
| WARN | 任意 | `Warn`（黄） |
| INFO | `ui.success` | `Done`（绿） |
| INFO | 其他 | `Info`（青） |
| DEBUG / TRACE | — | 简洁模式不显示 |

### 10.5 模块边界

- `logging.rs`：唯一日志模块。含 `init()`、自定义 `Layer`（格式化）、
  公开 API `info` / `success` / `warn` / `error` / `blank`（终端 UI 输出）、
  `ui_message`（通用底层，带 level + event_name）、`Fields`（事件字段收集）。
- 各命令/模块直接调用 `logging::info(...)` 等，或直接 `tracing::*!` 宏 +
  字面量 target。内部细节（spawn、watch 注册、WS 隧道建立/关闭）用 DEBUG 级，
  出错用 ERROR/WARN——简洁模式下自动隐藏细节、保留问题。
- 顶层错误由 `main` 捕获，经 `logging::error` 打印 anyhow 原因链（`{error:#}`）
  后 `exit(1)`，避免 Rust 默认 `Error: ... Caused by:` 双格式。

### 10.6 借鉴来源

参考 SeaLantern `crates/feature/src/observability.rs` 的"target + event_name +
薄封装函数"模式，但做减法：CLI 无常驻事件流，不需要事件键注册表，
只保留 target 约定与结构化字段，避免过度设计。

## 11. serve 模式：封装 vite preview

### 11.1 定位

`axctl serve` 是**纯前端生产预览**——封装 `vite preview`，服务
`vite.config.ts` 的 `build.outDir`（通常 `dist/`）。**无后端、无 API 代理**
（前端请求 `/api/*` 会失败，因为没有后端可转发）。

设计取舍：不自研静态服务（tower-http / axum fallback），直接复用 vite
preview——SPA fallback、mime、缓存头、history 路由全由 vite 处理，行为与
真实产物严格一致，且零新增依赖（复用 vite.rs 的包管理器探测与 spawn）。

### 11.2 CLI

```text
axctl serve [--dir <frontend-root>] [--addr 127.0.0.1:4173] [--open]
```

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--dir` | 当前目录 | 前端项目根（含 package.json / vite.config.ts） |
| `--addr` | 127.0.0.1:4173 | 监听地址（4173 = vite preview 惯例端口） |
| `--open` | false | 启动后自动开浏览器（透传 vite --open） |

### 11.3 流程

1. 定位前端根：`--dir` 优先，否则当前目录（须有 package.json）。
2. `vite::spawn_vite_preview`：按包管理器拼 `pnpm exec vite preview` /
   `npx --no-install vite preview` + `--host --port --strictPort`。
   `--strictPort`：端口被占直接报错，不静默换端口；
   `--no-install`：本地 node_modules 无 vite 时立即报错，不让 npx 联网装包。
3. 就绪探测（合并轮询，`wait_ready`）：
   - **HTTP GET /** 探测（非纯 TCP——TCP 连通可能是"端口占用者"而非 vite，
     裸 TCP 占用者 accept 但不响应 HTTP；探测带 2s 超时防挂起）；
   - 每轮先查子进程是否退出（`try_wait`）——vite 异步启动，`--strictPort`
     遇端口被占 / dist 缺失时**启动后**才报错退出，必须持续监控；
   - HTTP 首次通后隔 300ms "稳定确认"（子进程仍存活 + HTTP 仍通）才判
     Ready，覆盖"首轮连上占用者、vite 随后退出"的竞态窗口。
4. 错误区分：提前退出（报退出码 + 端口被占 / dist 缺失提示）vs 15s 超时。
5. 打印就绪地址，等待 Ctrl+C / SIGTERM → kill preview 进程树。

### 11.4 为什么无 cargo metadata / workspace 解析

serve 只在前端根跑 vite preview，不需要知道 backend member、依赖闭包等
workspace 信息——保持极简独立，与 dev 解耦。这也契合"纯前端预览"定位：
不编译、不解析、不写文件，只读 dist。

### 11.5 已知边界

- 需要项目里有 vite（node_modules 就绪）；否则 spawn 失败并报错。
- SPA fallback 对"真实缺失的静态资源"也回 index.html（vite preview
  行为）——预览场景可接受，正式 release 由 server 侧缓存策略处理。
- `--dir` 指向非本 workspace 的独立 dist 项目也可用（vite 自己读配置）。

