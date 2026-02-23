# WebUI Hyper Integration Example

A performance-focused WebUI integration example using [hyper](https://hyper.rs/) — Rust's fast, correct HTTP implementation.

This example combines a WebUI app's template and state JSON using the WebUI Rust crates,
writes the result to `dist/index.html`, and serves it over HTTP at `http://127.0.0.1:8080/` with simple HMR.

## Performance highlights

- **HTTP/2 only** — multiplexed streams, header compression, and binary framing via `hyper-util`'s `http2::Builder`
- **hyper 1.x** — zero-copy HTTP parsing with minimal allocations
- **`Bytes` bodies** — reference-counted buffers avoid copying response data
- **`Full<Bytes>`** — lightweight body type with no boxing overhead
- **Async I/O** — tokio-backed non-blocking networking for high concurrency
- **Static content types** — `&'static str` references for headers, no allocation per request
- **Binary-safe asset reads** — `fs::read` instead of `read_to_string` for correct handling of all file types

## Prerequisites

- Rust toolchain (the repo uses Rust 2021 and pins the toolchain in `rust-toolchain.toml` at the repo root).

## Running the server

From this folder, run with the default `hello-world` app:

```bash
cd examples/integration/hyper
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
5. Start an async HTTP server on `http://127.0.0.1:8080/` that:
	- Serves `dist/index.html` for `/` and `/index.html`.
	- Serves files from `<app>/assets/` via `/assets/*` routes (e.g., `/assets/app.js`, `/assets/styles.css`).
	- Exposes `/hmr` for hot module reloading.
6. Watch all files in `<app>/templates/`, `<app>/data/`, and `<app>/assets/` directories for changes; when any file changes, re-render `dist/index.html`.

The client-side script in `<app>/assets/app.js` polls `/hmr` and automatically reloads the page when a new version is detected.

## Why not HTTP/3?

HTTP/3 runs over QUIC (UDP) and requires TLS — there is no plaintext mode, even on localhost. This makes it impractical for a simple dev server example where ease of use matters.

hyper does not yet support HTTP/3 natively. It is tracked on their roadmap:

- [hyperium/hyper#1818](https://github.com/hyperium/hyper/issues/1818) — HTTP/3 tracking issue
- [hyper roadmap 2025](https://seanmonstar.com/blog/hyper-roadmap-2025/) — HTTP/3 listed as a priority

HTTP/3 in Rust today requires a separate stack (`h3` + `h3-quinn` + `quinn`) with its own API, mandatory TLS certificates, and UDP transport. Once hyper ships native HTTP/3 support, this example can be updated to auto-negotiate HTTP/1.1, HTTP/2, and HTTP/3.

## Notes

- This crate is **not** part of the main Rust workspace; it is a standalone Cargo project under `examples/integration/hyper`.
- Always run `cargo run` from the `examples/integration/hyper` directory so the program can find the app directories relative to the current working directory.
