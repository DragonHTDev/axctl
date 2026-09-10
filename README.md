# axctl

An all-in-one development tool for **axum + Vite** projects, inspired by the
tauri-cli developer experience.

axctl replaces ad-hoc Node glue scripts (`dev` / `build` / `package` in a
`.mjs` file) with a single Rust CLI, and provides a companion library
(`axctl-core`) that embeds the built frontend into your server binary.

## Features

- **One unified entry port in dev** — a reverse proxy in front of Vite and
  your backend, so HTTP and WebSocket/HMR both work at `http://127.0.0.1:3000`.
- **Workspace-aware hot reload** — only the backend dependency closure is
  watched; unrelated crates do not trigger rebuilds.
- **Frontend-fresh production builds** — a `dist` sentinel contract makes
  Cargo re-embed the new frontend after `vite build`.
- **Native packaging** — the whole build pipeline plus `cargo-packager`
  installers (NSIS / MSI / DMG / deb / AppImage / …).

## Install

```bash
# from a checkout
cargo install --path crates/axctl

# or, once published
cargo install axctl
```

## Quick start

Declare your backend crate in the workspace `Cargo.toml`:

```toml
[workspace.metadata.axctl.dev]
backend_package = "my-server"        # the axum server crate
backend_port = 3001                  # backend dev listen port
proxy_addr = "127.0.0.1:3000"        # unified entry port
```

Then run, from the project root:

```bash
axctl dev       # http://127.0.0.1:3000  (proxy -> vite + backend)
axctl build     # production backend with the frontend embedded
axctl package   # native installers via cargo-packager
axctl serve     # preview the built frontend
axctl info      # environment diagnostics
```

## Commands

| Command   | Description                                                         | Status      |
| --------- | ------------------------------------------------------------------- | ----------- |
| `init`    | Initialize a project (detect environment, generate config)          | not yet     |
| `dev`     | Development mode with reverse proxy and hot reload                  | implemented |
| `build`   | Production build: `vite build` + sentinel + `cargo build --release` | implemented |
| `serve`   | Preview the built frontend via `vite preview`                       | implemented |
| `package` | Build + generate installers via `cargo-packager`                    | implemented |
| `info`    | Environment diagnostics                                             | implemented |

## Embedding the frontend (`axctl-core`)

Your server embeds the Vite build output and serves it with SPA fallback:

```rust,ignore
use axctl_core::embed::FrontendAssets;
use axum::Router;

// release: embeds `dist/` at compile time; debug: empty wrapper.
let assets: FrontendAssets<'static> =
    axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");

let app = Router::new()
    .nest("/api", api_routes)                          // your API first
    .fallback_service(axctl_core::serve::spa(assets)); // SPA fallback
```

To make Cargo re-embed after a frontend change, add the sentinel contract:

```rust
// build.rs
fn main() {
    println!("cargo:rerun-if-changed=dist/.axctl-sentinel");
    println!("cargo:rerun-if-changed=dist");
}
```

See [`crates/axctl-core/README.md`](crates/axctl-core/README.md) for details.

## Project layout

```
crates/
├── axctl/         # CLI binary (clap + command orchestration)
└── axctl-core/    # Library for user projects (embed / serve / probe)
docs/design.md     # architecture and design decisions
tests/fixtures/    # minimal-app (Vite + axum), embed-app (pure Rust)
```

## Development

```bash
cargo test --workspace            # unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Ignored black-box smoke tests (require Node + a built fixture):

```bash
cargo test -p axctl --test build_smoke -- --ignored --nocapture
```

## Documentation

See [`docs/design.md`](docs/design.md) for architecture and design decisions.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
