# WebUI Tauri Integration

Open a pre-built WebUI app in a Tauri window, without Node or an HTTP server.
Like the Electron example, this loads `protocol.bin` and JSON state, renders
once, and serves the resulting HTML and browser assets through `webui://`.

## Run

Install [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/).
No Tauri CLI or JavaScript Tauri package is needed.

From the repository root:

```bash
pnpm --filter hello-world-example build
cargo run --manifest-path examples/integration/tauri/Cargo.toml --locked --release -- \
  examples/app/hello-world/dist \
  examples/app/hello-world/data/state.json \
  --theme=@microsoft/webui-examples-theme
```

Pass another app's build directory and state file to open it instead.
Add `--plugin=webui` for apps compiled with the WebUI hydration plugin.
Use `--theme=<file-or-package>` to resolve and inject theme tokens at startup.
File paths resolve from the working directory; npm packages resolve from the
state file's directory. Omit it to preserve token CSS already in the state.

This is a **read-only snapshot**, not an application backend. Client-side
interactions work, but server-backed routing, mutations and persistence
require application-owned handlers. Only `/` serves the rendered page.
Close the window to exit.

Use trusted builds and relative asset URLs. The host confines browser assets
to the build directory, limits input files to 32 MiB, blocks foreign navigation
and popups, and grants no native invoke capabilities. Its browser profile is
ephemeral. Do not modify the build directory during a run.

## Verify

```bash
cargo test --manifest-path examples/integration/tauri/Cargo.toml --locked --no-default-features
cargo fmt --manifest-path examples/integration/tauri/Cargo.toml --check
cargo clippy --manifest-path examples/integration/tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo deny --locked --manifest-path examples/integration/tauri/Cargo.toml --config examples/integration/tauri/deny.toml check
```

The standard examples gate runs the headless tests; native builds remain
opt-in. The existing PR lint job also audits this isolated dependency graph.
This example has its own Cargo workspace and lockfile because Tauri
uses GTK3 on Linux while the WebUI desktop SDK uses GTK4. Its audit policy
keeps Tauri-specific dependency exceptions out of the SDK workspace.
