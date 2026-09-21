# microsoft-webui-desktop

Native desktop SDK for WebUI applications.

This crate owns the runtime-neutral pieces: desktop bundle metadata, safe
custom-protocol routing, startup rendering, bounded asset reads, and protobuf
IPC dispatch, application construction, and native webview backends.
Applications use one `webui_desktop` API on macOS, Windows, and Linux.

Rust-first packaged apps should load bundles with
`DesktopRuntime::from_bundle_config` so the executable can keep route providers
and typed IPC handlers in Rust while reusing immutable `protocol.bin` and asset
files from the bundle.

There are no default features:

- `native`: the current platform's native window and system webview.
- `source`: source compilation, bundle construction, and packaging APIs.
- `cli`: the `webui-desktop` tooling binary, including `native` and `source`.

Apps normally enable `native` and opt into `source` only during development.
`webui desktop package` selects an optimized, runtime-only app build by default.
The default library supports headless bundle rendering and custom backends
without requiring native SDKs or the template compiler.

See the [desktop guide](https://microsoft.github.io/webui/guide/integrations/desktop)
for application setup and customization.
