# Desktop incoming host URL boundary

With the `native-url-activation` Cargo feature, a macOS `LocalServerFrame` can
register one `on_url_activation` handler before the frame runs. The feature
includes `local-server` and selects the existing general URL parser.
Plain `local-server` exposes neither the activation method/types nor the AppKit
selector and retains no activation queue/readiness state. `native-services`
independently retains its URL parser dependency, without enabling incoming
activation. Callers of the previously implicit, unpublished activation API must
explicitly select `native-url-activation`; feature omission is a compile-time
API absence, not a runtime `Unsupported` fallback. With the feature selected,
non-macOS targets retain the explicit `Unsupported` registration error.

Its AppKit
`application:openURLs:` adapter passes bounded validated custom-scheme URLs
only to that window's Rust host callback, off the UI thread. The default
bundle/source frames and local-server frames without a handler do no URL work.
The adapter does not navigate the webview, authorize application actions,
expose URL content through native IPC, or synthesize commands.

The registered scheme is a bounded lower-case non-reserved custom identifier.
One AppKit delivery admits at most eight URLs; each is at most 2,048 UTF-8
bytes, has that scheme and a host authority, and has no raw or percent-encoded
controls, credentials or fragment. Before window readiness, at most eight
validated URLs are retained; ready activations enter a bounded worker queue.
Overflow and retirement are rejected without exposing URL content in logs.
Window close and host retirement end admission. An activation already dequeued
may race close, so the host must check its own current lifetime before acting;
the SDK cannot undo a callback that has begun.

The SDK does not own `CFBundleURLTypes`, Launch Services registration, default
handlers, startup arguments, or second-instance forwarding. Direct invocation
of the AppKit delegate in a controlled fixture tests adapter association, not
whether macOS actually sends cold or warm URLs to an app. OS-delivery proof
requires an independently registered and packaged application fixture.
