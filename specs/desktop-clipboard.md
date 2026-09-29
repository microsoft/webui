# Host-only native PNG clipboard completion

On macOS, a trusted local-server `NativeServices` handle may request one
clipboard write for an existing opaque `CapturedContent` token. Admission
checks the owning window, finished document epoch, viewport revision, and
retained resource identity. Windows/Linux explicitly reject the operation.
The feature adds no renderer method, browser global or URL opener.

One pending write is admitted per window. The native UI dispatcher passes
only a small token across its queue. On delivery, it revalidates the token,
clones a bounded `Arc<Vec<u8>>` rather than the complete PNG. The exact-sized
encoded `Vec` moves into that private Arc without duplicating its bytes at
retention; native scheduling copies only the Arc header. The dispatcher drops
all capture locks before calling one public AppKit helper. The helper observes the pasteboard
count, copies bounded NSData, and under the ClipboardState lock
revalidates the **exact capture token, owner lifetime and deadline** while
holding CaptureState's lock only for this non-blocking queued-to-writing
transition. No allocation or scheduling intervenes before `clearContents`;
no capture lock is held during AppKit I/O. The same helper accepts either
the production general pasteboard or a test-only unique private pasteboard:
clear, write immutable `public.png` data, verify the returned Boolean,
check a stable change count and read back identical bytes. NSData and
pasteboard operations may make bounded input-sized native copies; the PNG
resource itself is at most the capture budget. An AppKit refusal or
readback mismatch never destroys the retained capture, allowing retry.

Cancellation by navigation, retake, explicit capture release, owner
revocation or window close wins while queued and blocks mutation. A synchronous
AppKit write already started cannot be retracted; completion rechecks the
window/document token, and an obsolete callback never reports success to a
new document. A finite deadline completes the awaitable while retaining
the native Busy reservation until its callback or teardown. At most eight
short-lived clipboard deadline threads per window may be reserved; capacity
exhaustion fails explicitly instead of accumulating sleepers. Dropped
requests never become a success. A host that elects to open a validated
URL must **await successful clipboard completion first** and own that
later decision outside this SDK.

The controlled tests use only `NSPasteboard.pasteboardWithUniqueName`, including
the public `NativeServices` admission → Dispatch → **same production helper**
→ `public.png` Boolean, change-count and byte-for-byte readback. A
pre-write cancellation test proves the private board's change count remains
unchanged. Tests release the private name and never read or overwrite the
user's general clipboard. Although production
calls the exact same helper on `generalPasteboard`, OS permission,
contention and user-environment behavior on that global board remain
unverified until separately authorized and observed.
