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
