# axctl

An all-in-one development tool for **axum + Vite** projects, inspired by the
tauri-cli developer experience.

axctl replaces ad-hoc Node glue scripts (`dev` / `build` / `package` in a
`.mjs` file) with a single Rust CLI:

- **dev** — start Vite, build & run the backend, and expose one unified entry
  port with a reverse proxy (HTTP + WebSocket/HMR) and source-watch hot reload.
- **build** — `vite build` → write the dist sentinel → `cargo build --release`
  so the backend re-embeds the fresh `dist/`.
- **package** — the full build pipeline followed by `cargo-packager` to
  produce native installers.
- **serve** — preview the built frontend via `vite preview` (no backend).
- **info** — environment diagnostics (Rust / Node / package manager).

## Install

```bash
# npm (recommended: prebuilt binaries per platform)
npx axctl --help

# crates.io (installs the `axctl` command)
cargo install axctl-rs
```

## Quick start

Point axctl at your backend crate in the workspace `Cargo.toml`:

```toml
[workspace.metadata.axctl.dev]
backend_package = "my-server"        # the axum server crate
backend_port = 3001                  # backend dev listen port
proxy_addr = "127.0.0.1:3000"        # unified entry port
```

Then run, from the project root:

```bash
axctl dev       # http://127.0.0.1:3000  (proxy -> vite + backend)
axctl build     # production backend with embedded frontend
axctl package   # native installers via cargo-packager
axctl serve     # preview the built frontend
axctl info      # environment diagnostics
```

`axctl dev` is workspace-aware: it only watches the backend dependency
closure, so unrelated crates do not trigger rebuilds. If the workspace has
multiple binary crates, axctl warns and tells you how to pin the backend with
`backend_package`.

## Commands

| Command   | Description                                                         | Status       |
| --------- | ------------------------------------------------------------------- | ------------ |
| `init`    | Initialize a project (detect environment, generate config)          | not yet      |
| `dev`     | Development mode with reverse proxy and hot reload                  | implemented  |
| `build`   | Production build: `vite build` + sentinel + `cargo build --release` | implemented  |
| `serve`   | Preview the built frontend via `vite preview`                       | implemented  |
| `package` | Build + generate installers via `cargo-packager`                    | implemented  |
| `info`    | Environment diagnostics                                             | implemented  |

The runtime side of embedding the frontend into your server lives in the
companion crate [`axctl-core`](https://crates.io/crates/axctl-core)
(`frontend!` macro + SPA serving).

## Documentation

See the [handbook](https://github.com/dragonhtdev/axctl/blob/master/docs/README.md)
for architecture, configuration, and development guides.

## License

Licensed under either of
[Apache-2.0](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-APACHE)
or [MIT](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-MIT) at your
option.
