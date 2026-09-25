# microsoft-webui-desktop

Native desktop SDK for WebUI applications.

This crate owns the runtime-neutral pieces: desktop bundle metadata, safe
custom-protocol routing, startup rendering, bounded asset reads, and protobuf
IPC dispatch, application construction, and native webview backends.
Applications use one `webui_desktop` API on macOS, Windows, and Linux.

Native Windows builds use Windows App SDK 1.8.11 for system-owned overlay
caption buttons and WebView2 for application content; no XAML or web-rendered
caption controls are required. Install the matching-architecture Windows App
Runtime 1.8 (8000.946.1701.0 or newer 1.8 servicing version), the applicable
Visual C++ Redistributable, and WebView2. Other runtime families are not a
substitute. See [Microsoft's runtime downloads](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads).

The build stages the small bootstrap DLL and its license notices beside native
Windows executables. Windows portable packaging preserves these companions,
including for custom runners; redistribute the directory and install the
shared runtime separately. This is framework-dependent deployment, not a
self-contained Windows App Runtime bundle. Non-Windows and headless builds
do not use these assets.

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

See the [desktop guide](https://microsoft.github.io/webui/guide/concepts/desktop)
for application setup and customization.

## Regenerating the Windows projection

Maintainers generate the private Rust projection from the official
`Microsoft.WindowsAppSDK.InteractiveExperiences` NuGet package
`1.8.260708001`, using its `metadata/10.0.17763.0` directory. The archive SHA-256
is `496eea92d353b5d3601b67353f06dcadd6d2d9b635575acebe6e42587dbfad76`.
From the workspace root, run:

```powershell
cargo run -p xtask --features windows-app-sdk-tools -- windows-app-sdk-bindings <metadata-directory>
cargo fmt --all
```

The generator verifies each metadata input against its pinned hash and uses
Microsoft's `windows-bindgen` version compatible with the workspace Windows
Rust crates. Unused APIs without projected dependencies are reported and
omitted. Consumers compile the checked-in projection, without a generator,
NuGet restore, or network access during Cargo builds.
