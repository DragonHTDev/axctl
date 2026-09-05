# minimal-app fixture

axctl dev 冒烟测试用的最小 axum + Vite 项目。

## 用途

验证 `axctl dev` 的完整链路：

- 前端代理：`http://127.0.0.1:3000/` → vite (5173)
- 后端代理：`http://127.0.0.1:3000/api/status` → 后端 (3001)
- 热重载：修改 `src/main.rs` 后后端自动重启（`/api/status` 返回的 `pid` 会变化）

`/api/status` 返回进程 PID，用于断言"重启确实发生了"。

## 为什么放仓库里

源文件自 Temp 目录转移而来（原 `opencode\axctl-fixture`），
避免临时目录被系统清理后丢失冒烟测试基准。

## 结构

- `Cargo.toml`：单 package axum 后端，带 `[package.metadata.axctl]` 配置示例。
  后端从环境变量 `AXCTL_BACKEND_ADDR` 读监听地址（axctl 传入）。
  文件顶部有空的 `[workspace]` 表——声明本包独立于外层 axctl workspace，
  否则 cargo 向上找到 axctl 根 workspace 而本包不在 members 会报错。
- `src/main.rs`：axum server，`/api/status` + `/api/hello`。
- `package.json` / `vite.config.ts` / `index.html`：vite 前端。
  vite 固定 5173 端口、关闭清屏（避免冲掉 axctl 输出）。
- `.npmrc` / `pnpm-workspace.yaml`：pnpm 9 的 esbuild build-script 白名单
  （否则 `pnpm exec vite` 前会报 `ERR_PNPM_IGNORED_BUILDS`）。
  `pnpm-workspace.yaml` 的 `packages: ["."]` 同时声明本目录是独立的 pnpm
  workspace 根，避免向上命中 axctl 仓库根的 workspace 配置导致 vite 解析错乱。

## 使用

```powershell
# 安装前端依赖（首次，产物不入 git）
pnpm install

# 在 fixture 目录运行
axctl dev

# 验证
curl http://127.0.0.1:3000/          # → vite 页面
curl http://127.0.0.1:3000/api/status # → {"ok":true,...}
```

注意：fixture 是独立项目，**不是** axctl workspace 的 member，
不会被 `cargo build --workspace` 编译。
