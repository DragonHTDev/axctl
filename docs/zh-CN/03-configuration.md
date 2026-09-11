# 03 · 配置参考

axctl 的配置入口是**用户项目 `Cargo.toml` 的 metadata**，无需额外配置文件。

## 双层配置

配置可写在 workspace 根，也可写在某个 member，两层按字段合并：

```toml
# workspace 根 Cargo.toml（虚拟 manifest 项目的主配置）
[workspace.metadata.axctl.dev]
backend_package = "my-server"
backend_port = 3001
proxy_addr = "127.0.0.1:3000"
frontend_dev_url = "http://127.0.0.1:5173"
frontend_root = "."

# 某个 member 内（如 server/Cargo.toml）——只覆盖指定字段
[package.metadata.axctl.dev]
backend_port = 3999
```

> 为什么必须支持 workspace 层：真实项目的根 `Cargo.toml` 常是**虚拟 manifest**
> （只有 `[workspace]`、没有 `[package]`）。若只认 `[package.metadata.axctl]`，
> 这类项目的配置会完全失效。

### 优先级（从高到低）

```
package.metadata.axctl.dev     # 当前 member 的 dev 嵌套段
package.metadata.axctl         # 当前 member 的扁平字段
workspace.metadata.axctl.dev   # workspace 根的 dev 嵌套段
workspace.metadata.axctl       # workspace 根的扁平字段
默认值
```

低层缺失的字段由高层补齐（字段级 merge）。

## 字段

| 字段                     | 类型       | 默认           | 说明                                                         |
| ------------------------ | ---------- | -------------- | ------------------------------------------------------------ |
| `backend_port`           | `u16`      | `3001`         | dev 后端监听端口（必须让开代理端口）                           |
| `frontend_dev_url`       | `string`   | 无             | 已运行的 vite 地址（对应 tauri `devUrl`）；配置了则探测复用    |
| `proxy_addr`             | `string`   | `127.0.0.1:3000` | dev 代理监听地址（用户唯一入口）                             |
| `frontend_root`          | `string`   | workspace 根   | 前端目录（相对 workspace 根）                                  |
| `frontend_build_command` | `string`   | pnpm / npm 探测 | 前端构建命令                                                 |
| `backend_command`        | `string`   | 无             | 兼容写法：从中解析 `-p` / `--bin` 以定位 backend member        |
| `backend_package`        | `string`   | 见下           | 后端 package 名（多 binary workspace 时必填）                  |
| `extra_watch_dirs`       | `string[]` | `[]`           | 额外监听目录（相对 workspace 根）；**任何变化都触发重启**；两层配置拼接去重 |

## backend member 解析

确定"谁是需要监听 / 重启 / 打包的后端 crate"，按以下优先级（`backend.rs`）：

1. `backend_package` 配置
2. `backend_command` 里的 `-p` / `--package`（或 `--bin`，经 `package_for_bin` 反查）
3. cwd 所在 member
4. workspace 内**唯一**的 binary package

若以上都失败（多 binary 且未显式配置），axctl 会**明确报错**并提示配置
`backend_package`——不会静默猜一个。多 binary workspace 且未显式配置时，
成功解析后还会打印一条提示，告知实际选中了哪个 crate。

> 二进制启动采用"先 `cargo build -p <pkg> --bin <bin>`、再 spawn 编译产物"，
> **不直接执行 `backend_command`**。`backend_command` 仅用于定位 member。

## 地址与安全

- 监听地址经 `parse_addr` 解析，host 做字符白名单校验（拒绝 shell 元字符，
  防注入，因为 host 可能被拼进命令串）。
- `proxy_addr` / serve 的 `--addr` 配成**非 loopback**（如 `0.0.0.0`）时，
  axctl 打印警告——会把无鉴权的后端与 vite dev server 暴露到网络（vite 的
  `@fs` 与 HMR 有历史 RCE，如 CVE-2025-30221），仅应在受信本机开发环境使用。
