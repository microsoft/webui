# Actual-native IPC fixture

Internal native integration test for macOS, Windows and Linux, not a public demo.
No localhost server, browser stub, focus manipulation, foreground requirement, or
latency claim. Platform planning is portable; actual execution requires that
platform's native webview runtime and a usable GUI session.

Prerequisites: current `target/debug/webui-desktop` CLI built with `cli`, existing
`protoc`, and the repository's pinned `packages/webui-desktop/node_modules`
dependencies. The native runner is an example target of `microsoft-webui-desktop`
and uses the root workspace lockfile.

From the repository root:

```sh
cargo xtask native-ipc
```

The harness checks committed bindings with the real CLI generator's read-only
check mode before compiling, so stale fixtures fail rather than being silently
regenerated. After intentional generator changes, refresh both IPC fixtures with
`WEBUI_UPDATE_IPC_FIXTURE=1 cargo test -p microsoft-webui-desktop-build --test generate fixture_matches_generator`.
The harness compiles two optimized native Rust hosts against the root lockfile
offline: a source-enabled tooling runner and a runtime-only release consumer
with no default features. It typechecks and bundles generated TypeScript against
the SDK-reserved runtime asset. Source mode uses `DesktopApp::from_source`;
packaged mode uses the independently built consumer and `DesktopApp::from_bundle`.
Every invocation receives fresh output directories.
The SDK packager creates `macos-app`, `windows-portable`, or `linux-portable`
according to `DesktopPlatform::current()`. The packaged executable discovers its
resources through `find_packaged_resources_dir`, checked against SDK packaging
metadata. The harness verifies the packaged executable matches the runtime-only
consumer's SHA-256, records each runner's feature metadata, and rejects parser,
source build, generator, Tokio, Rayon, and CLI dependencies in the consumer's
normal/build dependency tree. Windows uses `.exe` executables and the `.cmd` protoc plugin;
TypeScript runs through Node rather than a platform-specific shell shim.
The bundle's separate original build input is removed before either native run.
Before packaged execution, the source app, staging bundle, and both unpackaged
runner copies are removed. Only package resources can supply the app. Both
native flows run optimized release code; this is correctness evidence, not a
performance measurement. A separate release no-IPC fixture exercises source and
bundle modes without compiling `application-ipc`, checks that IPC assets,
endpoints and the IPC bootstrap script are absent, and verifies ordinary API and
window closure. On Windows, the harness copies the Windows App SDK bootstrap
DLL and its license, notices, and provenance from `target/release` beside each
temporary runner, including the no-IPC example built under `target/release/examples`.
It preflights all companions before copying and checks that portable packaging
preserves their bytes. The Windows runtime bootstrap is required even when
application IPC is disabled.

The protocol asserts:

- Fetch-visible JavaScript and protobuf response MIME headers, including
  `Cache-Control: no-store` on rejected IPC requests.
- A first connection delayed beyond the default five-second handshake budget
  still succeeds; waiting before hello does not expire the document proof.
- JS typed Promise → Rust, including acknowledged `Empty`/`void` RPC completion.
- Rust typed Future → registered async JS handler.
- Rust notifications → JS subscriptions, including unsubscribe.
- JS notification → Rust startup handler → confirmation notification.
- Notification acceptance while its handler is explicitly blocked until a
  separate typed Release RPC, distinguishing acceptance from completion.
- `u64::MAX`/`bigint`, complete binary byte validation at 0, 16 KiB and 256 KiB.
- Invalid JS input, intentional Rust handler failure, cancellation observed by a
  handler drop guard and a typed acknowledgement, and subsequent successful calls
  on the same connection. The fixture does not assume an aborted future resumes.
- Full main-document navigation while a typed request is pending, cancellation
  of the retired Rust handler, fresh document generation, and rejection of a
  request made through the old Rust session. Where available, this transition
  uses the Navigation API, including WKWebView's nil native navigation identity;
  later transitions retain location and history traversal coverage.
- Startup notification registration after navigation and a successful nested
  Rust-to-JS RPC in that new document.
- Idempotent explicit JS connection close, settlement of an in-flight request
  with `closed`, rejection of new requests, and recovery after a second full
  document load.
- Hash navigation, History API SPA state changes, and same-document back/forward
  traversal preserving the exact Rust session generation through typed RPCs.
- Native disconnect observed before the next navigation: a one-shot
  `/fixture-disconnect-observation` custom-protocol request checks `is_closed()`
  on the typed host session previously captured by `LifecycleHold`. JS awaits a
  successful observation before navigating. This is not a retry or a local
  `connection.closed` proxy. An unsettled native disconnect fails immediately.
- Real `history.back()` and `history.forward()` between full documents, checking
  fresh generation, retired session rejection, and cancellation in each direction.
  On `pageshow.persisted`, the fixture explicitly calls `connectDesktop` again
  and awaits normal native admission. It does not authorize a second activation,
  replay requests, reload, or skip the traversal. Old JS connections must remain
  terminal; fresh renderer RPC/notification probes must not reach their retained
  handlers/subscriptions. Fresh-document history loads also run these probes,
  while `persisted_restores` records whether actual BFCache restores occurred.
- Typed final acknowledgement, owned-window close and normal native process exit.

