# 04 · 命令参考

```
axctl <COMMAND> [OPTIONS]
```

可用命令：`dev` / `build` / `serve` / `package` / `info`（`init` 未实现）。

## `axctl dev`

启动完整开发环境：vite + 后端 + 反向代理 + 源码监听重启。

### 端口规划

| 端口 | 用途                            | 说明                                       |
| ---- | ------------------------------- | ------------------------------------------ |
| 3000 | axctl 代理（用户唯一入口）      | release 时后端直接监听 3000，入口不变       |
| 5173 | vite dev server                 | 前端 + HMR                                  |
| 3001 | 后端 dev 监听                   | 必须让开 3000，可配置（`backend_port`）     |

### 链路

```
axctl dev
├─ 0. cargo metadata → WorkspaceInfo + 双层配置 + backend target
├─ 1. 前端：配了 frontend_dev_url → 探测复用；否则探测/拉起 5173
├─ 2. 后端：cargo build -p <pkg> --bin <bin> → spawn 产物（:3001）
├─ 3. 反向代理 (:3000)：/api → 后端，其余 → vite，WS 隧道
├─ 4. 计算 WatchSet（backend 依赖闭包 + extra_watch_dirs）并监听
└─ 5. 源码变化 → 重编译重启（代理与 vite 全程不动）
```

### 代理行为

| 能力          | 实现                                                                 |
| ------------- | -------------------------------------------------------------------- |
| HTTP 转发     | axum handler 接请求，reqwest 转发（共享 Client，连接复用），body 流式转发 |
| 重定向透传    | `redirect(Policy::none())`——30x 原样透传，保留 `Location`（登录跳转 / OAuth 必需） |
| hop-by-hop 头 | 请求侧过滤 `connection` / `upgrade` / `transfer-encoding` 等；响应侧过滤连接管理头 |
| WebSocket     | axum `WebSocketUpgrade` 接浏览器，`tokio-tungstenite` 连 vite，双向帧转发（含子协议 / path+query 透传） |
| 路径分流      | `/api` 前缀 → 后端；其余（`/@vite/`、`/node_modules/` 等）→ vite     |

### 热重载

- 监听范围 = backend 的 **path dependency 闭包**（workspace 内）+ `extra_watch_dirs`，
  只注册精确路径，不监听整个项目根。
- 重启采用三态状态机（`idle` / `restarting` / `pending`）：重启进行中收到新变更
  置 `pending`，本轮结束后补一轮，**不丢变更**。
- 重启语义有平台差异：Unix 先编译成功再杀旧进程（编译期 API 不断）；
  Windows 因 exe 文件锁须先杀旧进程再编译。

### 安全提示

`proxy_addr` 非 loopback 时打印警告（见 [配置参考](03-configuration.md#地址与安全)）。

## `axctl build`

生产构建，把前端内嵌进后端 release 二进制。

```
axctl build [--dir <frontend-root>]
```

链路：定位前端根 → `vite build`（产出 `dist/`）→ 写 `dist/.axctl-sentinel`
（dist 指纹哨兵）→ `cargo build --release -p <backend> --bin <bin>`。

哨兵让 cargo 感知 dist 变化，触发 `build.rs` 重跑 → 主 crate 重编 → `frontend!`
重读新 dist。用户 server 需在 `build.rs` 声明契约（见
[设计要点 · 哨兵机制](05-design-notes.md#哨兵机制前端新鲜度)）。

## `axctl serve`

纯前端生产预览，封装 `vite preview`。**无后端、无 API 代理**。

```
axctl serve [--dir <frontend-root>] [--addr 127.0.0.1:4173] [--open]
```

| 参数     | 默认               | 说明                              |
| -------- | ------------------ | --------------------------------- |
| `--dir`  | 当前目录           | 前端项目根（含 `package.json`）    |
| `--addr` | `127.0.0.1:4173`   | 监听地址                           |
| `--open` | `false`            | 启动后自动开浏览器                 |

就绪探测：HTTP `GET /` + 每轮检查子进程是否退出 + 300ms 稳定确认——避免把
"端口占用者"误判为 ready。`--strictPort` 让端口被占直接报错；`--no-install`
让本地无 vite 时立即报错（不联网装包）。

## `axctl package`

完整构建链 + `cargo-packager` 产出原生安装包。

```
axctl package [--dir <frontend-root>] [--format <FMT>]... [--package <PKG>] [--out-dir <DIR>]
```

| 参数        | 说明                                                        |
| ----------- | ----------------------------------------------------------- |
| `--format`  | 覆盖安装包格式（`nsis` / `wix` / `dmg` / `deb` / `appimage` 等），可多次；不传则由 cargo-packager 按配置决定 |
| `--package` | 指定要打包的 backend package（默认按解析规则）               |
| `--out-dir` | 安装包输出目录（透传 cargo-packager）                        |

链路：复用 `build` 的前端管线（vite build + 哨兵）→ `cargo build --release`
→ 在 backend member 目录执行 `cargo packager --release` → 扫描并列出产物。

packager 配置写在 backend crate 的 `[package.metadata.packager]`（或 `Packager.toml`），
由 cargo-packager 原生读取。**不配 `before-packaging-command`**——axctl 已在
前置步骤构建 release，再配会重复（幂等但多余）。

## `axctl info`

环境诊断：输出 Rust / Node / 包管理器 / 平台信息。

## `debug-ws`（隐藏）

打印 workspace 解析结果与计算出的 WatchSet，用于验证 `cargo_metadata` 路径在
真实项目上的表现。不进 `--help`。
