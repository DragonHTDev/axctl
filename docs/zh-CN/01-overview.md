# 01 · 概览

axctl 是面向 **axum + Vite** 项目的一体化开发工具，对标 tauri-cli 的开发体验：
一条命令拉起完整开发环境，一条命令产出可分发产物。

## 解决什么问题

传统 axum + Vite 项目的开发流程往往依赖手写的 Node 胶水脚本：启动 vite、
编译后端、起代理、监听源码重启、打包产物。这类脚本分散、难测试、平台差异多。

axctl 把这些收敛为统一的 Rust CLI：

| 命令        | 作用                                                                 |
| ----------- | -------------------------------------------------------------------- |
| `axctl dev` | 启动 vite、编译并运行后端、暴露统一入口（反向代理 HTTP + WebSocket/HMR），源码变化自动重编译重启 |
| `axctl build` | `vite build` → 写 dist 哨兵 → `cargo build --release`（后端编译期内嵌最新前端） |
| `axctl serve` | 通过 `vite preview` 预览构建产物（纯前端，无后端）                     |
| `axctl package` | 完整构建链 + `cargo-packager` 产出原生安装包                        |
| `axctl info` | 环境诊断（Rust / Node / 包管理器）                                     |

`init` 尚未实现。

## 两个 crate

axctl 是 workspace 双 crate 结构：

| crate        | 类型       | 职责                                                          |
| ------------ | ---------- | ------------------------------------------------------------- |
| `axctl`      | CLI 二进制 | 命令编排（dev / build / serve / package / info）               |
| `axctl-core` | 库         | 供**用户项目**依赖：编译期内嵌前端（`frontend!`）+ SPA 服务（`spa`） |

为什么 `axctl-core` 是库而不是 CLI 的一部分：把"前端构建产物内嵌进 server
二进制"的能力必须在**用户项目**里编译（宏在调用方 crate 展开），因此它天然
以库形式存在。CLI 只负责编排，不参与用户项目的编译。

## 与 tauri-cli 的差异

tauri 的前端经 webview 的 IPC 访问后端，**不走 HTTP**，所以无需代理、只认
`devUrl`。axctl 的前端在浏览器/WebView 里通过 **HTTP** 访问后端，因此用
**反向代理**统一入口（默认 `127.0.0.1:3000`）——同样对 `vite.config` 零侵入，
并兼容 tauri 项目的 `devUrl` 心智（`frontend_dev_url`）。

## 下一步

- [架构](02-architecture.md) — 仓库布局与模块边界
- [配置参考](03-configuration.md) — Cargo.toml metadata 配置
- [命令参考](04-commands.md) — 各命令的完整行为
- [设计要点](05-design-notes.md) — 关键决策与踩坑记录
- [开发指南](06-development.md) — 本地开发 / 测试 / 提交