Read `.runs/native-*/result.json`, alongside stdout/stderr logs and the immutable
binary/bundle. Successful scoped JSON is printed only after all protocol
assertions and the native `WindowClosed` lifecycle event. The harness also checks
normal exit and the exact SHA-256 before and after each run. Failures/timeouts
are not converted into passes; native assertion failures and unhandled JS errors
override any pass-shaped output. Cleanup signals only the exact child PID.

`--timeout 60` bounds each native run. Both runner variants are always rebuilt
to prevent stale artifacts from passing acceptance. No sleep is used
as readiness. GUI screenshot/foreground testing is deliberately not performed:
the fixture tests native protocol behavior, records document visibility, and
does not change a product interface. There are no automatic platform skips or
native retries. Unsupported platforms, unavailable displays/webview runtimes,
generator failures and permission errors fail explicitly. Do not use sandbox
disable flags, modify Xauthority, grant OS permissions through the harness, or
force application activation to make a failure pass.

The renderer counts in the final report describe the initial document's four
successful saves; `saves` and `startup_notifications` are cumulative Rust counts
across documents (five and two respectively).

`/fixture-diagnostic` is a diagnostic-only custom-protocol route for script-load
and failure logs, including failures before IPC admission. It does not dispatch
application methods or satisfy any of the four-flow assertions. On script failure
the fixture requests its own window close; the harness still fails without the
single fully asserted success report.

## CI preparation and commands

Use the repository Rust toolchain, Node 22+, pnpm with the checked-in lockfile,
and `protoc` on PATH, including its standard protobuf imports.
The root CLI must include `cli`; the standalone fixture always enables `native`
and `application-ipc`, and only its tooling runner enables `source`. Preparation
from the repository root:

```sh
pnpm install --frozen-lockfile
cargo build --locked -p microsoft-webui-desktop --features cli --bin webui-desktop
cargo fetch --locked
```

The fetch seeds platform dependencies for the harness's offline locked build.
Embedded SDK assets must already be current, as for the production SDK build.
All three CI platforms run `cargo xtask desktop-acceptance`
before example generation. This shared gate checks committed generated fixtures
and embedded assets, generated Rust/TypeScript consumers, browser contracts in
Chromium/Firefox/WebKit, and the isolated nonempty SDK feature matrix. It rejects
`WEBUI_UPDATE_IPC_FIXTURE` so a regeneration environment cannot mask drift.

macOS requires Xcode command-line tools and an ordinary logged-in GUI session:

```sh
cargo xtask native-ipc --timeout 60
```

Windows requires MSVC build tools, a Windows SDK, the WebView2 Evergreen Runtime
(122.0.2365.46 or later), the x64 Windows App Runtime 1.8
(8000.946.1701.0 or newer in the 1.8 family),
`protoc.exe` on PATH and an interactive desktop-capable runner. In PowerShell:

```powershell
cargo xtask native-ipc --timeout 60
```

Before typed IPC checks, Windows verifies that `fetch` remains native, response
metadata/cloning and HEAD semantics are intact, binary POST bodies round-trip,
and abort before dispatch, immediately after dispatch, and before body consumption
rejects with `AbortError`. A subsequent fetch must succeed. Dedicated and shared
workers fetch a runtime-only resource to exercise the required native request-source
filter. These checks run in both source and packaged modes.

Linux requires GTK >= 4.10 and WebKitGTK 6.0 >= 2.42. Ubuntu 24.04 CI package
dependencies are `build-essential`, `pkg-config`, `protobuf-compiler`,
`libgtk-4-dev`, `libwebkitgtk-6.0-dev`, `apparmor`, `dbus-x11`, `xvfb`, and `xauth`. Use
standard sandbox-enabled WebKitGTK in a non-root CI account:

```sh
GDK_BACKEND=x11 xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- \
  cargo xtask native-ipc --timeout 60
```

Ubuntu 24.04's AppArmor restriction on unprivileged user namespaces can make
WebKitGTK abort with `bwrap: setting up uid map: Permission denied`, followed by
`Failed to fully launch dbus-proxy`. The Linux CI step temporarily loads
`linux-apparmor` with `sudo apparmor_parser -r` and removes it with
`sudo apparmor_parser -R` in an exit trap, including on test failure. The profile
allows user namespaces only for this fixture's executable copies under `.runs`,
covering both source and SDK-packaged paths. WebKit's bubblewrap/seccomp sandbox
and the system-wide AppArmor restriction remain enabled; the harness itself
does not grant permissions. Do not disable either sandbox or the global
user-namespace restriction to run this fixture.

The existing GTK adapter disables page cache for IPC-enabled views. That does
not constitute a Linux history pass until this real test runs there.

Inspect platform paths without starting a GUI or claiming native proof:

```sh
cargo xtask native-ipc --plan win32
cargo xtask native-ipc --plan linux
```

## Current verification boundary

Actual WKWebView source and SDK-packaged `.app` executions have demonstrated
explicit BFCache reconnection on both back and forward, with cached generations
3/4 replaced by 5/6 and old connections/callbacks staying retired.
Optimized source and runtime-only packaged modes, plus both release no-IPC
modes, pass on the available macOS host. Both IPC runs observed two persisted
history restores. Unhandled rejections remain failures even alongside a
pass-shaped report; no terminal-code fallback was added. The fixture does not
alter production cache/admission behavior. Native Windows/Linux execution has
not been performed locally.
Contact Book primary/error screenshots remain a separate product acceptance
requirement; this protocol fixture does not provide them.
