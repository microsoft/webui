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
WebView2 Runtime and the target-gated Win32 backend; validate runtime behavior
on Windows CI or a Windows developer machine with WebView2 installed.

Desktop app runners use one cross-platform frame API. Build a
`DesktopRuntime`, then call `webui_desktop_runner::run_runtime(runtime, window)`
or construct a `DesktopFrame` and call `run_frame`. Do not branch on
`cfg(target_os)` in app code to select macOS, Windows, or Linux modules. The
runner crate dispatches to the current platform backend and keeps future shell
features on one shared contract.

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

This builds the entry template and opens it in the native system webview. Add a
Rust host only when the app needs dynamic route state, native IPC, or direct
window control. To create the progressive starting point, run:

```bash
webui desktop init ./my-app
cd ./my-app
webui desktop run ./src
```

The command creates `src/index.html`, `package.json`, and a `desktop/` runner
crate. It never overwrites those generated files unless `--force` is passed.

## Window options

`webuiDesktop` and `WindowOptions` support `title`, `width`, `height`,
`min_width`, `min_height`, `max_width`, `max_height`, `resizable`, `maximized`,
`fullscreen`, `always_on_top`, `center`, `background`, `titlebar`, `effect`,
`remember_state`, and `devtools`. `background` is `#rrggbb` or `#rrggbbaa` and
is painted before the first web content paint.

`titlebar` is one of `native`, `hidden-inset`, `overlay` with a `height`, or
`none`. `effect` is one of `none`, `vibrancy`, `acrylic`, `mica`, or `tabbed`.
Unsupported requested options fail validation before the native shell starts;
they do not silently degrade. Each backend advertises exactly what it
implements, so asking for something unsupported is a startup error with an
actionable message rather than an option that quietly does nothing:

| Capability | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Lifecycle events | Yes | Yes | Yes |
| Titlebar styles (`hidden-inset`, `overlay`, `none`) | Yes | Yes | Yes |
| Window controls (`WindowHandle` commands) | Yes | Yes | Yes |
| Window effects | Yes (all four) | Yes (except `tabbed`) | No |
| Application menu | Yes | No | No |
| Tray icon | Yes | No | No |
| Jump list | No | No | No |
| Popovers | No | No | No |
| Downloads | No | No | No |

Each backend advertises the specific effects it implements, and requesting one
it does not advertise is a startup error:

| `effect` | macOS | Windows | Linux |
| --- | --- | --- | --- |
| `none` | Yes | Yes | Yes |
| `vibrancy` | Yes | Yes, as acrylic | No |
| `acrylic` | Yes | Yes | No |
| `mica` | Yes | Yes | No |
| `tabbed` | Yes | No | No |

Where a platform has no exact analogue but a close one, it maps rather than
fails: `vibrancy` on Windows renders through the acrylic system backdrop.
Where it has no analogue at all, it does not advertise the effect, so the
mismatch surfaces as a validation error instead of a silent no-op - `tabbed`
is a macOS titlebar treatment with no Windows equivalent. Linux advertises no
effects, because GTK4 exposes no portable blur, and no tray, because GTK4
removed `GtkStatusIcon`. On Windows, `overlay` removes the native caption
buttons, so web content must draw its own.

`platform_capabilities()` in `webui-desktop-cli` is the source of truth; the
table above mirrors it. Availability also depends on the desktop environment:
a Linux compositor may ignore `always_on_top`, and Wayland controls window
placement, so `center` is advisory there.

For non-native titlebars, the runtime injects these CSS custom properties:
`--webui-titlebar-inset-start`, `--webui-titlebar-inset-end`,
`--webui-titlebar-height`, and, when `background` is set,
`--webui-window-background`. Use them in layout rather than hardcoding
platform offsets. For example, macOS controls reserve a leading inset of about
78px and Windows caption buttons reserve a trailing inset of about 138px.

Mark an application-drawn drag area with `webui-drag`; add `webui-no-drag` to a
nested interactive element. Double-clicking a drag area toggles maximize. Do
not use Chromium's `-webkit-app-region: drag`: WKWebView and WebKitGTK do not
implement that Chromium-specific property, and WebView2 apps should use the
WebUI drag contract for cross-platform behavior.

## Lifecycle and native window control

Rust handlers are registered with `DesktopFrame::on_event` and return
`EventResponse::Continue` or `EventResponse::PreventDefault`. The latter can
cancel `WindowCloseRequested` and `NavigationRequested`; handlers must not block
the backend UI thread. The complete event set is `Ready`, `WindowResized`,
`WindowMoved`, `WindowMaximized`, `WindowUnmaximized`, `WindowMinimized`,
`WindowRestored`, `WindowEnteredFullscreen`, `WindowLeftFullscreen`,
`WindowFocused`, `WindowBlurred`, `WindowCloseRequested`, `WindowClosed`,
`ThemeChanged`, `ScaleFactorChanged`, `NavigationRequested`,
`NavigationCompleted`, and `Exiting`.

The same events are mirrored into web content as cancelable-aware `CustomEvent`s
on `window`: `webui:ready`, `webui:window-resized`,
`webui:window-moved`, `webui:window-maximized`,
`webui:window-unmaximized`, `webui:window-minimized`,
`webui:window-restored`, `webui:window-entered-fullscreen`,
`webui:window-left-fullscreen`, `webui:window-focused`,
`webui:window-blurred`, `webui:window-close-requested`,
`webui:window-closed`, `webui:theme-changed`,
`webui:scale-factor-changed`, `webui:navigation-requested`,
`webui:navigation-completed`, and `webui:exiting`. Event data is in `detail`.
The cancelable Rust events are the close and navigation requests; web listeners
can observe those events, while the Rust handler is the authority for native
cancellation.

