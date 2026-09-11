# 01 · Overview

axctl is an all-in-one development tool for **axum + Vite** projects, built for a
tauri-cli-like developer experience: one command to bring up the whole development
environment, one command to produce a distributable artifact.

## What it solves

A typical axum + Vite workflow relies on hand-written Node glue scripts: start
Vite, build the backend, run a proxy, watch-and-restart, package artifacts. Such
scripts get scattered, are hard to test, and accumulate platform quirks.

axctl folds all of it into a single Rust CLI:

| Command         | What it does                                                                                                     |
| --------------- | ---------------------------------------------------------------------------------------------------------------- |
| `axctl dev`     | Start Vite, build & run the backend, expose one unified entry (reverse proxy for HTTP + WebSocket/HMR), rebuild and restart on source changes |
| `axctl build`   | `vite build` → write the dist sentinel → `cargo build --release` (backend embeds the fresh frontend at compile time) |
| `axctl serve`   | Preview the built frontend via `vite preview` (static, no backend)                                                |
| `axctl package` | The full build pipeline + `cargo-packager` to produce native installers                                           |
| `axctl info`    | Environment diagnostics (Rust / Node / package manager)                                                           |

`init` is not implemented yet.

## Two crates

axctl is a workspace of two crates:

| Crate        | Kind          | Responsibility                                                         |
| ------------ | ------------- | ---------------------------------------------------------------------- |
| `axctl-rs`      | CLI binary    | Command orchestration (dev / build / serve / package / info)           |
| `axctl-core` | library       | For **user projects**: compile-time frontend embedding (`frontend!`) + SPA serving (`spa`) |

Why `axctl-core` is a library rather than part of the CLI: embedding the built
frontend into a server binary must happen when the **user project** is compiled
(the macro expands in the caller's crate), so it naturally lives as a library.
The CLI only orchestrates; it never compiles the user's project.

## How it differs from tauri-cli

In tauri, the frontend talks to the backend over webview IPC — **not HTTP** — so no
proxy is needed and it only knows about `devUrl`. In axctl, the frontend talks to
the backend over **HTTP** (browser or webview), so axctl uses a **reverse proxy**
with a single entry port (default `127.0.0.1:3000`). This is still zero-intrusion
to `vite.config`, and it keeps the tauri `devUrl` mental model (`frontend_dev_url`).

## Next

- [Architecture](02-architecture.md) — repo layout and module boundaries
- [Configuration](03-configuration.md) — Cargo.toml metadata
- [Commands](04-commands.md) — full behavior of each command
- [Design notes](05-design-notes.md) — key decisions and pitfalls
- [Development](06-development.md) — local dev / test / commit
