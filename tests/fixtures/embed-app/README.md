# embed-app fixture

验证 `axctl-core::embed` 的 release 内嵌机制。

## 结构

- `Cargo.toml`：独立 crate（空 `[workspace]` 自绝），path 依赖 axctl-core。
- `src/main.rs`：`#[derive(Embed)]` 内嵌 `static/`，启动时断言能取到
  index.html / assets/app.js、不存在文件返回 None、iter 列出文件。
- `static/`：版本化假前端产物（**入库**，非 vite 生成的 dist）。

> 目录叫 `static/` 而非 `dist/`：`tests/fixtures/*/dist/` 被 .gitignore
> 忽略（那是 vite 产物目录约定），embed fixture 需要内容入库验证内嵌。
> folder 名不影响内嵌机制。

## 验证

```powershell
# release：真内嵌。改 static 名后二进制仍能读到文件 = 内嵌成功
cargo run --release

# 可选：确认不依赖磁盘
Rename-Item static static-hidden
.\target\release\axctl-embed-fixture.exe   # 仍输出 ok
Rename-Item static-hidden static

# debug：不内嵌、从磁盘读（static 在则 ok；serve 层 debug 不会调 get）
cargo run
```
