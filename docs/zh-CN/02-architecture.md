# 02 · 架构

## 仓库布局

```
axctl/                          # workspace 根（虚拟 manifest）
├── Cargo.toml                  # [workspace] members = ["crates/axctl", "crates/axctl-core"]
├── crates/
│   ├── axctl/                  # CLI：clap 定义 + 命令编排
│   └── axctl-core/             # 库：供用户项目依赖（内嵌 / 服务 / 探测）
├── tests/fixtures/             # 测试夹具（minimal-app / embed-app）
├── docs/                       # 本手册（zh-CN / en）
└── .github/workflows/ci.yml    # CI
```

### 为什么拆成双 crate

"把前端构建产物内嵌进 server 二进制"必须在**用户项目**里编译，所以它天然是
库 crate。CLI 只负责命令编排；内嵌 / 服务 / 探测等能力由 `axctl-core` 以库
形式提供给用户项目。

## CLI 模块（`crates/axctl`）

| 模块              | 职责                                                       |
| ----------------- | ---------------------------------------------------------- |
| `main.rs`         | clap 入口、`dispatch`、`run_async`、隐藏调试命令 `debug-ws` |
| `logging.rs`      | 统一日志管道（tracing 自定义 Layer，两种展示模式）          |
| `config.rs`       | 双层配置读取（workspace + package merge）、地址解析        |
| `workspace.rs`    | `cargo_metadata` → `WorkspaceInfo` / `MemberInfo`          |
| `watch_set.rs`    | `WorkspaceInfo` + 配置 → `WatchSet`（要监听的精确路径集合） |
| `watcher.rs`      | `notify` 监听（只注册 WatchSet 中的精确路径）               |
| `backend.rs`      | backend 解析 / 编译 / 产物路径（先 build 后 spawn）         |
| `process.rs`      | 跨平台子进程管理（进程树终止、shutdown 信号）               |
| `proxy.rs`        | 反向代理（HTTP 转发 + WebSocket 隧道）                      |
| `vite.rs`         | vite 探测 / 启动（dev）/ 预览（serve）/ 构建（build）       |
| `commands/*.rs`   | 各子命令实现（dev / build / serve / package / info）        |

### 模块依赖方向

```
dev.rs ──► workspace.rs ──► config.rs（读取 metadata）
   │            │
   │            └──────► watch_set.rs ──► watcher.rs
   ├──► vite.rs / process.rs / proxy.rs / backend.rs / logging.rs
```

`workspace.rs`、`watch_set.rs`、`config.rs` 是**纯数据模块**（解析 + 计算，
无 IO 副作用），可独立单元测试；`dev.rs` 只做编排。

## 库模块（`crates/axctl-core`）

| 模块        | 职责                                                              |
| ----------- | ----------------------------------------------------------------- |
| `embed.rs`  | release 内嵌：`frontend!` 宏 + `FrontendAssets` 包装值（debug 空包装） |
| `serve.rs`  | 运行时静态服务：`spa()` SPA fallback 路由（缓存头 / ETag）         |
| `probe.rs`  | 环境探测（Rust / Node / 包管理器 / 平台）                          |

> `axctl-core` 的公开文档注释（docs.rs）用英文；CLI 内部注释用中文。
> 详见 [开发指南 · 语言边界约定](06-development.md#语言边界约定)。

## 一次 `dev` 的数据流

```
用户访问 :3000
      │
      ▼
 proxy.rs ──┬── /api/* ───────► 后端 server (:3001)
            ├── 其他 ──────────► vite dev server (:5173)
            └── WS 升级 ───────► vite（HMR 隧道）
      ▲
      │ 由 dev.rs 编排：workspace 解析 → vite 就绪 → backend build+spawn → 代理 → 监听重启
```
