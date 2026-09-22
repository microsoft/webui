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
- `application-ipc`: typed application IPC, sessions, workers, and browser assets.
- `source`: source compilation, bundle construction, and packaging APIs.
- `cli`: the `webui-desktop` tooling binary, including `native` and `source`.

Apps normally enable `native` and opt into `source` only during development.
`webui desktop package` selects an optimized, runtime-only app build by default.
The default library supports headless bundle rendering and custom backends
without requiring native SDKs or the template compiler.
Application IPC is opt-in: neither `native`, `source`, nor `cli` enables it.
Enable `application-ipc` alongside `native` when using generated Rust bindings.
Without it, no application IPC sessions, workers, embedded assets, or native
transport handlers are installed. Window controls and lifecycle events remain
available independently.

Packaging supports macOS `.app` bundles and Windows/Linux portable directories,
not installers, archives, or signing. Shell configuration exposes app icons,
menus, and tray icons, subject to backend capabilities.

Application IPC uses `ipc::IpcLimits::default()` with checked
`with_max_frame_bytes(...)` and `with_default_timeout(...)` policy builders.
The SDK owns queue, callback, control-reservation, and aggregate memory budgets;
applications cannot tune these independently.

See the [desktop guide](https://microsoft.github.io/webui/guide/integrations/desktop)
for application setup and customization.
