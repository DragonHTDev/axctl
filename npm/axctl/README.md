# axctl

All-in-one development tool for **axum + Vite** projects (dev / build / serve /
package / info), distributed for Node.js.

## Install

```sh
npm install --save-dev axctl
# or run without installing
npx axctl --help
```

This package ships a small launcher; the actual `axctl` binary comes from a
per-platform package installed through `optionalDependencies`.

## Usage

```sh
npx axctl dev       # start Vite + backend + reverse proxy
npx axctl build     # production build (frontend embedded into the backend)
npx axctl package   # native installers via cargo-packager
npx axctl info      # environment diagnostics
```

Run any command from your project root. Configuration lives in your `Cargo.toml`
(`[workspace.metadata.axctl.dev]`) — see the handbook.

## Documentation

See the [handbook](https://github.com/dragonhtdev/axctl/blob/master/docs/README.md).

## License

Licensed under either of
[Apache-2.0](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-APACHE)
or [MIT](https://github.com/dragonhtdev/axctl/blob/master/LICENSE-MIT) at your
option.