Linux never emits `WindowMoved`/`webui:window-moved`. GTK4 removed the GTK3
window-position query APIs, and Wayland deliberately does not let a client
read its own toplevel position, so there is no API to source this event from;
handlers that depend on it should treat its absence on Linux as expected
rather than a bug.

`DesktopFrame::window_handle` exposes a `Send + Sync` `WindowHandle` that queues
UI-thread commands: `set_title`, `set_size`, `minimize`, `maximize`,
`unmaximize`, `fullscreen`, `center`, `focus`, `close`, `start_drag`, and
`set_always_on_top`. Queue errors are returned to the caller. Set
`remember_state: true` to persist validated position, size, and maximized state;
stale or off-screen state is rejected on the next launch.

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

Add `--release` to `desktop package` when measuring or distributing an optimized
Rust runner. If invoking the CLI through Cargo, the flag belongs after `--`:

```bash
cargo run -p microsoft-webui-cli -- desktop package ./my-app \
  --target macos-app --out ./packages --release
```

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
loads the packaged bundle with `DesktopRuntime::from_bundle_config`. Omitting
`--runner` packages the generic sidecar and is appropriate only for
file-backed/static seed-state bundles.

App-specific runners can use the shared resource helper instead of OS-specific
bundle paths:

```rust
fn main() -> anyhow::Result<()> {
    let (runtime, window) = match webui_desktop_runner::find_packaged_resources_dir() {
        Some(resources) => load_packaged_runtime(&resources)?,
        None => build_source_runtime()?,
    };

    webui_desktop_runner::run_runtime(std::sync::Arc::new(runtime), window)
}
```

For cold start, load `manifest.webui-desktop.json` once when the runner needs
window or shell metadata, then pass it into
`DesktopRuntime::from_bundle_config_and_manifest(config, manifest)`. Bundle
assets are served through the manifest integrity index, avoiding per-request
metadata reads for immutable packaged assets.

The Rust packager currently writes:

| Target | Output |
|--------|--------|
| `macos-app` | Runnable `<AppName>.app` with a WKWebView launcher and bundled resources |
| `windows-portable` | Portable folder layout for a Windows runner and bundled resources |
| `linux-portable` | Portable folder layout for a Linux runner and bundled resources |

Installer targets return actionable diagnostics for the required platform
tooling:

| Target | Required tooling |
|--------|------------------|
| `windows-msi` | WiX 3.11 and `signtool.exe` |
| `windows-msix` | Windows SDK `makeappx.exe` and `signtool.exe` |
| `linux-appimage` | `appimagetool` |
| `linux-deb` | Debian package writer |
| `linux-rpm` | RPM package writer |

## IPC model

Desktop IPC is protobuf-first. Web content sends protobuf request bytes to a
reserved custom-protocol endpoint and receives protobuf response bytes. The
Rust dispatcher is allowlisted, validates payload size before dispatch, and
returns structured protobuf errors instead of panicking.

## Rust route state

Desktop apps that need dynamic state write route data in Rust. Register route
providers on the desktop host:

```rust
let runtime = webui_desktop::DesktopApp::builder(build_options)
    .state_value(seed_state)
    .asset_root("./dist")
    .route("/", |ctx| {
        Ok(serde_json::json!({ "page": "dashboard" }))
    })?
    .route("/contacts/:id", |ctx| {
        let id = ctx.param("id").unwrap_or("");
        Ok(contact_detail_state(id))
    })?
    .build()?;
```

The runtime uses these providers for full HTML renders and WebUI router partial
requests. Provider errors are surfaced instead of falling back silently. The CLI
`--state` path remains a simple seed-state fallback.

For route-backed apps, keep mutable collections in shared Rust storage and
borrow them while preparing a route's view model. Use the render seed for
global settings and theme tokens rather than copying the complete application
store into every route. Return only the collections the current page needs.

Desktop hosts can also register custom-protocol API handlers, for example
`/api/contacts/:id`, so existing browser code can keep using `fetch("./api")`
while packaged apps mutate Rust-owned state in memory.

## Security and performance defaults

- The runtime loads from a custom app origin.
- Navigation outside the app origin is denied unless explicitly allowed.
- Packaged assets are immutable and served from the bundle resource root.
- Build/package output paths are rejected when they overlap input directories.
- Protocol data, CSS maps, and asset metadata are shared by reference.
- Development-only features are excluded from production bundles.

Measure the packaged release app with representative data. Time page readiness,
not just process creation, and distinguish warm-cache launches from cold-cache
launches. On macOS, physical footprint is more useful than raw RSS; include the
WebKit content, networking, and GPU processes instead of reporting only the
Rust host.

## Shell extension points

Desktop manifests include a `shell` object for native features such as app icons,
menus, Windows jump lists, popovers, and app-controlled downloads. Backends expose
only capabilities they can implement safely on the current OS; unsupported
features fail with actionable diagnostics rather than silently pretending to
work.

New shell features should be added to the frame/backend contract first. Each
platform backend then implements the same method or reports that the capability
is unavailable, so application developers keep using a single cross-platform API.

## Web inspector

Pass `--devtools` to `webui desktop build` or `webui desktop run` to mark the
desktop webview as inspectable. On macOS, open Safari and enable Safari >
Settings > Advanced > Show features for web developers, then use Safari's
Develop menu to inspect the running app.
