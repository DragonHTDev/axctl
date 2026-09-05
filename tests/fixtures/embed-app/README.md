# embed-app fixture

验证 `axctl-core::embed` 的 release 内嵌机制。

## 结构

- `Cargo.toml`：独立 crate（空 `[workspace]` 自绝），path 依赖
  axctl-core + include_dir（include_dir! 展开引用裸路径，调用方须直连）。
- `src/main.rs`：`axctl_core::frontend!` 宏绑定 `static/`（release 内嵌 /
  debug 空包装），启动时断言能取到 index.html / assets/app.js、不存在文件
  返回 None、files() 递归列出文件。
- `static/`：版本化假前端产物（**入库**，非 vite 生成的 dist）。

> 目录叫 `static/` 而非 `dist/`：`tests/fixtures/*/dist/` 被 .gitignore
> 忽略（那是 vite 产物目录约定），embed fixture 需要内容入库验证内嵌。
> 目录名不影响内嵌机制。

原理与形态取舍（为何宏而非 derive）见 design §6.2，本 README 只给操作步骤。

## 验证

```powershell
# release：真内嵌
cargo run --release

# 可选：确认不依赖磁盘（改名 static → 二进制仍能读到文件 = 内嵌成功）
# 注意：PowerShell 对名为 static 的目录用 Rename-Item 会报设备名解析错
# （"represents a path or device name"），须用 Move-Item -LiteralPath。
Move-Item -LiteralPath static -Destination static-hidden
.\target\release\axctl-embed-fixture.exe   # 仍输出 ok
Move-Item -LiteralPath static-hidden -Destination static

# debug：不内嵌（frontend! 展开为空包装，dev 前端归 vite）
cargo run
```
