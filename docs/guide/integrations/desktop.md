# WebUI Desktop

WebUI desktop apps use the same build-time protocol and server-side rendering
pipeline as browser apps, then run in a lightweight Rust desktop shell. The
shell uses system webviews only:

| Platform | Webview |
|----------|---------|
| Windows | WebView2 |
| macOS | WKWebView |
| Linux | GTK4/WebKitGTK 6 |

Electron, Node, bundled Chromium, and localhost HTTP servers are not part of the
desktop runtime.

Linux builds require GTK4 and WebKitGTK 6 development packages on the target
system or a configured cross-compilation sysroot. Windows support uses the
WebView2 Runtime **122.0.2365.46 or later** and the target-gated Win32 backend;
older runtimes fail at startup with an update hint. Use the current Evergreen
Runtime for security updates. This minimum corresponds to
[WebView2 SDK 1.0.2365.46's worker request support](https://learn.microsoft.com/en-us/microsoft-edge/webview2/release-notes/sdk/1-0-2365-46).
Validate runtime behavior
on Windows CI or a Windows developer machine with WebView2 installed.
Intercepted HEAD responses retain the handler's status and content type but
never expose its body, including error responses. The handler still receives
HEAD rather than an implicit GET.

Windows application resources use the system browser's native request handling
in both source and packaged apps, including fetches from workers. WebUI does not
replace `window.fetch`: standard `Request`, `Response`, and `AbortSignal` behavior
is preserved. Aborting a resource fetch cancels browser delivery, not side effects
of a Rust API handler that has already run. Typed application IPC has its own
request cancellation contract.

On Windows, set a stable `DesktopAppBuilder::app_id` (or
`DesktopFrame::with_app_id`) to persist browser storage for your application.
Bundle-backed apps use their manifest identity automatically. Identities accept
1–255 ASCII letters, digits, dots, underscores, and hyphens, with no trailing
dot; invalid identities fail at startup. Different identities have isolated
localStorage and IndexedDB, even when they use the same runner executable.
Without an identity, each frame uses a fresh profile with best-effort shutdown
cleanup, not persistent browser storage.

Rust hosts use one SDK, `microsoft-webui-desktop`, imported as `webui_desktop`.
Construct an app with `DesktopApp`, register its Rust handlers, then pass the
resulting `DesktopFrame` to `webui_desktop::run_frame`. The native runner selects
the platform backend; application code does not need OS-specific launch paths.
Window features and event availability still depend on the platform.

## Command shape

Use `webui desktop ...` for desktop commands. `webui` is the only public CLI;
desktop support is implemented by a separate `webui-desktop` sidecar backend so
normal build/serve/inspect installs stay lean. The sidecar is resolved
automatically when desktop support is installed; set `WEBUI_DESKTOP_BINARY` only
to override discovery.

```bash
webui desktop build ./src \
  --state ./data/state.json \
  --servedir ./dist \
  --out ./desktop-bundle \
  --plugin=webui \
  --devtools
```

## Minimum desktop app

Start with no Rust at all:

```bash
webui desktop run ./src
```

This builds the entry template and opens it in the native system webview.
Restart the command after source changes; desktop `--watch` is not supported.
Add a
Rust host only when the app needs dynamic route state, native IPC, or direct
window control. To create the progressive starting point, run:

```bash
webui desktop init ./my-app
cd ./my-app
cargo run --manifest-path desktop/Cargo.toml --features source
```

The command creates `src/index.html`, `package.json`, and a `desktop/` runner
crate. It never overwrites those generated files unless `--force` is passed.
The runner is a standalone Cargo workspace, even when created inside another
workspace, and includes an optimized release profile. Init does not modify an
enclosing workspace. To join one deliberately, add the member yourself, remove
the generated standalone workspace declaration, and use the enclosing root's
release profile.

The generated runner defaults to bundled execution. Source development is an
explicit `source` feature; without it, missing packaged resources produce an
error before a window opens.

## One SDK for source and bundled apps

A native application needs one WebUI SDK dependency:

```toml
[dependencies]
webui-desktop = { package = "microsoft-webui-desktop", version = "0.0.29", default-features = false, features = ["native"] }

[features]
default = []
source = ["webui-desktop/source"]
```

| SDK features | Available APIs |
| --- | --- |
| None, the default | Bundle loading, rendering, route/API handlers, frame configuration, and custom backend integration |
| `native` | Built-in platform backend and `run_frame` |
| `application-ipc` | Typed application IPC, sessions, workers, and browser assets |
| `source` | Source compilation, `DesktopSourceConfig`, compiler options, and build/package APIs |
| `native, source` | Native source development |
| `cli` | The `webui-desktop` sidecar binary; includes `native`, `source`, and command-line tooling |

Application runners normally enable `native`, not `cli`. The generated
application's local `source` feature forwards to the SDK; selecting Cargo's
debug or release profile does not enable source compilation.
Neither `native`, `source`, nor `cli` enables application IPC. Without
`application-ipc`, the SDK does not reserve IPC paths or install application IPC
assets or transport handlers. Native window controls and lifecycle events do
not require application IPC.

For bundled execution:

```rust
use webui_desktop::{DesktopApp, DesktopAppBuilder, Result};

fn configure(app: DesktopAppBuilder) -> Result<DesktopAppBuilder> {
    app.route("/", |_| {
        Ok(serde_json::json!({ "page": "dashboard" }))
    })
}

fn main() -> Result<()> {
    let app = DesktopApp::from_bundle("./desktop-bundle")?;
    let frame = configure(app)?.build()?;
    webui_desktop::run_frame(frame)
}
```

The route-state examples use `serde_json` in the application. For source
development, choose a source input and reuse the same `configure` function:

```rust
#[cfg(feature = "source")]
fn source_app() -> webui_desktop::DesktopAppBuilder {
    use webui_desktop::{BuildOptions, DesktopApp, DesktopSourceConfig};

    let config = DesktopSourceConfig::new(BuildOptions {
        app_dir: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src"),
        ..Default::default()
    });
    DesktopApp::from_source(config).app_id("com.example.myapp")
}
```

Set source-only asset and theme options on `DesktopSourceConfig`. The shared
builder accepts startup state, route/API/IPC handlers, pre-resolved token CSS,
asset-size limits, window options, shell configuration, and application identity.
Register handlers before `.build()`: that call performs startup rendering and
returns a frame, not just a runtime.

Bundle construction preserves the manifest's window, shell, and app ID.
Explicit `.window(...)` and `.shell(...)` calls replace those defaults.
Configure render-related window options on the builder so the startup HTML and
native frame use the same values. Source hosts can supply a stable identity with
`.app_id(...)`; the completed frame exposes it as `app_id`.

## Window options

`webuiDesktop` and `WindowOptions` support `title`, `width`, `height`,
`min_width`, `min_height`, `max_width`, `max_height`, `resizable`, `maximized`,
`fullscreen`, `always_on_top`, `center`, `background`, `titlebar`, `effect`,
`remember_state`, and `devtools`. `background` is `#rrggbb` or `#rrggbbaa` and
is painted before the first web content paint.

`titlebar` is one of `native`, `hidden-inset`, `overlay` with a `height`, or
`none`. `effect` is one of `none`, `vibrancy`, `acrylic`, `mica`, or `tabbed`.
The launcher validates requested titlebar, effect, and shell capabilities before
native startup. Requests the selected backend does not advertise return an
error. Capabilities describe implemented features, not identical behavior on
every OS, OS version, or desktop environment.

The built-in backends advertise these feature groups:

| Capability | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Titlebar styles (`hidden-inset`, `overlay`, `none`) | Yes | Yes | Yes |
| Window effects | Yes (all four) | Yes (except `tabbed`) | No |
| Application menu | Yes | No | No |
| Tray icon | Yes | No | No |

Effects can use a documented native equivalent rather than the same material on
every OS:

| `effect` | macOS | Windows | Linux |
| --- | --- | --- | --- |
| `none` | Yes | Yes | Yes |
| `vibrancy` | Yes | Yes, as acrylic | No |
| `acrylic` | Yes, as vibrancy | Yes | No |
| `mica` | Yes, as vibrancy | Yes | No |
| `tabbed` | Yes | No | No |

Where a platform has no exact analogue but a close one, it maps rather than
fails: `vibrancy` on Windows renders through the acrylic system backdrop.
Where it has no analogue at all, it does not advertise the effect, so the
mismatch surfaces as a validation error. Linux advertises no window effects or
tray support. On Windows, `overlay` removes the native caption buttons, so web
content must draw its own.

Query `PlatformFrameBackend::capabilities()` when choosing optional features:

```rust
use webui_desktop::{DesktopFrameBackend, PlatformFrameBackend, WindowEffect};

let capabilities = PlatformFrameBackend::new().capabilities();
let effect = if capabilities.supports_effect(WindowEffect::Mica) {
    WindowEffect::Mica
} else {
    WindowEffect::None
};
```

Apply the chosen effect to your window options. Placement, focus, and stacking
remain subject to OS policy: a Linux compositor may ignore `always_on_top`, and
Wayland controls window placement, so `center` is advisory there.

For non-native titlebars, the runtime injects these CSS custom properties:
`--webui-titlebar-inset-start`, `--webui-titlebar-inset-end`,
`--webui-titlebar-height`, and, when `background` is set,
`--webui-window-background`. Use them in layout rather than hardcoding
platform offsets.

Mark an application-drawn drag area with `webui-drag`; add `webui-no-drag` to a
nested interactive element. Double-clicking a drag area toggles maximize. Do
not use Chromium's `-webkit-app-region: drag`: WKWebView and WebKitGTK do not
implement that Chromium-specific property, and WebView2 apps should use the
WebUI drag contract for cross-platform behavior.

## Lifecycle and native window control

Choose the lifetime of each Rust event handler:

- `frame.on_event(handler)?` registers a persistent handler for the frame's
  lifetime and returns `Result<(), EventRegistrationError>`.
- `frame.subscribe(handler)?` returns an `EventSubscription`. Keep the guard
  alive for as long as you need the handler; dropping it unregisters the
  callback from subsequent dispatches. A dispatch already in progress may
  still finish calling it.

For example:

```rust
use webui_desktop::{DesktopApp, EventResponse};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame = DesktopApp::from_bundle("./desktop-bundle")?.build()?;

    // Add non-blocking observer or cancellation logic inside these callbacks.
    frame.on_event(|_event| EventResponse::Continue)?;
    let _subscription = frame.subscribe(|_event| EventResponse::Continue)?;

    // The named guard stays alive until run_frame returns.
    webui_desktop::run_frame(frame)?;
    Ok(())
}
```

Do not use `let _ = frame.subscribe(...)?`: that drops the guard immediately.
Both registration methods are fallible. The combined limit is 256 handlers;
`EventRegistrationError::Capacity` means unused subscriptions should be dropped,
and registration after frame shutdown returns `Closed`.

Callbacks run synchronously on the backend UI thread and must not block it.
Return `EventResponse::Continue` to allow normal behavior.
`EventResponse::PreventDefault` can cancel `WindowCloseRequested` and
`NavigationRequested`; it does not cancel other event types. The event vocabulary
is `Ready`, `WindowResized`,
`WindowMoved`, `WindowMaximized`, `WindowUnmaximized`, `WindowMinimized`,
`WindowRestored`, `WindowEnteredFullscreen`, `WindowLeftFullscreen`,
`WindowFocused`, `WindowBlurred`, `WindowCloseRequested`, `WindowClosed`,
`ThemeChanged`, `ScaleFactorChanged`, `NavigationRequested`,
`NavigationCompleted`, and `Exiting`. A backend need not emit every event on
every platform. `Ready` means native initialization, not document or component
readiness.

While a document is available, backends also send best-effort `CustomEvent`
notifications named `webui:` plus the kebab-case event name. These are
asynchronous observations, not another cancellation mechanism. See
[Rust to JavaScript: lifecycle events](#rust-to-javascript-lifecycle-events)
for delivery limits and payloads.

The Linux backend does not emit `WindowMoved`/`webui:window-moved`. Do not make
portable application behavior depend on receiving position changes.

`DesktopFrame::window_handle()` exposes a `Send + Sync` `WindowHandle` that queues
UI-thread commands: `set_title`, `set_size`, `minimize`, `maximize`,
`unmaximize`, `fullscreen`, `center`, `focus`, `request_close`, `start_drag`, and
`set_always_on_top`. Clone the handle to send commands from application workers;
the frame itself is not cloneable.

A successful command call means **accepted for dispatch**, not that the OS has
applied it. In particular, `request_close()` queues a close request that a Rust
handler may still veto. Queues are bounded to 256 commands, titles to 16 KiB of
UTF-8 each, and buffered title payloads to 64 KiB in total. Handle `QueueFull`,
`TitleQueueFull`, and `TitleTooLarge` rather than assuming a command was accepted.
Do not spin or block the UI thread retrying a full queue.

Dropping the frame closes its command channel and releases its event
registrations. Retained handles return `WindowCommandError::Closed` after
shutdown; stop sending rather than retrying that error. Retaining a subscription
does not keep the native frame alive. Pending commands are discarded during
teardown, not flushed as a shutdown guarantee.

Set `remember_state: true` to request restoration of available window geometry.
Position and placement remain platform-dependent. Invalid or off-screen saved
geometry is not applied.

## Bundle contents

`webui desktop build` writes an immutable bundle:

| Path | Description |
|------|-------------|
| `protocol.bin` | Compiled protobuf protocol |
| `assets/` | Generated CSS, copied static assets, and the desktop IPC helper |
| `state.json` | Optional startup state |
| `manifest.webui-desktop.json` | App metadata, window defaults, package targets, and SHA-256 hashes |

Static assets are copied with traversal protection. Link CSS remains the default
desktop CSS strategy for startup performance; use `--theme` when the app relies
on design tokens.

## Packaging

Package a Rust-first desktop app root in one command:

```bash
webui desktop package ./my-app --target macos-app --out ./packages
webui desktop package ./my-app --target macos-app --out ./packages \
  --theme @microsoft/webui-examples-theme
```

Packaging builds an optimized release runner with its default Cargo features
disabled. This matches the generated app's runtime-only configuration. Use
`--debug` to choose a development build; it does not enable source compilation.
For example:

```bash
webui desktop package ./my-app --target macos-app --out ./packages --debug
```

Apps needing additional Cargo features can opt in with
`--runner-features feature-a,feature-b` or a `webuiDesktop.runnerFeatures` array.
`--runner-default-features` or `webuiDesktop.runnerDefaultFeatures: true`
restores the runner's defaults. These are explicit overrides: enabling `source`
can reintroduce the compiler into the runner. Keep source support off for normal
distribution.

For app roots, the sidecar reads `webuiDesktop` from `package.json`, runs the
configured web build scripts, builds the app-specific Cargo runner crate, stages
non-generated assets, builds the bundle, and packages that runner. Example:
Pass `--theme` to override `webuiDesktop.theme` for a one-off package.
Pass `--icon` to override `webuiDesktop.icon`; macOS packages use `.icns` icons
as `CFBundleIconFile`, and portable layouts copy the icon into resources.

```json
{
  "webuiDesktop": {
    "app": "src",
    "state": "data/state.json",
    "assets": "dist",
    "theme": "@microsoft/webui-examples-theme",
    "icon": "desktop/app.icns",
    "plugin": "webui",
    "runnerCrate": "contact-book-desktop",
    "buildScripts": ["build:deps", "build:client"],
    "appId": "com.microsoft.webui.contactbook",
    "appName": "Contact Book Manager",
    "appVersion": "1.0.0",
    "title": "Contact Book Manager",
    "width": 1200,
    "height": 800,
    "devtools": true
  }
}
```

Existing bundle packaging remains available for lower-level flows:

```bash
webui desktop package ./desktop-bundle --target macos-app --out ./packages \
  --runner ./target/release/my-desktop-host
```

Use `--runner` for existing bundles with route providers or IPC commands. The
runner is your app-specific Rust executable; it registers routes/commands and
loads the packaged bundle through `DesktopApp`. Omitting
`--runner` packages the generic sidecar and is appropriate only for
file-backed/static seed-state bundles.

App-specific runners can use the shared resource helper instead of OS-specific
bundle paths:

```rust
use webui_desktop::{DesktopApp, DesktopError, Result};

fn main() -> Result<()> {
    let resources = webui_desktop::find_packaged_resources_dir()
        .ok_or(DesktopError::PackagedResourcesNotFound)?;
    let frame = DesktopApp::from_bundle(resources)?.build()?;
    webui_desktop::run_frame(frame)
}
```

When application setup already loaded a `DesktopBundleManifest`, use
`DesktopApp::from_bundle_config_and_manifest(config, manifest)`, where `config`
is a `DesktopBundleConfig`. This constructor returns the builder directly; add
handlers and call `.build()?` without reloading the manifest. The generated
runner already selects packaged resources first and only offers a source
fallback when built with its local `source` feature.

The Rust packager currently writes:

| Target | Output |
|--------|--------|
| `macos-app` | Runnable `<AppName>.app` with a WKWebView launcher and bundled resources |
| `windows-portable` | Portable folder layout for a Windows runner and bundled resources |
| `linux-portable` | Portable folder layout for a Linux runner and bundled resources |

`--target all` writes these three layouts using the same runner. It does not
cross-compile the executable; supply a runner built for the intended platform.
Installer generation, archives, and signing are not supported.

## Message passing

Application messages use generated Rust and TypeScript interfaces from one
proto3 contract. This is a Mojo-style authoring model over supported system
WebView APIs, not Chromium's internal Mojo runtime.

| Direction | Use | Transport | Extensible |
| --- | --- | --- | --- |
| JavaScript → Rust request | Generated host method returning `Promise<T>` | bounded protobuf | Yes, shared schema |
| Rust → JavaScript request | Generated renderer method returning `IpcCall<T>` | bounded protobuf | Yes, shared schema |
| Rust → JavaScript notification | Generated renderer emitter and JS subscription | bounded protobuf | Yes, shared schema |
| JavaScript → Rust notification | Generated host emitter and Rust handler | bounded protobuf | Yes, shared schema |
| Native lifecycle → JavaScript | `webui:*` events on `window` | injected `CustomEvent` | No, fixed event set |
| JavaScript → Rust window control | `webui-drag` regions | bounded host message | No, closed command set |
| Rust → native window | `WindowHandle` | UI-thread command queue | No, fixed command set |

Lifecycle notifications and window controls remain separate from application
data. A native lifecycle event is not an application RPC response.

### Define and generate the contract

Install the browser runtime and pinned generator tooling:

```sh
pnpm add @microsoft/webui-desktop
pnpm add -D ts-proto@2.12.3 @bufbuild/protobuf@2.15.0
```

The host also needs `protoc`. The generator reports a missing tool rather than
downloading it silently. A minimal `schema/application.proto` can declare both
directions:

```proto
syntax = "proto3";
package example.desktop;
import "webui/ipc/options.proto";
import "google/protobuf/empty.proto";

option (webui.ipc.contract_name) = "example.desktop";
option (webui.ipc.contract_major) = 1;

message Item { uint64 id = 1; bytes image = 2; }
message Label { string text = 1; }

service Host {
  option (webui.ipc.receiver) = HOST;
  rpc Save(Item) returns (google.protobuf.Empty) {
    option (webui.ipc.id) = 1101;
  }
  rpc Selected(Item) returns (webui.ipc.Notification) {
    option (webui.ipc.id) = 1102;
    option (webui.ipc.notification) = true;
  }
}

service Renderer {
  option (webui.ipc.receiver) = RENDERER;
  rpc LabelFor(Item) returns (Label) {
    option (webui.ipc.id) = 2001;
  }
  rpc Changed(Item) returns (webui.ipc.Notification) {
    option (webui.ipc.id) = 2002;
    option (webui.ipc.notification) = true;
  }
}
```

IDs above 1023 are application-owned and must remain unique and stable. An
`Empty` response is an acknowledged RPC: its promise resolves when the handler
finishes. An explicit `Notification` is different: its emitter waits for
bounded admission, not subscriber completion.

Generate the bindings and commit the generated files and compatibility lock:

```sh
webui desktop ipc generate schema/application.proto \
  --include schema \
  --rust-out desktop/src/generated \
  --ts-out src/generated \
  --lock schema/ipc-schema.lock.json \
  --ts-proto-plugin node_modules/.bin/protoc-gen-ts_proto
```

Use the same command with `--check` in CI to detect drift without rewriting
files. The SDK options schema is included automatically. Rust build scripts can
instead call `webui_desktop_build::generate`.

### JavaScript requests and receivers

Bundle the generated `ipc.ts` with the application:

The generated bindings load the desktop runtime and protobuf codecs only when
`connectDesktop()` is explicitly called. Concurrent calls share loading but
create independent connections; importing types or bindings never connects.
Enable ESM code splitting and deploy all emitted chunks to defer their transfer
and parsing. Module-loading failures reject the returned promise without
transport activation or automatic retries.

The first connection can be deferred until the user needs IPC. Its handshake
timeout starts when connection admission begins, not when the document loads.
Navigating away invalidates that document's unused admission proof.

The direct runtime import below loads the package normally. If the application
does not need IPC at startup, move that import into its explicit connection
action using `await import('@microsoft/webui-desktop')`.

```typescript
import { createDesktopTransport } from '@microsoft/webui-desktop';
import { connectDesktop } from './generated/ipc.js';

const connection = await connectDesktop(createDesktopTransport(), {
  renderer: {
    async labelFor(item, context) {
      context.signal.throwIfAborted();
      return { text: `Item ${item.id}` };
    },
  },
  onError: error => console.error(error.code, error.message),
});

const changed = connection.renderer.onChanged(item => updateView(item));
const item = { id: 42n, image: new Uint8Array([1, 2, 3]) };

await connection.host.save(item, { timeoutMs: 5_000 });
await connection.host.selected(item);

changed.close();
connection.close();
```

`save` is JS-to-Rust request/reply; `selected` is a notification to the Rust
receiver. `labelFor` handles Rust-initiated requests. `onChanged` receives
Rust-initiated notifications. `updateView` is application code.

### Rust receivers and messages to JavaScript

Enable application IPC explicitly on the host dependency:

```toml
[dependencies]
webui-desktop = { package = "microsoft-webui-desktop", version = "0.0.29", default-features = false, features = ["native", "application-ipc"] }
prost = "0.14.4"
```

The generated messages use `prost`; generator/compiler dependencies belong in
the build tool, not the runtime host.

```rust
use std::sync::Arc;
use webui_desktop::{
    ipc::{CallOptions, IpcFuture, IpcOptions, IpcRegistry, NotificationContext, RequestContext},
    DesktopApp,
};

#[path = "generated/ipc.rs"]
mod generated;
use generated::{messages::example::desktop::Item, HostHandler, RendererClient};

struct Host;

impl HostHandler for Host {
    fn save(&self, context: RequestContext, item: Item) -> IpcFuture<()> {
        Box::pin(async move {
            let page = RendererClient::new(context.session);
            let label = page.label_for(item.clone(), CallOptions::default()).await?;
            println!("{}", label.text);
            page.changed(item).await?;
            Ok(())
        })
    }

    fn selected(&self, _context: NotificationContext, item: Item) -> IpcFuture<()> {
        Box::pin(async move {
            println!("Selected {}", item.id);
            Ok(())
        })
    }
}

let mut registry = IpcRegistry::new(&generated::SCHEMA);
generated::register_host(&mut registry, Arc::new(Host))?;
let frame = DesktopApp::from_bundle(resources)?
    .ipc_registry(registry)
    .ipc_options(IpcOptions::for_schema(&generated::SCHEMA))
    .build()?;
webui_desktop::run_frame(frame)?;
```

Startup registration installs both Rust RPC and notification handlers before
the document becomes ready. The SDK executes Rust application handlers away
from the native UI thread; application code does not need Tokio just to bind
handlers. Source hosts use the same registry and options with
`DesktopApp::from_source(config)`.

### Types, permissions, and lifetime

Application code uses generated objects rather than method strings or manual
byte encoding. JavaScript 64-bit integers are `bigint`; bytes are `Uint8Array`;
protobuf maps are `Map<K,V>`. Invalid types and out-of-range values are rejected,
not silently coerced.

The default denies application methods. `IpcOptions::for_schema` explicitly
grants the generated contract; hosts may narrow its ID allowlists. Both ends
must use the same schema hash and major version. The native bootstrap and
reserved runtime assets are identical in source and packaged modes.

Requests return typed promises/futures and structured `IpcError`s. Timeouts
include queue time. `AbortSignal`, dropped Rust calls, navigation and shutdown
settle callers once; cancellation does not roll back completed side effects.
There are no automatic retries. Notifications preserve order per subscriber,
and subscription disposal prevents queued callbacks from starting. Expensive
browser work still belongs in Web Workers; blocking Rust work must cooperate
with cancellation or remain within its bounded application workers.

Rust hosts configure IPC with `IpcOptions`. Its `limits` policy defaults to
1 MiB encoded frames and a 30-second renderer RPC timeout:

```rust
use std::time::Duration;
use webui_desktop::ipc::IpcLimits;

let limits = IpcLimits::default()
    .with_max_frame_bytes(256 * 1024)?
    .with_default_timeout(Duration::from_secs(10))?;
```

Assign the policy to `IpcOptions::limits` before calling
`DesktopAppBuilder::ipc_options(...)`. `max_frame_bytes()` and
`default_timeout()` read the configured choices. The checked builders accept
frame sizes from 2,176 bytes through 8 MiB and whole-millisecond timeouts from
1 ms through 5 minutes. Rust calls select their timeout with `CallOptions`;
changing the renderer default does not change admission or notification
deadlines. Queue, callback, control, and aggregate memory budgets are managed
by the SDK, not independently configurable.

IPC capacity is bounded by frames, retained bytes, calls, and callback fanout.
Overload rejects explicitly. Native metadata carries admission and availability
signals; application payloads remain protobuf binary, without base64 or
per-byte object arrays. No idle polling or localhost server is introduced.

Connections belong to an admitted document, not just a URL. Full navigation
revokes old credentials and callbacks; reacquire a connection in the replacement
document. Ordinary same-document routing preserves it. The authenticated
principal is the committed main-document capability. Cross-origin documents
without that capability are denied; same-origin parent delegation is within
the trusted application boundary.

WKWebView back/forward document navigation waits for the outgoing renderer to
observe retirement before proceeding, so interrupted calls retain `navigated`
rather than an incidental browser Fetch error. If delivering that control fails,
the native shell cancels the traversal and reports a diagnostic; the old
connection stays closed. Same-document History API traversal preserves the
connection.

WebKit may restore a document from its back/forward cache without rerunning
application modules. The SDK retires its old connections on `pagehide` and
prepares a fresh inactive bootstrap epoch before a cache-eligible page freezes.
Reconnect explicitly on restored `pageshow`; this waits for normal native
admission and does not replay calls or revive old subscriptions:

```typescript
window.addEventListener('pageshow', event => {
  if (event.persisted) {
    reconnectAndBindHandlers().catch(reportError);
  }
});
```

`reconnectAndBindHandlers` is application code that creates a new transport,
calls the generated `connectDesktop`, and retains its new subscriptions. The
old connection remains closed, even after a successful restoration.

### Rust to JavaScript: lifecycle events

Backends send lifecycle notifications to the current web document as
`CustomEvent`s on `window`, named `webui:` plus the kebab-case event name.
Delivery is asynchronous and best-effort: a page may not exist yet, may be
navigating, or may already have been destroyed. Events are not replayed for
listeners attached later.

When delivered, event data is on `detail`, with a `type` field matching the event
name:

```javascript
window.addEventListener('webui:window-resized', (e) => {
  // e.detail === { type: 'window-resized', window_id: 1, width: 1200, height: 800 }
  console.log(e.detail.width, e.detail.height);
});

window.addEventListener('webui:theme-changed', (e) => {
  // e.detail === { type: 'theme-changed', dark: true }
});
```

The event vocabulary below does not imply every backend or document observes
every event:

| Event | `detail` fields beyond `type` |
| --- | --- |
| `webui:ready` | none |
| `webui:window-resized` | `window_id`, `width`, `height` |
| `webui:window-moved` | `window_id`, `x`, `y` |
| `webui:window-maximized` | `window_id` |
| `webui:window-unmaximized` | `window_id` |
| `webui:window-minimized` | `window_id` |
| `webui:window-restored` | `window_id` |
| `webui:window-entered-fullscreen` | `window_id` |
| `webui:window-left-fullscreen` | `window_id` |
| `webui:window-focused` | `window_id` |
| `webui:window-blurred` | `window_id` |
| `webui:window-close-requested` | `window_id` |
| `webui:window-closed` | `window_id` |
| `webui:theme-changed` | `dark` |
| `webui:scale-factor-changed` | `scale` |
| `webui:navigation-requested` | `window_id`, `url` |
| `webui:navigation-completed` | `window_id`, `url` |
| `webui:exiting` | none |

`theme-changed` and `scale-factor-changed` are application-wide and carry no
`window_id`. Sizes and positions are logical pixels on every platform; use
`scale` from `webui:scale-factor-changed` to convert to device pixels.

These are notifications, not decisions. Calling `preventDefault()` on
`webui:window-close-requested` or `webui:navigation-requested` does nothing:
the native decision is already made by then, and the Rust handler is the only
authority that can cancel it. See
[Lifecycle and native window control](#lifecycle-and-native-window-control) for
the Rust side.

Keep listeners cheap. `webui:ready` describes native readiness; it may precede
page loading or listener registration. Use `DOMContentLoaded` and component
lifecycle hooks for page initialization instead. `NavigationCompleted` also
does not guarantee that application components have finished their own work.

Do not use `webui:exiting` or `webui:window-closed` for required cleanup,
persistence, or delivery acknowledgements. The document can be gone before
those notifications could run. Keep native cleanup in Rust and persist
important application data before teardown.

### JavaScript to Rust: window control

Custom titlebars need to drag, minimize, maximize, and close the native window.
That path is deliberately not general-purpose IPC: the injected drag script
translates `webui-drag` regions into one of exactly four host messages
(`start-drag`, `minimize`, `toggle-maximize`, `close`), each capped at 256
bytes and rejected if it is anything else. Mark regions declaratively rather
than sending these yourself:

```html
<header webui-drag>
  My App
  <button webui-no-drag @click="{onSettings()}">Settings</button>
</header>
```

See [Window options](#window-options) for the drag-region contract.

## Rust route state

Desktop apps that need dynamic state write route data in Rust. Register route
providers on the desktop host:

```rust
let frame = webui_desktop::DesktopApp::from_bundle(resources)?
    .state_value(seed_state)
    .route("/", |_ctx| {
        Ok(serde_json::json!({ "page": "dashboard" }))
    })?
    .route("/contacts/:id", |ctx| {
        Ok(serde_json::json!({ "contactId": ctx.param("id") }))
    })?
    .build()?;
webui_desktop::run_frame(frame)?;
```

The same registration methods apply to `DesktopApp::from_source(config)`.
Providers run during startup rendering as well as subsequent HTML renders and
WebUI router partial requests, including full reloads of `/` and its
`/index.html` alias. `DesktopRuntime::startup_html()` is only the construction-time
snapshot, not a live view. Provider errors are surfaced instead of falling
back silently. The CLI `--state` path remains a simple seed-state fallback.
Startup providers run while `build()` constructs the frame. Once the native
window is running, routes, API handlers and rendering execute on bounded
application workers rather than the UI thread. These handlers can run
concurrently; synchronize shared mutable application state. Lifecycle event
callbacks still run on the UI thread and must not block.

For route-backed apps, keep mutable collections in shared Rust storage and
borrow them while preparing a route's view model. Use the render seed for
global settings and theme tokens rather than copying the complete application
store into every route. Return only the collections the current page needs.

Desktop hosts can also register custom-protocol API handlers, for example
`/api/contacts/:id`, so existing browser code can keep using `fetch("./api")`
while packaged apps mutate Rust-owned state in memory.
Native API request bodies are limited to 1 MiB. At most 16 ordinary application
jobs are admitted at once; overload returns HTTP 503 rather than waiting in an
unbounded queue. Handle failures explicitly. Do not automatically retry mutations
unless the application defines an idempotency contract. Cancellation stops
delivery but does not undo a Rust callback that already started.

## Security and performance defaults

- The runtime loads from a custom app origin.
- Navigation outside the app origin is denied unless explicitly allowed.
- Packaged assets are immutable and served from the bundle resource root.
- Asset URLs may percent-encode filenames. Indexed assets retain the same
  root confinement as source assets, including symlink checks on opened files.
- Build/package output paths are rejected when they overlap input directories.
- Protocol data, CSS maps, and asset metadata are shared by reference.
- Generated runners default to bundle-only execution. Keep source support and
  development inspection disabled for distribution unless explicitly needed.

Measure the packaged release app with representative data. Time page readiness,
not just process creation, and distinguish warm-cache launches from cold-cache
launches. On macOS, physical footprint is more useful than raw RSS; include the
WebKit content, networking, and GPU processes instead of reporting only the
Rust host.

## Shell extension points

Desktop manifests include a `shell` object for app icons, menus, and tray icons.
Unknown shell fields are rejected. A configuration type
does not imply backend support: check the capability table above before enabling
an optional feature. Configure shell defaults in the manifest or explicitly
replace them with `DesktopAppBuilder::shell(...)`.

Compose application services through Rust route/API/IPC handlers and lifecycle
subscriptions. A reusable service can return its subscription guard so its
caller controls the lifetime.

For a custom host or native embedding, implement `DesktopFrameBackend` and call
`run_frame_with(frame, &backend)`. This entry point validates the frame's
requested window and shell features against the backend's capabilities before
invoking it. It is available without the SDK's `native` feature. A custom backend
must retain the owning frame until its loop and callbacks finish; dropping it
ends event registration and command submission.

Read configuration through `frame.app_id()`, `frame.runtime()`, `frame.window()`,
and `frame.shell()`; configuration fields are not publicly mutable. Configure
window styling on `DesktopAppBuilder` before `build()`. A frame constructed
directly from an existing runtime must use the same titlebar/background styling
as that runtime. Clone `frame.window_handle()` for runtime window commands.

Custom protocol consumers must match `DesktopProtocolResponse::body` as
`DesktopResponseContent::Bytes` or `File`. Keep buffered response ownership,
including any reservation, until the native consumer releases it. File responses
own an open, length-bounded reader and should be streamed, not reopened by path.
`body.as_bytes()` returns `None` for files; `body.into_bytes()` is an explicit,
fallible materialization option for non-native consumers. The default asset
length limit remains 32 MiB, without requiring an asset-sized Rust buffer.

## Web inspector

Pass `--devtools` to `webui desktop build` or `webui desktop run` to mark the
desktop webview as inspectable. On macOS, open Safari and enable Safari >
Settings > Advanced > Show features for web developers, then use Safari's
Develop menu to inspect the running app.
