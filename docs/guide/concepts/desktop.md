# Desktop Apps

WebUI desktop apps render the same templates as browser apps in a native window.
They use WebView2 on Windows, WKWebView on macOS, and GTK4/WebKitGTK 6 on Linux.
No Electron, bundled browser, Node runtime, or localhost server is needed in
the shipped app.

## Get started

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
files unless you pass `--force`.

| SDK feature | What it adds |
| --- | --- |
| Default (none) | Bundles, rendering, route/API handlers, custom backends |
| `native` | System webview and `run_frame` |
| `source` | Build from templates with `DesktopSourceConfig` |
| `application-ipc` | Typed application messages |
| `cli` | Desktop CLI sidecar; not needed in an app runner |

For a native runner:

```toml
[dependencies]
webui-desktop = { package = "microsoft-webui-desktop", version = "0.0.29", default-features = false, features = ["native"] }
serde_json = "1.0"

[features]
default = []
source = ["webui-desktop/source"]
```

Enable `application-ipc` separately if you use generated messages.

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

`frame.window_handle()` queues `set_title`, `set_size`, `minimize`, `maximize`,
`unmaximize`, `fullscreen`, `center`, `focus`, `request_close`, `start_drag`,
and `set_always_on_top`. A successful call means the command was accepted,
not that the OS has completed it. Handle queue errors; `request_close` can
still be cancelled by a Rust event handler.

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

Set window defaults in `webuiDesktop` in your app's `package.json`:

```json
{
  "webuiDesktop": {
    "app": "src",
    "state": "data/state.json",
    "assets": "dist",
    "projectionManifests": ["dist/webui-projection.json"],
    "runnerCrate": "my-app-desktop",
    "buildScripts": ["build:client"],
    "appId": "com.example.contacts",
    "appName": "Contacts",
    "titlebar": { "style": "overlay", "height": 48 }
  }
}
```

`app`, `state`, and `assets` are paths relative to the app root.
`projectionManifests` supplies metadata from the client build; keep it current.
Other options include `theme`, `plugin`, `icon`, `appVersion`, window dimensions,
and `devtools`. Use the same `WindowOptions` fields in a Rust source host.

`WindowOptions` supports title and size limits, resize/maximize/fullscreen,
always-on-top, centering, background, titlebar, effect, remembered geometry,
and devtools. `titlebar.style` is `native`, `hidden-inset`, `overlay`, or `none`.
For an overlay, `titlebar.height` sizes the application band. On Windows,
caption buttons default to Standard (32 DIPs) independently of that height;
set `webuiDesktop.captionButtonSize` to `"tall"` for 48-DIP buttons. The Rust
field and bundle manifest key are `caption_button_size`. This setting has no
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

For an app root with `webuiDesktop` metadata:

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

`webuiDesktop.icon` supplies the macOS `.icns` bundle icon and is copied into
portable resources. For the Windows executable and taskbar, embed an `.ico` as
icon resource 1 in the Rust runner. On macOS, source-mode runners can supply
an absolute `.icns` path through `DesktopShellConfig::icon_path` to show a Dock
icon without an `.app` bundle.

Use `--devtools` with `webui desktop run` or `build` to allow inspection. On
macOS, enable Safari's Develop menu to inspect WKWebView.
