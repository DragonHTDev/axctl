# 02 · Architecture

## Repository layout

```
axctl/                          # workspace root (virtual manifest)
├── Cargo.toml                  # [workspace] members = ["crates/axctl", "crates/axctl-core"]
├── crates/
│   ├── axctl/                  # CLI: clap definitions + command orchestration
│   └── axctl-core/             # library for user projects (embed / serve / probe)
├── tests/fixtures/             # test fixtures (minimal-app / embed-app)
├── docs/                       # this handbook (zh-CN / en)
└── .github/workflows/ci.yml    # CI
```

### Why two crates

Embedding the built frontend into a server binary must happen when the **user
project** compiles, so it naturally lives as a library crate. The CLI only
orchestrates commands; the embed / serve / probe capabilities are exposed to user
projects by `axctl-core`.

## CLI modules (`crates/axctl`)

| Module            | Responsibility                                                       |
| ----------------- | -------------------------------------------------------------------- |
| `main.rs`         | clap entry, `dispatch`, `run_async`, hidden `debug-ws` command        |
| `logging.rs`      | Unified log pipeline (tracing layer, two display modes)              |
| `config.rs`       | Two-layer config read (workspace + package merge), address parsing   |
| `workspace.rs`    | `cargo_metadata` → `WorkspaceInfo` / `MemberInfo`                    |
| `watch_set.rs`    | `WorkspaceInfo` + config → `WatchSet` (exact paths to watch)         |
| `watcher.rs`      | `notify` watching (registers only the WatchSet's exact paths)        |
| `backend.rs`      | backend resolution / build / artifact path (build-then-spawn)        |
| `process.rs`      | Cross-platform child-process management (tree kill, shutdown signal) |
| `proxy.rs`        | Reverse proxy (HTTP forwarding + WebSocket tunneling)                |
| `vite.rs`         | Vite detect / start (dev) / preview (serve) / build (build)          |
| `commands/*.rs`   | Subcommand implementations (dev / build / serve / package / info)    |

### Module dependency direction

```
dev.rs ──► workspace.rs ──► config.rs (reads metadata)
   │            │
   │            └──────► watch_set.rs ──► watcher.rs
   ├──► vite.rs / process.rs / proxy.rs / backend.rs / logging.rs
```

`workspace.rs`, `watch_set.rs` and `config.rs` are **pure data modules** (parsing
and computation, no IO side effects) and are unit-tested independently; `dev.rs`
only orchestrates.

## Library modules (`crates/axctl-core`)

| Module      | Responsibility                                                             |
| ----------- | -------------------------------------------------------------------------- |
| `embed.rs`  | Release embedding: `frontend!` macro + `FrontendAssets` wrapper (empty in debug) |
| `serve.rs`  | Runtime static serving: `spa()` SPA fallback router (cache headers / ETag)  |
| `probe.rs`  | Environment probing (Rust / Node / package manager / platform)             |

> Public doc comments in `axctl-core` (docs.rs) are English; CLI-internal
> comments are Chinese. See [Development · Language rule](06-development.md#language-rule).

## Data flow of a `dev` run

```
user hits :3000
      │
      ▼
 proxy.rs ──┬── /api/* ───────► backend server (:3001)
            ├── everything else ► vite dev server (:5173)
            └── WS upgrade ────► vite (HMR tunnel)
      ▲
      │ orchestrated by dev.rs: workspace → vite ready → backend build+spawn → proxy → watch
```
