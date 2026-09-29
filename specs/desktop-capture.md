# Host-owned visible-content capture

## Windows x64/ARM64 adapter

Windows WebView2 0.39.1 `ICoreWebView2::CapturePreview(PNG, IStream, completion)`
runs on the owning window STA. The host schedules via a generation-checked,
payload-free window wake, not page IPC. A custom writable COM stream caps
cumulative source writes and resident output to the configured PNG limit
(never above 12 MiB); its buffer is moved, not copied, into the existing
per-window captured-content resource on native completion. Full physical
viewport bounds are checked against the 1600×1200 / RGBA hard limit and
host-selected options before WebView2 encodes. Unlike WKSnapshot, WebView2's
CapturePreview offers no downsampling, so an oversized viewport is rejected,
not cropped. PNG signature, IHDR dimensions, and terminal IEND are checked
before retention. Navigation, viewport change, close and owner revocation
cancel delivery and discard pending stream bytes; native work keeps the busy
reservation until its callback returns, even after the 10-second logical
deadline. No Windows GUI execution or preview-iframe pixel verification has
been performed. Linux remains unsupported.

## macOS adapter and observed evidence

The local-server frame's opt-in `native-capture` capability (which includes
`native-services`) can create one opaque
native PNG resource for its **exact owning window and finished main-document
epoch**. This is a host-only API and cannot be invoked by an ungranted
renderer. Preview frame grants remain exact, unprivileged HTTP origins;
capture does not promote them to the main document or native IPC authority.

`WKWebView.takeSnapshot` receives the visible WK bounds, never an NSWindow or
scroll document rect. `WKSnapshotConfiguration.snapshotWidth` is in native
points; the macOS adapter accounts for backing scale and measures the actual
returned image pixels. It rejects zero, clipped/aspect-invalid, oversized,
or failed images before retaining bytes. A bounded AppKit RGBA bitmap is
converted to PNG using public zlib with a destination checked against
`compressBound` **before** allocation; no AppKit PNG encoder, TIFF, or browser
data URL is involved. At 1600×1200 RGBA the bitmap is 7,680,000 bytes, raw
filtered scanlines are 7,681,200 bytes, and the maximum PNG allocation is
7,683,613 bytes including framing. The host cannot infer
iframe readiness from a main-document finish: it must confirm preview HTTP
completion, iframe load and child-produced application readiness separately.
The native capture API itself does not execute a page script.

One native callback may be outstanding per window. A ten-second deadline
completes the Rust future but keeps the native reservation until WebKit's
callback or teardown; a late callback skips bitmap drawing and PNG encoding.
A dropped future or navigation cancels delivery with that same reservation
rule, so a late reply cannot be attributed to a new document. Page-provided
NSError descriptions are not allocated; only WK's numeric error code crosses
the host boundary.
Retake, release, actual navigation and window retirement discard any retained
PNG. The one existing macOS host-lifetime close callback first discards
retained capture bytes **synchronously on owner revocation**, then enqueues
the normal native window-close wake. This callback never invokes a host
Future waker while HostLifetime's lock is held; native teardown later
acknowledges any pending capture. Repeated owner revoke is idempotent, and
retry-close may schedule another wake without restoring bytes. Captured-content
metadata contains private window,
document and resource identity, with only measured dimensions and byte length
public. Reads copy at most one bounded binary chunk, report offset/EOF and
reject stale, cross-window or released resources.

No generated IPC method is installed automatically. A host that grants one
must independently authenticate/authorize each operation, pace chunk reads
with transport credit and account for encoded bytes within its existing IPC
limits. A complete PNG never belongs in one IPC JSON response. Linux returns
explicit `Unsupported`, not a screen-capture fallback.

The controlled **macOS 27 ARM64** fixture under
`crates/webui-desktop/tests/fixtures/native-capture*` observed actual colored
iframe pixels after independent child readiness and two WK paint settlement
snapshots. That evidence does not qualify macOS 13 or other untested runtime
versions; a mere cross-origin exception or main-frame
`NavigationCompleted` is not proof of a painted child.
