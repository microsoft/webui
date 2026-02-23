# WebUI Tiny HTTP Integration Example

This example combines a WebUI app's template and state JSON using the WebUI Rust crates,
writes the result to `dist/index.html`, and serves it over HTTP at `http://127.0.0.1:8080/` with simple HMR.

## Prerequisites

- Rust toolchain (the repo uses Rust 2021 and pins the toolchain in `rust-toolchain.toml` at the repo root).

## Running the server

From this folder, run with the default `hello-world` app:

```bash
cd examples/integration/tiny_http
cargo run
```

Or specify a different app by name:

```bash
cargo run -- --app hello-world
```

The `--app` argument selects a folder under `examples/app/`. Any folder with the same structure works:

```
examples/app/<name>/
├── templates/index.html
├── data/state.json
└── assets/
```

This will:

1. Read `<app>/templates/index.html` as the WebUI template.
2. Load state from `<app>/data/state.json`.
3. Parse the template into a WebUI protocol using `webui-parser`.
4. Render the protocol with the state using `webui-handler` and write the result to `dist/index.html`.
5. Start an HTTP server on `http://127.0.0.1:8080/` that:
	- Serves `dist/index.html` for `/` and `/index.html`.
	- Serves files from `<app>/assets/` via `/assets/*` routes (e.g., `/assets/app.js`, `/assets/styles.css`).
	- Exposes `/hmr` for hot module reloading.
6. Watch all files in `<app>/templates/`, `<app>/data/`, and `<app>/assets/` directories for changes; when any file changes, re-render `dist/index.html`.

The client-side script in `<app>/assets/app.js` polls `/hmr` and automatically reloads the page when a new version is detected.

## Notes

- This crate is **not** part of the main Rust workspace; it is a standalone Cargo project under `examples/integration/tiny_http`.
- Always run `cargo run` from the `examples/integration/tiny_http` directory so the program can find the app directories relative to the current working directory.
