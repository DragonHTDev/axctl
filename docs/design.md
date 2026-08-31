# axctl 设计文档

面向 axum + Vite 项目的一体化开发工具（对标 tauri-cli 的开发体验）。
本文档记录架构决策、模块边界与关键实现方案。

## 1. 定位与目标

- 为 axum + Vite 项目提供 `dev` / `build` / `serve` / `package` / `info` 等一体化命令。
- 对标 tauri-cli：一条命令拉起完整开发环境，产物打包成单文件分发。
- 通用工具，不绑定具体业务；以 Cargo.toml `[package.metadata.axctl]` 为唯一配置入口。

## 2. 语言边界约定

| 范畴 | 语言 | 说明 |
| --- | --- | --- |
| 用户可见 | 英文 | CLI 输出、`--help`、README、docs.rs 文档注释、tracing 日志、发布说明 |
| 开发内部 | 中文 | 源码内部 `//` 注释、配置文件注释、git 提交信息、GitHub 简介 |

## 3. 总体架构：workspace 双 crate

```
axctl/                     # workspace 根
├── Cargo.toml             # [workspace] members = ["crates/axctl", "crates/axctl-core"]
├── crates/
│   ├── axctl/             # CLI：clap 定义 + 命令编排（薄壳，无核心逻辑）
│   └── axctl-core/        # 库：给用户项目直接依赖，负责内嵌/服务/探测
│       ├── embed.rs       #   内嵌静态资源（rust-embed，release 专用）
│       ├── serve.rs       #   运行时静态文件服务 + SPA fallback
│       └── probe.rs       #   环境探测（Rust / Node / pnpm 版本）
├── docs/design.md         # 本文档
└── src/                   # 旧根级 src（待迁移，见 §6）
```

### 3.1 为什么拆成双 crate

"把 web 构建产物内嵌进 server 二进制"的能力必须在**用户项目**里编译，
所以它天然是库 crate，而不是 CLI 逻辑。CLI 只负责编排命令，
内嵌 / 服务 / 探测等能力由 `axctl-core` 以库形式提供给用户项目。

## 4. 子命令设计

| 命令 | 职责 | 关键依赖 |
| --- | --- | --- |
| `init` | 初始化项目（探测环境、生成配置） | probe |
| `dev` | 开发模式（见 §5） | notify、vite、反向代理 |
| `build` | 生产构建（见 §6） | vite、cargo |
| `serve` | 静态预览：直接服务构建产物 | axctl-core::serve |
| `package` | 打包：对接 cargo-packager | cargo-packager |
| `info` | 环境诊断：输出 Rust / 前端 / 系统信息 | probe |

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
| HTTP 转发 | axum handler 接请求，reqwest 转发，body 流式转发（不整体读入内存） |
| WebSocket 隧道 | axum `WebSocketUpgrade` 接浏览器，`tokio-tungstenite` 连 vite，双向帧转发 + 保活 ping/pong + 关闭传播 |
| 路径分流 | `/api` 前缀 → 后端；其余（含 `/@vite/`、`/node_modules/` 等）→ vite |

新增依赖：`reqwest`（转发客户端）、`tokio-tungstenite`（WS 客户端）。

### 5.5 HMR 必须走隧道（关键坑）

用户的 vite 配置 `hmr` 未固定端口时，HMR client 会使用**页面加载端口**
（即 3000）连 WebSocket。因此 axctl 代理**必须**做 WS 隧道，否则 HMR 断裂、
前端改动触发整页刷新。WS 隧道是硬需求，不是可选项。

### 5.6 dev 完整链路

```
axctl dev
├─ 1. 检测/拉起 vite (5173)
├─ 2. notify 编译并启动后端 (3001)
├─ 3. 起反向代理 (3000)：/api → 3001，其余 → 5173，WS 隧道
└─ 4. 后端源码变化 → 重编译重启（代理和 vite 全程不动）
```

### 5.7 与 tauri 的对比

tauri 前端经 webview IPC 桥访问后端，不走 HTTP，因此无代理需求，对 vite.config
零侵入、只认 `devUrl`。axctl 的前端在浏览器里必须走 HTTP，因此用反向代理方案
统一入口，同样对 vite.config 零侵入，且兼容 tauri 项目的 `devUrl` 心智。

## 6. build 模式：内嵌 web 产物

### 6.1 链路

```
axctl build
├─ 1. 校验 frontend_root / package.json
├─ 2. vite build（pnpm 或 npm，可配置）      → dist/
├─ 3. 写入 dist/.axctl-sentinel（内容 = 产物 hash）  ← 哨兵
└─ 4. cargo build --release
       ├─ build.rs rerun-if-changed 命中 → 重编
       └─ 宏展开 → 新 dist 烙进二进制 ✅
```

### 6.2 内嵌方案：rust-embed

- release 模式编译期内嵌，单文件分发。
- debug 模式读磁盘，前端改动免重编译。
- `axctl-core` 提供 `include_frontend!` 宏 / Assets derive。

### 6.3 哨兵机制（关键坑）

cargo 的增量编译感知不到 dist 目录变化，过程宏读取 dist 后 cargo 认为"源码没变"
不重编，导致内嵌旧产物。解法：

- 用户项目 build.rs 写 `cargo:rerun-if-changed=../dist` 与 `.axctl-sentinel`
- axctl build 在 cargo build 前 touch 哨兵文件，保证触发重编

### 6.4 运行时服务（axctl-core::serve）

```
用户 server：
    .nest("/api", api_routes)
    .fallback(axctl_core::serve::spa(Assets));  // 找不到文件 → index.html
```

SPA fallback、mime、缓存头由 axctl-core 实现，替代 axum-vite。
dev 模式后端只挂 API（vite 负责前端），release 模式 API + 内嵌静态资源，
由 axctl-core 提供切换帮手。

## 7. 配置设计

### 7.1 唯一入口：Cargo.toml metadata

```toml
[package.metadata.axctl]
# dev 模式：后端 dev 监听端口（让开代理端口）
backend_port = 3001

[package.metadata.axctl.dev]
# 前端 dev server 地址；配置了且可达 → 复用（对应 tauri 的 devUrl）
frontend_dev_url = "http://127.0.0.1:5173"
# 代理监听地址
proxy_addr = "127.0.0.1:3000"

[package.metadata.axctl.build]
# 前端根目录（默认仓库根）
frontend_root = "."
# 构建命令（默认 pnpm 或 npm）
frontend_build_command = "pnpm"
```

### 7.2 配置读取

经 `cargo metadata` 读取 `[package.metadata.axctl]`，serde 反序列化。

## 8. 依赖清单

### crates/axctl（CLI）

| 依赖 | 用途 |
| --- | --- |
| clap | 参数解析 |
| anyhow | 错误处理 |
| tokio | 异步运行时 |
| tracing / tracing-subscriber | 日志 |
| serde / serde_json / toml | 配置 |
| notify-debouncer-full | 文件监听（防事件风暴） |
| reqwest | 代理 HTTP 转发 |
| tokio-tungstenite | 代理 WS 隧道 |
| axum | 代理服务器框架 |

### crates/axctl-core（库）

| 依赖 | 用途 |
| --- | --- |
| rust-embed | 内嵌静态资源 |
| axum | 静态服务 / SPA fallback |
| mime_guess | mime 推断 |

## 9. 待办 / 开放问题

- [ ] 迁移根级 `src/` 到 `crates/axctl/src/`（§6 完成后）。
- [ ] `init` 子命令脚手架内容。
- [ ] Windows 进程树终止（`taskkill /t`）跨平台封装。
- [ ] axum 0.7 / 0.8 兼容策略。
