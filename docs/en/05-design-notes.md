# 05 · Design notes

Key decisions and pitfalls. Each one corresponds to a naive approach that looks
fine but breaks in practice.

## HMR must go through a WebSocket tunnel

**Symptom**: when Vite's `hmr` port is not pinned, the HMR client connects to the
**page's load port** (the proxy port 3000). If the proxy does not forward
WebSockets, HMR breaks and every frontend edit triggers a full page reload.

**Details that were easy to miss** (missing them caused handshake failures):

1. **Subprotocol echo**: the Vite HMR server only accepts upgrades carrying
   `Sec-WebSocket-Protocol: vite-hmr`. RFC 6455 requires that when a client offers
   a subprotocol, the server picks one in the response; otherwise the browser
   considers the handshake failed. So the browser↔proxy leg uses
   `requested_protocols()` + `protocols()` to make axum select one, and the
   proxy↔vite leg sends the subprotocol itself.
2. **path + query passthrough**: Vite authenticates with `?token=`, so the proxy
   must preserve the original path and query when connecting to Vite.

Conclusion: a WS tunnel is a hard requirement, and it must faithfully pass through
the subprotocol and path/query.

## Sentinel (frontend freshness)

**Symptom**: cargo's incremental build does not notice changes in `dist/`; after
`frontend!` reads dist once, cargo thinks "nothing changed" and does not recompile
— embedding a stale frontend.

**Fix**: `axctl build` writes `dist/.axctl-sentinel` after `vite build` and before
`cargo release`, with content = the **dist fingerprint** (a hash of file names +
sizes + mtimes; it changes on every build). Your server's `build.rs` declares the
contract:

```rust
fn main() {
    println!("cargo:rerun-if-changed=dist/.axctl-sentinel");
    println!("cargo:rerun-if-changed=dist"); // optional fallback
}
```

Sentinel changes → `build.rs` re-runs → the main crate recompiles → `frontend!`
re-reads dist.

> We do not use a "write only if content changed" strategy: the sentinel's job is to
> make cargo notice that a build happened. If identical content were skipped, cargo
> might skip recompilation. The fingerprint includes mtime so it always changes.

## Process model: build first, then spawn

**Why not `cargo run`**: cargo would be the parent and the server its child.
Killing cargo orphans the server, which keeps the port; the next restart fails to
bind. Also the API is down for the whole compile (old process killed, new one not
started).

**Fix**: `cargo build -p <pkg> --bin <bin>` until success → spawn the artifact
(`target/debug/<bin>`, a **leaf process**) → killing the root process is enough.

**Restart semantics (platform differences)**:

- Unix: build first, then kill the old process — the old backend keeps serving
  during compilation, and a failed build leaves it running.
- Windows: kill first, then build — Windows locks a running exe, so cargo cannot
  overwrite the artifact otherwise. Windows therefore gives up "API available
  during compilation".

**Process-tree kill**: Unix recursively collects descendants with `pgrep -P`, then
`SIGTERM`/`SIGKILL`; Windows uses `taskkill /t /f`. The backend is a leaf, but Vite
(pnpm → node) is a tree and needs a tree kill.

## Watch scope: workspace-aware

**Why not watch the whole project root**:

- On Linux, `inotify` uses one watch descriptor per directory. Recursively watching
  the whole root registers `node_modules` / `target` / `.git` too, blowing past
  `fs.inotify.max_user_watches` (default 8192) → watch fails or silently drops events.
- Semantics: editing `src-tauri` (another binary) should not restart the backend.

**Fix**: `watch_set.rs` computes the backend's **path dependency closure** (BFS,
only within the workspace) and registers only, per member in the closure: `src/`
(recursive, filtered to Rust sources), `Cargo.toml`, and `build.rs`. `extra_watch_dirs`
become **trigger-all** directories (any file change triggers, no `.rs` filter).

Dependencies come from `cargo metadata`'s **resolve.nodes** (the resolved graph),
not the Cargo.toml declarations — so renamed dependencies resolve correctly, and
only normal + build deps are counted (dev-dependencies are excluded).

## Embedding: include_dir, not rust-embed

`axctl-core`'s `frontend!` macro is built on `include_dir`. It was chosen over
`rust-embed` because we need a **value** (`let assets = macro!(...)`) that can be
passed to the serve layer; rust-embed's derive form attaches assets to a type and
only supports static calls.

Constraint: `include_dir!` expands to bare `include_dir::` references, so the
**caller crate must depend on `include_dir = "0.7"` directly**.

Release embeds; debug yields an empty wrapper — in debug nothing is embedded or
read from disk, since the dev frontend is served by Vite.

## SPA fallback: extension heuristic

`spa()` decides the fallback by **path extension** (no extension = SPA client route
→ `index.html`; extension present and missing = real 404), not by the `Accept` header.

Rationale: curl / test tools send `Accept: */*` by default yet still hit SPA routes;
and a missing `fetch()` for an asset with an extension still gets a 404, never HTML.

**Known boundary (do not file as a bug)**: a **valid SPA route** with an extension
returns 404 (e.g. `/order/2024.12`). Dot-bearing client routes must be handled
otherwise: make them dot-free, or register them explicitly in your own router.

Cache header tiers: HTML `no-store`; under `assets/` 1 year `immutable` (Vite
content hash); everything else `public, no-cache`; ETag (content hash) +
`If-None-Match` → 304.

## Unified log pipeline

- tracing is the **only** channel; the default display matches tauri-cli's minimal
  style, and `RUST_LOG` reveals detail.
- stdout is reserved for data output; logs always go to **stderr**.
- Two modes: default (unset / `info` / `warn`) is the concise mode (`Info` / `Done` /
  `Warn` / `Error` prefixes); `debug` / `trace` enables the verbose mode
  (`LEVEL target: message key=value`).
- target convention: `axctl` (UI) / `axctl.dev` / `axctl.proxy` / `axctl.process` /
  `axctl.watcher` / `axctl.config`. Targeted filtering works:
  `RUST_LOG=axctl.proxy=debug`.
- level → prefix: ERROR→`Error`, WARN→`Warn`, INFO+`ui.success`→`Done`, INFO→`Info`.

## Known limitations

- **SPA extension heuristic**: dot-bearing valid client routes return 404 (above).
- **Windows cannot keep the API up during compilation**: the exe lock forces kill-first.
- **serve needs a local Vite**: `node_modules` must be ready, otherwise it errors.
- **`init` is not implemented**.
