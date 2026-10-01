# Tauri

The Tauri example opens a pre-built WebUI app in a native window using Rust,
without Node or an HTTP server. It loads `protocol.bin` and JSON state,
renders once, and serves the page and browser assets through a custom protocol.

Install [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/)
and the repository's pnpm dependencies, then run from the repository root:

```bash
pnpm --filter hello-world-example build
cargo run --manifest-path examples/integration/tauri/Cargo.toml --locked --release -- \
  examples/app/hello-world/dist \
  examples/app/hello-world/data/state.json
```

The two paths select the build directory and initial state file.
`--plugin=webui` enables browser hydration for apps built with that plugin;
omit it for plain HTML. Supply theme tokens in the state if the app uses them.
No Tauri CLI or JavaScript Tauri package is required.

Like the [Electron example](./electron), this is a read-only snapshot host,
not a complete application backend. Client-side interactions work; server
routing, API mutations and persistence require your own handlers. Only `/`
serves the rendered page.

Use trusted builds and relative asset URLs. The origin is `webui://app` on
macOS/Linux and `https://webui.app` on Windows. The example uses an ephemeral
profile, blocks foreign navigation and popups, and grants no native invoke
capabilities. Close the window to exit.

See [`examples/integration/tauri`](https://github.com/microsoft/webui/tree/main/examples/integration/tauri)
for the host and local verification commands.
