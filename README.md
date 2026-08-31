# axctl

An all-in-one development tool for axum + Vite projects, inspired by the
tauri-cli developer experience.

## Project layout

```
crates/
├── axctl/         # CLI binary (clap + command orchestration)
└── axctl-core/    # Library for user projects (embed / serve / probe)
docs/design.md     # Architecture and design decisions
```

## Subcommands

| Command   | Description                                          |
| --------- | ---------------------------------------------------- |
| `init`    | Initialize a project (detect environment, generate config) |
| `dev`     | Development mode: watch source changes, rebuild and restart the server automatically |
| `build`   | Production build: build frontend assets + cargo release build |
| `serve`   | Static preview: serve built assets directly           |
| `package` | Package: generate installers via cargo-packager      |
| `info`    | Environment diagnostics: print Rust / frontend / system info |

## Development

```bash
cargo run -p axctl -- info    # run axctl
pnpm axctl -- info            # or run via the pnpm wrapper
cargo test                    # run tests
```

## Documentation

See [docs/design.md](docs/design.md) for architecture decisions and design notes.
