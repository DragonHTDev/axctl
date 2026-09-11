# 04 · Commands

```
axctl <COMMAND> [OPTIONS]
```

Commands: `dev` / `build` / `serve` / `package` / `info` (`init` not implemented).

## `axctl dev`

Brings up the full development environment: Vite + backend + reverse proxy +
source-watch restart.

### Ports

| Port | Purpose                          | Notes                                                 |
| ---- | -------------------------------- | ----------------------------------------------------- |
| 3000 | axctl proxy (single user entry)  | On release the backend listens on 3000 directly — the entry does not change |
| 5173 | Vite dev server                  | frontend + HMR                                         |
| 3001 | backend dev listen               | must avoid 3000; configurable (`backend_port`)         |

### Flow

```
axctl dev
├─ 0. cargo metadata → WorkspaceInfo + two-layer config + backend target
├─ 1. frontend: if frontend_dev_url is set → reuse it; else probe/start 5173
├─ 2. backend: cargo build -p <pkg> --bin <bin> → spawn artifact (:3001)
├─ 3. reverse proxy (:3000): /api → backend, else → vite, WS tunnel
├─ 4. compute WatchSet (backend dependency closure + extra_watch_dirs) and watch
└─ 5. source change → rebuild & restart (proxy and vite stay up)
```

### Proxy behavior

| Capability        | Implementation                                                              |
| ----------------- | --------------------------------------------------------------------------- |
| HTTP forwarding   | axum handler, forwarded via reqwest (shared Client, keep-alive), streamed body |
| Redirect passthrough | `redirect(Policy::none())` — 30x passed through as-is with `Location` (needed for login / OAuth) |
| hop-by-hop headers | request side filters `connection` / `upgrade` / `transfer-encoding` etc.; response side filters connection-management headers |
| WebSocket         | axum `WebSocketUpgrade` from the browser, `tokio-tungstenite` to Vite, bidirectional frame forwarding (with subprotocol and path+query passthrough) |
| Path split        | `/api` prefix → backend; everything else (`/@vite/`, `/node_modules/`, ...) → vite |

### Hot reload

- Watch scope = the backend's **path dependency closure** (within the workspace)
  plus `extra_watch_dirs`; only exact paths are registered, never the whole root.
- Restart uses a three-state machine (`idle` / `restarting` / `pending`): changes
  arriving during a restart set `pending` and are picked up in another round —
  **no change is lost**.
- Restart semantics differ by platform: on Unix, build succeeds first and only then
  is the old process killed (API stays up during compilation); on Windows the old
  process must be killed first because of the exe file lock.

### Safety

A non-loopback `proxy_addr` prints a warning (see
[Configuration](03-configuration.md#addresses-and-safety)).

## `axctl build`

Production build: embeds the frontend into the backend release binary.

```
axctl build [--dir <frontend-root>]
```

Flow: locate the frontend root → `vite build` (`dist/`) → write
`dist/.axctl-sentinel` (dist fingerprint) → `cargo build --release -p <backend> --bin <bin>`.

The sentinel makes cargo notice the `dist/` change, re-runs `build.rs`, recompiles
the main crate, and lets `frontend!` re-read the new dist. Your server must declare
the contract in `build.rs` (see [Design notes · Sentinel](05-design-notes.md#sentinel-frontend-freshness)).

## `axctl serve`

Static production preview, wrapping `vite preview`. **No backend, no API proxy.**

```
axctl serve [--dir <frontend-root>] [--addr 127.0.0.1:4173] [--open]
```

| Option   | Default          | Description                              |
| -------- | ---------------- | ---------------------------------------- |
| `--dir`  | current dir      | Frontend root (with `package.json`)      |
| `--addr` | `127.0.0.1:4173` | Listen address                           |
| `--open` | `false`          | Open the browser after start             |

Readiness probing: HTTP `GET /` + a per-round child-exit check + a 300 ms
stabilization — so a "port squatter" is not mistaken for ready. `--strictPort`
makes a taken port fail loudly; `--no-install` fails immediately if Vite is absent
locally (instead of downloading it).

## `axctl package`

The full build pipeline + `cargo-packager` installers.

```
axctl package [--dir <frontend-root>] [--format <FMT>]... [--package <PKG>] [--out-dir <DIR>]
```

| Option      | Description                                                                |
| ----------- | -------------------------------------------------------------------------- |
| `--format`  | Override package format(s) (`nsis` / `wix` / `dmg` / `deb` / `appimage` ...); repeatable; when omitted, cargo-packager decides from config |
| `--package` | Backend package to package (defaults to the resolved backend)              |
| `--out-dir` | Output directory for installers (passed to cargo-packager)                 |

Flow: reuse `build`'s frontend pipeline (vite build + sentinel) → `cargo build
--release` → run `cargo packager --release` in the backend member directory →
scan and list artifacts.

Packager config lives in the backend crate's `[package.metadata.packager]` (or a
`Packager.toml`), read natively by cargo-packager. **Do not set
`before-packaging-command`** — axctl already builds the release beforehand, so it
would be redundant.

## `axctl info`

Environment diagnostics: Rust / Node / package manager / platform.

## `debug-ws` (hidden)

Prints the resolved workspace and the computed WatchSet, to verify the
`cargo_metadata` path against real projects. Hidden from `--help`.
