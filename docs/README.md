# axctl Handbook

Development manual for **axctl** — an all-in-one tool for axum + Vite projects.

**Languages / 语言**

- [中文（zh-CN）](zh-CN/01-overview.md)
- [English (en)](en/01-overview.md)

---

## What is axctl? / axctl 是什么

axctl brings a tauri-cli-like developer experience to **axum + Vite** projects:
one command to bring up the whole development environment, one command to
produce a distributable artifact. It ships as a CLI (`axctl`) plus a library
(`axctl-core`) that embeds the built frontend into your server binary.

axctl 为 **axum + Vite** 项目带来对标 tauri-cli 的开发体验：一条命令拉起完整开发
环境，一条命令产出可分发产物。它由 CLI（`axctl`）与库（`axctl-core`，负责把前端
构建产物内嵌进 server 二进制）两部分组成。

## Contents / 目录

| #  | Chapter (English)                         | 章节（中文）                        |
| -- | ----------------------------------------- | ----------------------------------- |
| 01 | [Overview](en/01-overview.md)             | [概览](zh-CN/01-overview.md)        |
| 02 | [Architecture](en/02-architecture.md)     | [架构](zh-CN/02-architecture.md)    |
| 03 | [Configuration](en/03-configuration.md)   | [配置参考](zh-CN/03-configuration.md) |
| 04 | [Commands](en/04-commands.md)             | [命令参考](zh-CN/04-commands.md)    |
| 05 | [Design notes](en/05-design-notes.md)     | [设计要点](zh-CN/05-design-notes.md) |
| 06 | [Development](en/06-development.md)       | [开发指南](zh-CN/06-development.md) |

## Language rule / 语言边界约定

| Scope                                                                    | Language |
| ------------------------------------------------------------------------ | -------- |
| User-visible: CLI output, `--help`, README, docs.rs comments, logs        | English  |
| Internal: CLI crate comments, git commits, this handbook's `zh-CN` pages   | 中文     |

详见 [开发指南 · 语言边界约定](zh-CN/06-development.md#语言边界约定)。
See [Development · Language rule](en/06-development.md#language-rule).
