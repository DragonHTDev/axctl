# 03 · Configuration

axctl is configured through the **metadata table in your project's
`Cargo.toml`** — no extra config file.

## Two layers

Configuration can live in the workspace root and/or in a member; the two layers
are merged per field:

```toml
# workspace root Cargo.toml (the main config for virtual-manifest projects)
[workspace.metadata.axctl.dev]
backend_package = "my-server"
backend_port = 3001
proxy_addr = "127.0.0.1:3000"
frontend_dev_url = "http://127.0.0.1:5173"
frontend_root = "."

# inside a member (e.g. server/Cargo.toml) — overrides only the listed fields
[package.metadata.axctl.dev]
backend_port = 3999
```

> Why the workspace layer is required: a real project's root `Cargo.toml` is often
> a **virtual manifest** (only `[workspace]`, no `[package]`). If axctl only read
> `[package.metadata.axctl]`, such projects would have no usable config at all.

### Precedence (highest first)

```
package.metadata.axctl.dev     # current member, nested `.dev`
package.metadata.axctl         # current member, flat fields
workspace.metadata.axctl.dev   # workspace root, nested `.dev`
workspace.metadata.axctl       # workspace root, flat fields
defaults
```

Fields missing in a lower layer are filled from a higher one (per-field merge).

## Fields

| Field                    | Type       | Default          | Description                                                                 |
| ------------------------ | ---------- | ---------------- | --------------------------------------------------------------------------- |
| `backend_port`           | `u16`      | `3001`           | Backend dev listen port (must avoid the proxy port)                          |
| `frontend_dev_url`       | `string`   | none             | Already-running Vite URL (the tauri `devUrl`); when set, reuse it            |
| `proxy_addr`             | `string`   | `127.0.0.1:3000` | Dev proxy listen address (the single user entry)                             |
| `frontend_root`          | `string`   | workspace root   | Frontend directory (relative to the workspace root)                          |
| `frontend_build_command` | `string`   | pnpm / npm probe | Frontend build command                                                       |
| `backend_command`        | `string`   | none             | Compat form: parse `-p` / `--bin` to locate the backend member               |
| `backend_package`        | `string`   | see below        | Backend package name (required for multi-binary workspaces)                  |
| `extra_watch_dirs`       | `string[]` | `[]`             | Extra dirs to watch (relative to workspace root); **any change restarts**; layers are concatenated & deduped |

## Backend member resolution

To decide "which crate is the backend to watch / restart / package", axctl uses
this precedence (`backend.rs`):

1. the `backend_package` config
2. `-p` / `--package` in `backend_command` (or `--bin`, resolved via `package_for_bin`)
3. the member containing the cwd
4. the workspace's **only** binary package

If all fail (multiple binaries, nothing configured explicitly), axctl **errors out**
and asks you to configure `backend_package` — it never silently guesses. When a
multi-binary workspace resolves without an explicit config, it also prints a note
telling you which crate was selected.

> Startup is "`cargo build -p <pkg> --bin <bin>` first, then spawn the artifact";
> axctl does **not** run `backend_command` directly. That field is only used to
> locate the member.

## Addresses and safety

- Listen addresses go through `parse_addr`, which validates the host against a
  character allow-list (rejects shell metacharacters to prevent injection, since
  the host may be interpolated into a command string).
- When `proxy_addr` / serve's `--addr` is **non-loopback** (e.g. `0.0.0.0`), axctl
  prints a warning: it exposes the unauthenticated backend and the Vite dev server
  to the network (Vite's `@fs` / HMR had historic RCEs such as CVE-2025-30221).
  Use it only in a trusted local dev environment.
