# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-09-10

### Added

#### `axctl` (CLI)

- `axctl dev` — development mode for axum + Vite projects: starts Vite, builds
  and runs the backend, and exposes a unified entry port with a reverse proxy
  (HTTP + WebSocket/HMR tunneling). Workspace-aware source watching rebuilds
  and restarts only the backend dependency closure; a three-state machine
  keeps changes that arrive during a rebuild from being lost.
- `axctl build` — production build: `vite build` → write the `dist` sentinel →
  `cargo build --release`, so the backend re-embeds the fresh frontend.
- `axctl serve` — preview the built frontend via `vite preview` (static, no
  backend).
- `axctl package` — the full build pipeline followed by `cargo-packager` to
  produce native installers, with packager-config preflight and artifact
  reporting.
- `axctl info` — environment diagnostics (Rust / Node / package manager).
- Double-layer configuration via `[workspace.metadata.axctl.dev]` /
  `[package.metadata.axctl.dev]`, merged per field.
- Backend target resolution with a multi-binary workspace warning
  (`backend_choice_hint`).

#### `axctl-core` (library)

- `frontend!` macro — embed a Vite `dist/` at compile time in release builds;
  empty wrapper in debug builds.
- `embed::FrontendAssets` — access embedded files (`get_file` / `files` /
  `is_embedded`).
- `serve::spa` — mount embedded assets as an axum fallback with mime / cache
  headers / ETag and SPA client-route fallback.
- `probe` — environment probing helpers.

#### Tooling

- `rustfmt.toml`, workspace release profile (`opt-level = "z"`, LTO).
- Fixtures: `minimal-app` (Vite + axum end-to-end) and `embed-app` (pure Rust
  embedding), plus ignored black-box smoke tests for `dev` / `preview` /
  `build`.

[Unreleased]: https://github.com/dragonhtdev/axctl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/dragonhtdev/axctl/releases/tag/v0.1.0
