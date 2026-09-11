# 06 · 开发指南

## 环境要求

- Rust（stable，`rustfmt` + `clippy` 组件）
- Node.js + pnpm（仅集成测试 / fixture 需要）

## 常用命令

```bash
# 构建 / 运行
cargo build
cargo run -p axctl -- --help
cargo run -p axctl -- info

# 质量门（提交前必须全绿）
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 发版前的打包清单校验
cargo package -p axctl --list
cargo package -p axctl-core --list
```

## 语言边界约定

| 范畴                                                                       | 语言 |
| -------------------------------------------------------------------------- | ---- |
| 用户可见：CLI 输出、`--help`、README、docs.rs 公开注释、tracing 日志         | 英文 |
| 开发内部：CLI crate 内部注释、git 提交信息、本手册 `zh-CN` 页面              | 中文 |

要点：

- `crates/axctl`（二进制）不进 docs.rs，其 `//!` / `///` 统一**中文**。
- `crates/axctl-core`（库）的**公开**文档注释（`//!` / `///`）用**英文**；
  其内部过程性注释（`//`）用**中文**。
- 提交信息用中文，风格 `type(scope): 描述`。

## 测试

### 单元测试

测试与被测模块同文件（`#[cfg(test)] mod tests`）。纯数据模块
（`workspace.rs` / `watch_set.rs` / `config.rs`）应尽量覆盖分支。

### 测试夹具（`tests/fixtures/`）

| 夹具          | 用途                                                                 |
| ------------- | -------------------------------------------------------------------- |
| `minimal-app` | Vite + axum 全链验证：`dev` / `serve` / `build` / `package` + 哨兵契约 |
| `embed-app`   | 纯 Rust：`frontend!` 内嵌 + `spa` 服务验证（无 vite）                 |

`minimal-app` 是**独立 workspace**（自己的 `[workspace]`），不参与主 workspace 构建。
它含 `[package.metadata.packager]` 与 `build.rs`（哨兵契约 + winres Windows 文件信息）。

### 黑盒冒烟（`crates/axctl/tests/`）

`dev_smoke` / `preview_smoke` / `build_smoke` 均为 `#[ignore]`，需要 Node + vite
且 fixture 前端已构建：

```bash
# 先构建 fixture 前端
cd tests/fixtures/minimal-app && pnpm exec vite build && cd ../../..

cargo test -p axctl --test build_smoke -- --ignored --nocapture
```

这些不进常规 CI（依赖 Node 环境）。

## CI

`.github/workflows/ci.yml` 在 push / PR 上跑：

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo package -p axctl --list` 与 `cargo package -p axctl-core --list`

## 提交与评审

- 一个逻辑改动一个提交，信息简明（中文）。
- 改动后本地跑齐"质量门"三件套再提交。
- 发版相关流程见仓库发版说明（待补充）。
