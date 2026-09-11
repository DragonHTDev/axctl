# 06 · Development

## Requirements

- Rust (stable, with `rustfmt` + `clippy` components)
- Node.js + pnpm (only for integration tests / fixtures)

## Common commands

```bash
# build / run
cargo build
cargo run -p axctl -- --help
cargo run -p axctl -- info

# quality gates (must be green before committing)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# packaging manifest check (before a release)
cargo package -p axctl --list
cargo package -p axctl-core --list
```

## Language rule

| Scope                                                                        | Language |
| ---------------------------------------------------------------------------- | -------- |
| User-visible: CLI output, `--help`, README, docs.rs public comments, logs     | English  |
| Internal: CLI crate comments, git commit messages, this handbook's `zh-CN` pages | Chinese |

Details:

- `crates/axctl` (binary) does not reach docs.rs, so its `//!` / `///` are **Chinese**.
- `crates/axctl-core` (library): its **public** doc comments (`//!` / `///`) are
  **English**; its internal process comments (`//`) are **Chinese**.
- Commit messages are Chinese, style `type(scope): description`.

## Testing

### Unit tests

Tests live next to the code (`#[cfg(test)] mod tests`). Pure data modules
(`workspace.rs` / `watch_set.rs` / `config.rs`) should cover their branches.

### Fixtures (`tests/fixtures/`)

| Fixture       | Purpose                                                                    |
| ------------- | -------------------------------------------------------------------------- |
| `minimal-app` | Vite + axum end-to-end: `dev` / `serve` / `build` / `package` + sentinel contract |
| `embed-app`   | Pure Rust: `frontend!` embedding + `spa` serving (no Vite)                  |

`minimal-app` is a **standalone workspace** (its own `[workspace]`) and does not
take part in the main workspace build. It contains a `[package.metadata.packager]`
section and a `build.rs` (sentinel contract + winres Windows file info).

### Black-box smoke tests (`crates/axctl/tests/`)

`dev_smoke` / `preview_smoke` / `build_smoke` are all `#[ignore]`; they need Node +
Vite with the fixture frontend already built:

```bash
# build the fixture frontend first
cd tests/fixtures/minimal-app && pnpm exec vite build && cd ../../..

cargo test -p axctl --test build_smoke -- --ignored --nocapture
```

These are not part of the regular CI (they need a Node environment).

## CI

`.github/workflows/ci.yml` runs on push / PR:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo package -p axctl --list` and `cargo package -p axctl-core --list`

## Commits and review

- One logical change per commit, with a concise Chinese message.
- Run the three quality gates locally before committing.
- Release process is documented separately in the repo (to be added).
