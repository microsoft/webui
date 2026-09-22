# Actual-native IPC fixture

Internal native integration test for macOS, Windows and Linux, not a public demo.
No localhost server, browser stub, focus manipulation, foreground requirement, or
latency claim. Platform planning is portable; actual execution requires that
platform's native webview runtime and a usable GUI session.

Prerequisites: current `target/debug/webui-desktop` CLI built with `cli`, existing
`protoc`, and the repository's pinned `packages/webui-desktop/node_modules`
dependencies. No root manifest entry is needed: this fixture has its own workspace.

From the repository root:

```sh
python3 crates/webui-desktop/tests/fixtures/native-ipc/run.py
```

The harness invokes the real CLI generator and its check mode, compiles the native
Rust host with its committed lockfile offline, typechecks and bundles generated
TypeScript against the SDK-reserved runtime asset,
and runs the exact same copied binary with `DesktopApp::from_source` and
`DesktopApp::from_bundle`. Every invocation receives fresh output directories.
The SDK packager creates `macos-app`, `windows-portable`, or `linux-portable`
according to `DesktopPlatform::current()`. The packaged executable discovers its
resources through `find_packaged_resources_dir`, checked against SDK packaging
metadata. The harness verifies identical hashes for the source and packaged
executable copies. Windows uses `.exe` executables and the `.cmd` protoc plugin;
TypeScript runs through Node rather than a platform-specific shell shim.
The bundle's separate original build input is removed before either native run.
Keep this fixture's registry dependency versions aligned with the root
`Cargo.lock` when updating dependencies, so native acceptance exercises the same
versions as the product build. Use `cargo update --manifest-path` with
`--precise` rather than editing either lockfile.

The protocol asserts:

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
  request made through the old Rust session.
- Startup notification registration after navigation and a successful nested
  Rust-to-JS RPC in that new document.
- Idempotent explicit JS connection close, settlement of an in-flight request
  with `closed`, rejection of new requests, and recovery after a second full
  document load.
- Hash navigation and History API SPA state changes preserving the exact Rust
  session generation through generated typed RPCs.
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

`--timeout 60` bounds each native run. `--skip-build` is for a host binary that
has already been rebuilt; generated bindings are still checked. No sleep is used
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
Python 3.9+, `protoc` on PATH (including its standard protobuf imports), and the
pinned `ts-proto`/protobuf dependencies already declared in the workspace.
The root CLI must include `cli`; the standalone fixture compiles both `native`
and `source`. Preparation from the repository root:

```sh
pnpm install --frozen-lockfile
cargo build --locked -p microsoft-webui-desktop --features cli --bin webui-desktop
cargo fetch --locked --manifest-path crates/webui-desktop/tests/fixtures/native-ipc/Cargo.toml
```

The fetch seeds platform dependencies for the harness's offline locked build.
Embedded SDK assets must already be current, as for the production SDK build.

macOS requires Xcode command-line tools and an ordinary logged-in GUI session:

```sh
python3 crates/webui-desktop/tests/fixtures/native-ipc/run.py --timeout 60
```

Windows requires MSVC build tools, a Windows SDK, the WebView2 Evergreen Runtime,
`protoc.exe` on PATH and an interactive desktop-capable runner. In PowerShell:

```powershell
python crates/webui-desktop/tests/fixtures/native-ipc/run.py --timeout 60
```

Linux requires GTK >= 4.10 and WebKitGTK 6.0 >= 2.42. Ubuntu 24.04 CI package
dependencies are `build-essential`, `pkg-config`, `protobuf-compiler`,
`libgtk-4-dev`, `libwebkitgtk-6.0-dev`, `apparmor`, `dbus-x11`, `xvfb`, and `xauth`. Use
standard sandbox-enabled WebKitGTK in a non-root CI account:

```sh
GDK_BACKEND=x11 xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- \
  python3 crates/webui-desktop/tests/fixtures/native-ipc/run.py --timeout 60
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

Pure cross-platform planning, without starting any GUI or claiming native proof:

```sh
python3 -m unittest discover -s crates/webui-desktop/tests/fixtures/native-ipc -p test_plan.py -v
python3 crates/webui-desktop/tests/fixtures/native-ipc/run.py --plan win32
python3 crates/webui-desktop/tests/fixtures/native-ipc/run.py --plan linux
```

## Current verification boundary

Actual WKWebView source and SDK-packaged `.app` executions have demonstrated
explicit BFCache reconnection on both back and forward, with cached generations
3/4 replaced by 5/6 and old connections/callbacks staying retired.
The latest hardened run passed packaged mode but failed source mode: the outgoing
`history-return` pending request rejected with `transport`, not the expected
`navigated`/`closed`. That race remains a failing regression, not an accepted
terminal-code fallback. An earlier pass-shaped report accompanied by an
unhandled rejection is not clean evidence. The fixture does not alter production
cache/admission behavior. Windows and Linux executable/layout planning has unit
coverage, but native Windows/Linux execution has not been performed locally.
