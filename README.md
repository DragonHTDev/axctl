# axctl

为 axum + Vite 项目提供一体化开发体验的命令行工具（对标 tauri-cli）。

## 子命令

| 命令    | 说明                                     |
| ------- | ---------------------------------------- |
| `init`  | 初始化项目（探测环境、生成配置）         |
| `dev`   | 开发模式：监听源码变化，自动重编译重启   |
| `build` | 生产构建：前端产物 + cargo release 构建  |
| `serve` | 静态预览：直接服务构建产物               |
| `package` | 打包：对接 cargo-packager 生成安装包   |
| `info`  | 环境诊断：输出 Rust / 前端 / 系统信息    |

## 开发

```bash
cargo run -- info      # 运行 axctl
pnpm axctl -- info     # 或经 pnpm 包装运行
cargo test             # 运行测试
```
