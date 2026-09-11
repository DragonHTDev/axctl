# axctl-core

Core runtime library for [`axctl`](https://crates.io/crates/axctl): embed a
Vite build output into your axum server binary and serve it with SPA fallback.

## What it provides

- **`frontend!` macro** — embed `dist/` at compile time in release builds;
  in debug builds it yields an empty wrapper, so your local dev server
  (Vite) keeps serving the frontend.
- **`serve::spa`** — mount embedded assets as the fallback of your axum
  router, with mime / cache headers / ETag and SPA client-route fallback.
- **`probe`** — environment probing helpers (Rust / Node / package manager).

## Install

```toml
[dependencies]
axctl-core = "0.1"
include_dir = "0.7"   # required by the `frontend!` macro expansion
```

The `frontend!` macro expands to an `include_dir!` call, so **you** must also
depend on `include_dir` directly (macros expand in the caller's crate).

## Usage

```rust,ignore
use axctl_core::embed::FrontendAssets;
use axum::Router;

// release: embeds `dist/` at compile time; debug: empty wrapper.
let assets: FrontendAssets<'static> =
    axctl_core::frontend!("$CARGO_MANIFEST_DIR/../dist");

let app = Router::new()
    .nest("/api", api_routes)                          // your API first
    .fallback_service(axctl_core::serve::spa(assets)); // SPA fallback

// assets.get_file("index.html");
// assets.files();
// assets.is_embedded();
```

### Re-embedding after a frontend change

Cargo does not track `dist/` by default, so a rebuilt frontend would not
re-trigger compilation. Add a `build.rs` contract (axctl writes this sentinel
after every frontend build):

```rust
fn main() {
    println!("cargo:rerun-if-changed=dist/.axctl-sentinel");
    println!("cargo:rerun-if-changed=dist");
}
```

## SPA routing behavior

- real embedded files are served with cache headers / ETag;
- `/` and extension-less paths fall back to `index.html` (client-side routes);
- extension-bearing paths that do not exist return a real 404, so `fetch()`
  never receives HTML for a missing asset.

**Known boundary:** the fallback decision uses a path-extension heuristic
(not the `Accept` header). A valid SPA route that contains a dot (e.g.
`/order/2024.12`) is treated as a missing asset and returns 404. Keep such
routes dot-free, or mount them explicitly in your own router.

## Documentation

See the [handbook](https://github.com/dragonhtdev/axctl/blob/master/docs/README.md)
for architecture, configuration, and development guides.

## License

Licensed under either of [Apache-2.0](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-APACHE)
or [MIT](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-MIT) at your option.
