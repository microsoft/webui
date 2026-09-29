# Desktop Apps

WebUI desktop apps render the same templates as browser apps in a native window.
They use WebView2 on Windows, WKWebView on macOS, and GTK4/WebKitGTK 6 on Linux.
Bundled/source apps need no Electron, bundled browser, Node runtime, or
localhost server. An opt-in Rust host can instead load its existing loopback
HTTP server directly in the same native window.

## Get started

For an npm project, opt in to desktop support with matching package versions:

```bash
npm install @microsoft/webui @microsoft/webui-desktop
```

The desktop package installs the native sidecar for your platform without
adding webview dependencies to the base `@microsoft/webui` package. Do not
disable optional dependencies. A Rust CLI installation can instead use a
matching `webui-desktop` executable on `PATH`.

Run an existing app without writing a Rust host:

```bash
webui desktop run ./src
```

`run` builds from source and opens a window. Restart it after changes; desktop
`--watch` is not supported. To scaffold an app with a Rust runner:

```bash
webui desktop init ./my-app
cd ./my-app
cargo run --manifest-path desktop/Cargo.toml --features source
```

Use a Rust host when you need route state, API handlers, typed IPC, or native
window control. The generated runner loads packaged resources by default;
`source` is an opt-in development feature. `desktop init` does not overwrite
files unless you pass `--force`. It generates `webui-desktop.json`; both its
Rust source runner and `webui desktop package ./my-app --out ./packages` read
the same identity and window settings from this file. For an existing app,
set desktop package options with command flags or the app-root
`webui-desktop.json`, not package.json. See the
[desktop CLI reference](/guide/cli/#webui-desktop) for fields and precedence.

| SDK feature | What it adds |
| --- | --- |
| Default (none) | Bundles, rendering, route/API handlers, custom backends |
| `native` | System webview and `run_frame` |
| `local-server` | Native window for an existing loopback HTTP origin on macOS, Windows or Linux (includes `native`; no source compiler or implicit IPC grant) |
| `native-services` | Trusted Rust-host browser/document OS openers on a local-server window (includes `local-server`; no implicit renderer grant) |
| `source` | Build from templates with `DesktopSourceConfig` |
| `application-ipc` | Typed application messages |
| `cli` | Desktop CLI sidecar; not needed in an app runner |

For a native runner:

```toml
[dependencies]
webui-desktop = { package = "microsoft-webui-desktop", version = "0.0.30", default-features = false, features = ["native"] }
serde_json = "1.0"

[features]
default = []
source = ["webui-desktop/source"]
```

Enable `application-ipc` separately if you use generated messages.

### Existing HTTP application

An application that already owns a loopback HTTP listener can use the opt-in
`local-server` feature on macOS, Windows or Linux. Pass the **bound** socket address;
keep that listener running until the window exits. The Rust host must verify
the identity of any externally owned server before creating a frame; an IP
address and port do not authenticate it. No source build, bundle loading,
response proxy, or second render occurs.

```rust
use webui_desktop::{DesktopApp, HostLifetime, LoopbackOrigin, LocalServerOptions, Result};
use std::net::SocketAddr;

fn show_existing_server(bound_address: SocketAddr) -> Result<()> {
    let origin = LoopbackOrigin::from_socket_addr(bound_address)?;
    let (owner, lifetime) = HostLifetime::new();
    let options = LocalServerOptions::new(origin, lifetime)
        .initial_path("/app?view=home")?;
    let frame = DesktopApp::from_local_server(options).build()?;
    let result = webui_desktop::run_local_server_frame(frame);
    drop(owner);
    result
}
```

The trusted host retains `HostLifetimeOwner` alongside its listener or verified
attached-daemon connection. Call `owner.revoke()` **before** releasing that
listener/connection; dropping the owner also revokes it. An owned host must
keep its listener bound until `WindowClosed`/`Exiting` because native close is
asynchronous and already-issued HTTP requests may still be in flight. A
revoked frame rejects new navigation, stops pending document loading and
schedules native window close without waiting for the window command queue.
Check the result of `owner.revoke()`: if native wake delivery fails, admission
remains retired but the window may stay open. Keep the listener bound, call
`owner.retry_close()` deliberately and wait for `WindowClosed`/`Exiting`;
a successful wake is not a close acknowledgement. Dropping an owner after a
wake failure retries once and logs any remaining failure.
Revoking an attached frame does not terminate the daemon. An attached daemon
can release its port before the desktop observes its loss, so this API is not
a network egress or port-rebind barrier. A bound IP address is not a daemon
identity check; the host must authenticate the daemon before constructing
this lifetime.
Use a separate lifetime per running native window.

Only an IP-literal loopback HTTP origin with an explicit nonzero port is
accepted (for example, `127.0.0.1` or `[::1]`; all bound IPv4 `127/8`
addresses are valid, but `localhost` and wildcard addresses are not).
Initial paths must start with a single `/`; top-level navigation outside that
exact origin is cancelled. macOS and Windows deny network-backed subframe
navigation by default. To load a local preview, retain the result of
the exact-origin grant while its iframe is in use:

```rust
let grant = frame.frame_policy().allow_unprivileged_origin(
    HttpFrameOrigin::from_localhost_subdomain(host, port)?,
)?;
```

Only that exact lowercase `.localhost` subdomain and port may load as a
subframe; dropping the grant prevents later document loads but does not retract
an in-flight request. The grant never promotes the preview to a top-level app
document or gives it native IPC or window controls. Linux currently rejects
these grants: WebKitGTK denies subframe document responses, but a request may
reach the HTTP server before that decision. Do not use GET requests with side
effects as a frame-isolation mechanism. Popups are denied, though a
browser-created `about:blank` child may still exist. Downloads and permissions
are not granted by this API; the behavior of sandbox-allowed preview downloads
still requires product qualification. Do not infer macOS 13 compatibility for
`.localhost` subdomains from results on newer systems.
Custom titlebar styles are rejected; use the native titlebar.
The browser fetches ordinary HTTP resources directly; this is not a network
egress sandbox. Page-originated native window controls are unavailable. Application IPC is
also disabled unless an owned macOS or Windows host explicitly calls
`LocalServerAppBuilder::application_ipc(&listener, registry, options)` before
`build()`, with its **retained bound TCP listener** and generated schema
registered through `IpcRegistry` and `IpcOptions::for_schema`. The builder
rejects a listener at another address or one that cannot exclude competing
same-port bindings. On Windows, use `bind_owned_local_server(address)` to
bind an exclusive `std::net::TcpListener` before passing it to the application's
server and this builder. An attached daemon cannot use this grant.
Import `createDesktopTransport` from `@microsoft/webui-desktop/native` for
that document; the ordinary package entry does not include the local native
carrier. If the host does not bundle that import, it can serve
`local_ipc_runtime_asset()` at `LOCAL_IPC_RUNTIME_PATH` on its existing
same-origin HTTP server. Native IPC credentials and frames do not go to
the HTTP server, and a missing carrier version does not fall back to HTTP.
Keep the original listener and `HostLifetimeOwner` alive while the window runs.
Let the native window close before waiting for server port quiescence;
`run_local_server_frame` returns only after the SDK releases its listener pin.
On macOS, ordinary window close, an explicit host close request, host-owner
revocation, and AppKit Quit all return control to the Rust host rather than
terminating the process. Quit (the default menu's Cmd+Q or `terminate:` action)
requests the window's normal, cancellable close. A host
`WindowCloseRequested` handler can return `PreventDefault`; a later Quit
retries. Owner revocation instead closes the window even if ordinary close
would be vetoed. Once `WindowClosed` and `Exiting` have been delivered and
native IPC has retired, `run_local_server_frame` returns so the owning Rust
host can shut down its backend and await worker/HTTP drain **off the UI
thread**.
If the native close cannot be queued or acknowledged within 15 seconds, the
SDK records an error and tries the ordinary cancellable AppKit close directly.
The Quit path does not return while a live window still owns the HTTP origin:
if that fallback is vetoed, keep the listener bound and close the window before
the recorded error is returned. This contract is for AppKit-driven Quit, not
forced OS termination or `SIGKILL`, which cannot run host cleanup.
If AppKit's event loop instead stops unexpectedly without closing the window,
the SDK closes that window on the UI thread before retiring IPC or returning
an error, including when an earlier Quit failed to queue a close and its
fallback was vetoed. This terminal recovery is not an ordinary cancellable
close request; the host still owns its backend shutdown after
`run_local_server_frame` returns.
Rust event callbacks and `window_handle()` remain available. Linux reports
an error if local-server application IPC is requested because WebKitGTK cannot
attribute native handler messages to a frame; unprivileged direct HTTP windows
and existing bundled/source apps still run there.
The current macOS webview uses an ephemeral website data store, even with a
stable app ID; this mode does not migrate or persist existing browser cookies.

### Trusted-host OS openers

An opt-in `native-services` Rust host can call `frame.native_services()?` to
obtain a `NativeServices` handle for its own window. This installs only a
Rust lifecycle subscription; it does **not** add a page global, a renderer
bridge, or an IPC grant. To expose a specific action to a page, authorize it
separately through an explicitly generated application IPC registry. Never
forward an arbitrary renderer-supplied file path to the document opener.

```rust
use webui_desktop::{NativeServiceError, NativeServices};

async fn open_help(services: &NativeServices) -> Result<(), NativeServiceError> {
    services.open_url("https://example.com/help")?.await
}
```

`open_url` parses an absolute HTTP(S) URL (maximum 2,048 bytes), requiring a
host and rejecting credentials, controls, and other schemes. `open_document`
accepts a trusted host-provided absolute local regular `.txt`, `.log`, `.md`,
`.csv`, `.json`, or `.pdf` path (maximum 4,096 bytes); it checks/canonicalizes
the file on a worker, rejects final symlinks and directories, and opens it
with the OS document handler. Do not pass sensitive files without explicit
host authorization. Both methods return
an awaitable `NativeOpen`: creating it means bounded worker admission, while
awaiting it reports the OS opener's response, failure, cancellation, or a
10-second deadline. It cannot guarantee that the external browser finished
loading or recall an OS launch already in progress. At most one open can be
in flight per window; even a navigation **request later prevented** by a host
handler cancels a pending OS open. Window close and host retirement also
cancel it. Do not synchronously wait for this future on the native UI
thread. No workers or timers start unless an opener is called.

Directory selection and native message/confirmation dialogs are **not**
provided by this host API yet.

On macOS, `services.content_geometry()?.await` reads a live `ContentGeometry`
snapshot on the AppKit main thread. Its `screen_rect` is the **WKWebView
content bounds**, converted through its window into global Cocoa screen
**points with a bottom-left origin**, not an `NSWindow` frame, CSS pixels, or
backing pixels. The snapshot also reports the native window generation,
main-document validity epoch, revision, backing scale, `WKWebView.pageZoom`, and
`WKWebView.magnification`. Windows and Linux return `Unsupported`; before a
main document finishes, macOS returns `GeometryUnavailable`. A prevented
navigation request does not retire the still-live document. An **actual**
main-frame provisional start invalidates pending reads; after a canceled
provisional load, the old document is usable only when WebKit reports its
previously finished URL still live. Navigation or close racing the async read
returns `Stale` or `Closed`. Move, resize,
fullscreen, backing-scale, and observed zoom changes invalidate revisions.
Re-read the snapshot and remeasure the DOM anchor after layout or viewport
changes. **Do not multiply CSS `getBoundingClientRect()` values by backing
scale:** the mapping from CSS and `visualViewport` at non-default WebKit zoom
to Cocoa points has not been established. No renderer script or native IPC
grant is installed by this API.

## Rust API

`DesktopApp` builds a `DesktopFrame`. Register state and handlers before
`build()`, then run the frame:

```rust
use webui_desktop::{DesktopApp, Result};

fn main() -> Result<()> {
    let frame = DesktopApp::from_bundle("./desktop-bundle")?
        .route("/", |_| Ok(serde_json::json!({ "page": "dashboard" })))?
        .build()?;
    webui_desktop::run_frame(frame)
}
```

For development, replace `from_bundle` with
`DesktopApp::from_source(DesktopSourceConfig::new(build_options))` and enable the
`source` feature. The same builder accepts `.state_value(...)`, `.route(...)`,
`.window(...)`, `.shell(...)`, and IPC registrations. `build()` performs startup
rendering; configure the window on the builder so the first render and native
frame agree.

Every initial window field, including width, height, titlebar style,
background, size limits, and devtools, can be set in Rust without a JSON file:

```rust
use webui_desktop::{DesktopApp, Rgba, TitlebarStyle, WindowOptions};

let frame = DesktopApp::from_bundle("./desktop-bundle")?
    .window(WindowOptions {
        title: "Contacts".into(),
        width: 1280,
        height: 900,
        titlebar: TitlebarStyle::Overlay { height: 48 },
        background: Some(Rgba { r: 16, g: 16, b: 20, a: 255 }),
        ..WindowOptions::default()
    })
    .build()?;
```

| API | Use |
| --- | --- |
| `DesktopApp::from_bundle`, `from_source` | Open a packaged bundle or compile source |
| `DesktopAppBuilder` | Register state, route/API/IPC handlers, window and shell options |
| `DesktopFrame` | Own the runtime, events, window handle, and shell configuration |
| `DesktopRuntime` | Render and handle requests without launching a window |
| `DesktopFrameBackend`, `run_frame_with` | Supply a custom backend |
| `find_packaged_resources_dir()` | Locate bundle resources next to a packaged runner |

`.route("/contacts/:id", |ctx| ...)` receives route parameters through
`ctx.param("id")`. Route providers run at startup and on subsequent HTML and
partial navigations. `ApiRouteRegistry` handles custom-protocol endpoints such
as `/api/contacts/:id` so client code can use ordinary `fetch`. Keep mutable
application data in Rust storage and synchronize access across handlers.

Set a stable `.app_id("com.example.contacts")` for persistent browser storage.
Without an app ID, the frame gets a fresh profile. IDs allow 1-255 ASCII
letters, digits, `.`, `_`, and `-`, but cannot end in a dot. Packaged apps take
the ID from their manifest.

## Events and window control

Register Rust event callbacks on the frame:

- `frame.on_event(handler)?` keeps the handler for the frame's lifetime.
- `frame.subscribe(handler)?` returns an `EventSubscription`; keep the guard
  alive and drop it to unsubscribe.

Callbacks run on the native UI thread. Return `EventResponse::Continue` for
normal behavior. Only `WindowCloseRequested` and `NavigationRequested` can
return `PreventDefault` to cancel the action. Do not block the UI thread.

`frame.window_handle()` queues `set_title`, `set_background`, `set_size`,
`minimize`, `maximize`, `unmaximize`, `fullscreen`, `center`, `focus`,
`request_close`, `start_drag`, and `set_always_on_top`. A successful call
means the command was accepted, not that the OS has completed it. Handle
queue errors; `request_close` can still be cancelled by a Rust event handler.

Set initial defaults with `WindowOptions` or desktop package flags. To
respond to a theme change after launch, use the Rust event callback:

```rust
use webui_desktop::{DesktopEvent, EventResponse, Rgba};

let handle = frame.window_handle().clone();
frame.on_event(move |event| {
    if let DesktopEvent::ThemeChanged { dark } = event {
        let color = if *dark {
            Rgba { r: 16, g: 16, b: 20, a: 255 }
        } else {
            Rgba { r: 250, g: 250, b: 250, a: 255 }
        };
        if let Err(error) = handle.set_background(color) {
            eprintln!("Could not update window background: {error}");
        }
    }
    EventResponse::Continue
})?;
```

`set_background` updates the native under-page color, the current document's
root background and CSS variable, and subsequent full document renders. App
CSS that paints its own content still controls that content. `set_size` changes
the live content size. Native titlebar style, caption-button size, visual
effects, and shell settings are selected before `build()`; they cannot be
switched on the live frame. Runtime changes do not modify the bundle manifest,
app ID, or defaults on the next launch.

The active web document can observe best-effort `CustomEvent`s on `window`:

```javascript
window.addEventListener('webui:window-resized', event => {
  console.log(event.detail.width, event.detail.height);
});
```

| Events | `detail` fields beyond `type` |
| --- | --- |
| `ready`, `exiting` | None |
| `window-resized` | `window_id`, `width`, `height` |
| `window-moved` | `window_id`, `x`, `y` |
| `window-maximized`, `window-unmaximized`, `window-minimized`, `window-restored` | `window_id` |
| `window-entered-fullscreen`, `window-left-fullscreen` | `window_id` |
| `window-focused`, `window-blurred`, `window-close-requested`, `window-closed` | `window_id` |
| `theme-changed` | `dark` |
| `scale-factor-changed` | `scale` |
| `navigation-requested`, `navigation-completed` | `window_id`, `url` |

JavaScript names have the `webui:` prefix; Rust uses the corresponding
`DesktopEvent` variants. Delivery is asynchronous and not replayed. `ready`
means native initialization, not page hydration. JavaScript
`preventDefault()` cannot cancel a native event; use the Rust callback.
Linux does not emit `window-moved`.

## Window and manifest

Configure desktop packaging on the CLI without adding desktop fields to
`package.json`:

```bash
webui desktop package ./my-app --target macos-app --out ./packages \
  --source ./my-app/src --state ./my-app/data/state.json \
  --assets ./my-app/dist \
  --projection-manifest ./my-app/dist/webui-projection.json \
  --build-script build:client \
  --runner-crate my-app-desktop \
  --app-id com.example.contacts --app-name Contacts \
  --titlebar-style overlay --titlebar-height 48
```

CLI file paths are relative to the current directory. Client projection
metadata must stay current. `desktop package` also accepts `--theme`,
`--plugin`, `--icon`, identity, publisher, runner and window flags; see the
[CLI reference](/guide/cli/#webui-desktop) for the full list. Rust hosts set the
same window fields on `WindowOptions`, pass source and startup state through
`DesktopSourceConfig`, and set identity and shell options on `DesktopAppBuilder`.
For Rust-owned bundle creation and packaging, pass `DesktopBundleOptions` to
`build_desktop_bundle` and `DesktopPackageOptions` to
`package_desktop_bundle` (with the `source` feature):

```rust
use std::path::PathBuf;
use webui_desktop::{
    build_desktop_bundle, package_desktop_bundle, BuildOptions, DesktopBundleOptions,
    DesktopPackageOptions, DesktopPackageTarget, DesktopShellConfig, TitlebarStyle, WindowOptions,
};

fn package(runner_exe: PathBuf) -> webui_desktop::Result<PathBuf> {
    let bundle_dir = PathBuf::from("desktop-bundle");
    build_desktop_bundle(DesktopBundleOptions {
        build_options: BuildOptions {
            app_dir: PathBuf::from("src"),
            ..BuildOptions::default()
        },
        out_dir: bundle_dir.clone(),
        state_file: Some(PathBuf::from("data/state.json")),
        asset_root: Some(PathBuf::from("dist")),
        token_css: None,
        app_id: "com.example.contacts".into(),
        app_name: "Contacts".into(),
        version: "1.0.0".into(),
        publisher: "Example".into(),
        window: WindowOptions {
            title: "Contacts".into(),
            titlebar: TitlebarStyle::Overlay { height: 48 },
            remember_state: true,
            ..WindowOptions::default()
        },
        icon_file: Some(PathBuf::from("desktop/icon.icns")),
        shell: DesktopShellConfig::default(),
        package_targets: vec![DesktopPackageTarget::MacosApp],
    })?;
    Ok(package_desktop_bundle(DesktopPackageOptions {
        bundle_dir,
        out_dir: PathBuf::from("packages"),
        target: DesktopPackageTarget::MacosApp,
        runner_exe,
    })?.output_path)
}
```

Set `BuildOptions::plugin` and `projection_manifests` for interactive builds.
`DesktopBundleOptions::token_css` accepts pre-resolved theme tokens. In a Rust
build script or host tool, run any web build scripts and compile the chosen
runner before calling these APIs; those build commands are not part of the
desktop runtime.

### Precompiled host layout

An application that owns its Rust HTTP server and sealed WebUI assets can
package **only** its already-compiled host and explicitly mapped resources.
Enable `microsoft-webui-desktop`'s `packaging` feature; it does not enable
`source`, `native`, a Node host, or runtime work:

```rust
use std::path::PathBuf;
use webui_desktop::{
    package_precompiled_host, DesktopPackageTarget, PrecompiledHostOptions,
    PrecompiledResource, ResourceKind,
};

fn layout() -> webui_desktop::Result<PathBuf> {
    let options = PrecompiledHostOptions::new(
        PathBuf::from("target/release/player"),
        DesktopPackageTarget::MacosApp,
        PathBuf::from("packages"),
    )?
    .identity("com.example.player", "Player", "1.0.0")?
    .target_triple("aarch64-apple-darwin")?
    .icon("icon.icns")?
    .add_resource(PrecompiledResource::new(
        "target/release/worker",
        "workers/worker",
        ResourceKind::Executable,
    )?)?
    .add_resource(PrecompiledResource::new(
        "dist/sealed.webui",
        "sealed/webui.bundle",
        ResourceKind::Data,
    )?)?;
    let result = package_precompiled_host(options)?;
    Ok(result.output_path)
}
```

On macOS, the host and mapped executables go under `Contents/MacOS`, while
data and the optional `.icns` icon go under `Contents/Resources`; an
`Info.plist` records the supplied identity. Windows and Linux portable layouts
put executables at the package root and data in `resources`. The result also
reports the copied host, resource, and icon paths for consumer-owned packagers.
Each mapping is a single file, not a directory; enumerate a sealed asset tree
explicitly if needed. Destinations must be safe relative paths without `.` or
`..`, and collisions (including case-only differences) are rejected.

The target triple must match the requested layout and the Mach-O, PE, or ELF
architecture of the host and native mapped resources. On Unix, executable
inputs must have an execute bit. Input symlinks, input/output overlap, missing
files, and any existing package path fail before writing; a copy-time I/O
failure may leave a partial *new* directory for the caller to remove. Windows
portable packaging additionally requires the matching native Windows build's
bootstrap DLL, license, notices, and provenance beside the host; it copies
those deployment files but does **not** install the shared Windows App Runtime,
Visual C++ Redistributable, or WebView2. There is no installer, DMG, ZIP,
NSIS, signing, or updater publication API in this layout slice. The existing
`source`/CLI bundle package flow remains unchanged.

Inputs are held open from validation through copying, and copied bytes and
native headers are checked before success. On Unix, new output files are
created exclusively relative to a private directory handle. On Windows,
directory creation remains path-based; **use an output parent not writable by
untrusted concurrent processes**. A directory-swap attacker with write access
to that parent can otherwise redirect a Windows destination despite exclusive
file creation. This layout API is not a sandbox or an installer security
boundary. Keep the output parent trusted on every platform so the returned
path cannot be renamed or replaced after packaging completes.

`WindowOptions` supports title and size limits, resize/maximize/fullscreen,
always-on-top, centering, background, titlebar, effect, remembered geometry,
and devtools. `titlebar.style` is `native`, `hidden-inset`, `overlay`, or `none`.
For an overlay, `titlebar.height` sizes the application band. On Windows,
caption buttons default to Standard (32 DIPs) independently of that height;
pass `--caption-button-size tall` for 48-DIP buttons. The Rust field and bundle
manifest key are `caption_button_size`. This setting has no
effect on macOS or Linux.

The built-in backends support native titlebars and custom titlebar styles.
Effects depend on the OS: macOS supports `vibrancy`, `acrylic`, `mica`, and
`tabbed`; Windows supports the first three; Linux supports none. Native menus
and tray icons are available on macOS only. Unsupported requested capabilities
fail before launch.

For a custom header, mark the drag region `webui-drag` and interactive
children `webui-no-drag`. A double-click on the drag region toggles maximize.
Use `--webui-titlebar-inset-start`, `--webui-titlebar-inset-end`, and
`--webui-titlebar-height` CSS variables to keep content clear of native
controls. Do not use Chromium-only `-webkit-app-region`; WKWebView and
WebKitGTK do not implement it.

`webui desktop build` writes a bundle:

| Bundle path | Contents |
| --- | --- |
| `protocol.bin` | Compiled template protocol |
| `assets/` | CSS, client assets, and optional IPC runtime |
| `state.json` | Optional seed state |
| `manifest.webui-desktop.json` | App identity, window and shell defaults, integrity hashes |

The generated manifest is for `DesktopApp::from_bundle`; do not hand-edit it.
Its `shell` section contains `icon_path`, menus, and tray configuration. Set
shell defaults on the builder with `.shell(...)` when running from source.
For macOS Dock artwork, bundle icons must stay relative to the selected bundle
root (no absolute paths, `..`, or symlink escapes); source hosts may supply an
absolute icon path. An invalid or unreadable icon does not block launch.
Check platform capabilities before requesting menus, tray, or effects.

## Message passing

Enable the SDK's `application-ipc` feature and install the browser runtime:

```bash
pnpm add @microsoft/webui-desktop
```

Define one proto3 contract for your app. For example:

```proto
syntax = "proto3";
package example.desktop;
import "webui/ipc/options.proto";
import "google/protobuf/empty.proto";

option (webui.ipc.contract_name) = "example.desktop";
option (webui.ipc.contract_major) = 1;

message Item { uint64 id = 1; }
service Host {
  option (webui.ipc.receiver) = HOST;
  rpc Save(Item) returns (google.protobuf.Empty) {
    option (webui.ipc.id) = 1101;
  }
}
service Renderer {
  option (webui.ipc.receiver) = RENDERER;
  rpc Changed(Item) returns (webui.ipc.Notification) {
    option (webui.ipc.id) = 2001;
    option (webui.ipc.notification) = true;
  }
}
```

Keep application IDs above 1023 unique and stable. Generate and commit the
bindings and compatibility lock; the generator requires `protoc`:

```bash
webui desktop ipc generate schema/application.proto \
  --rust-out desktop/src/generated \
  --ts-out src/generated \
  --lock schema/ipc-schema.lock.json
```

Add `--check` in CI to detect drift. The four application flows use generated
methods:

| Direction | API |
| --- | --- |
| JavaScript to Rust request | `connection.host.save(...)` returns a promise |
| JavaScript to Rust notification | Generated host emitter |
| Rust to JavaScript request | `RendererClient` method returns an `IpcCall<T>` |
| Rust to JavaScript notification | Generated renderer emitter and JS subscription |

In JavaScript, call the generated `connectDesktop(createDesktopTransport(), ...)`
to connect and register renderer handlers; importing bindings alone does not
connect. Close subscriptions and the connection when no longer needed. In
Rust, implement the generated `HostHandler`, register it with `IpcRegistry`,
and set `IpcOptions::for_schema(&generated::SCHEMA)` on the app builder before
`build()`. The default denies application methods until you explicitly grant
the generated contract.

Generated JavaScript values use `bigint` for 64-bit integers, `Uint8Array` for
bytes, and `Map` for protobuf maps. Requests support timeouts and cancellation;
cancellation does not undo a Rust handler that already ran. Notifications
acknowledge admission, not subscriber completion. Connections belong to a
document: reconnect after a full navigation or a back/forward-cache restore.
`IpcOptions::limits` configures message size and timeout (defaults: 1 MiB and
30 seconds). There are no automatic retries.

Native `webui:*` lifecycle events and `webui-drag` window controls are not
application IPC and do not require `application-ipc`.

## Package and run

For an app root:

```bash
webui desktop package ./my-app --target windows-portable --out ./packages
```

Targets are `macos-app`, `windows-portable`, and `linux-portable`. The default
build is an optimized, bundle-only runner; `--debug` builds a debug runner.
`--target all` creates all three layouts but does not cross-compile the
executable. Use `--runner <PATH>` when packaging an existing bundle with a
custom host; `--no-web-build` skips app build scripts when you ran them already.
Installers, signing, and archives are not generated.

The Windows portable package is a **folder**, not a standalone `.exe`. Keep
`resources/webui`, `Microsoft.WindowsAppRuntime.Bootstrap.dll`, and the
Windows App SDK notices beside the runner. The target machine needs WebView2
Runtime 122.0.2365.46 or later, Windows App Runtime 1.8
(8000.946.1701.0 or later in that family), and the Visual C++
Redistributable. Use the current [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).
Linux builds need GTK4 and WebKitGTK 6 development packages.

`--icon` supplies the macOS `.icns` bundle icon and is copied into
portable resources. For the Windows executable and taskbar, embed an `.ico` as
icon resource 1 in the Rust runner. On macOS, source-mode runners can supply
an absolute `.icns` path through `DesktopShellConfig::icon_path` to show a Dock
icon without an `.app` bundle.

Use `--devtools` with `webui desktop run` or `build` to allow inspection. On
macOS, enable Safari's Develop menu to inspect WKWebView.
