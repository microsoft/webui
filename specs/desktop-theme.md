# Native desktop theme contract

The `native-services` Rust capability is local to one owned local-server
window. Only a trusted host may set its native theme; it persists the chosen
preference and emits initial SSR/CSS independently. No automatic renderer
global, script, or application IPC method is installed. A new document or IPC
connection reads `current_theme()` on admission: `ThemeChanged` is an
incremental notification, not an initialization guarantee.

On macOS, the UI-thread operation applies Aqua/DarkAqua to the exact NSWindow;
System removes the override. The attached WKWebView inherits the window
appearance. Native events and the snapshot use that view's effective
appearance rather than NSApplication's global appearance. Only one request
may be queued per window, with typed busy, cancellation, closed, unavailable,
and deadline errors; a timeout does not imply a native operation was reversed.
Navigation invalidates pending work but never clears an applied preference.
Host-owner revocation rejects queued appearance work before the AppKit close
wake drains, even when the window is still live; it cannot roll back a
native call already in progress.
The host must read the snapshot after a timeout before retrying.

Windows and Linux decline `set_theme` and `current_theme` with
`ThemeUnsupported`. The existing Windows `GetSysColor(COLOR_WINDOW)` event
is a system-color approximation, not a reliable application theme source or
proof that WebView2 and the native caption agree. Do not expose it as a
coherent per-window snapshot. WebView2 runtime 122+ supports the official
`ICoreWebView2Profile::put_PreferredColorScheme(AUTO/LIGHT/DARK)` API, but
the one-window/profile ownership and an OS-supported source for System mode
must be verified before a Windows adapter may report success.

## Windows native qualification plan

On real x64 and ARM64 Windows runners with WebView2 runtime 122 or later:

1. Verify `ICoreWebView2Profile` can be queried from this window's WebView2
   and determine whether another root sharing the profile changes when its
   `PreferredColorScheme` changes. A shared profile affecting unrelated roots
   is a scope gap that must be surfaced, not hidden.
2. With the OS preference independently controlled, compare System's
   effective value against the supported Windows app-theme source, WebView2
   `matchMedia('(prefers-color-scheme: dark)')`, the native caption, and OS
   dialogs. Do not use `GetSysColor(COLOR_WINDOW)` as that app-theme source.
3. Set Light/Dark explicitly, verify the WebView2 profile property and DWM
   immersive caption mode, then capture normal and narrow screenshots
   including the caption. Retry navigation and admission to prove the current
   snapshot initializes a newly loaded page even if an event was dropped.
4. Check isolated and shared profiles, denied/failed native calls, navigation
   during a queued operation, teardown, and the earliest supported Windows
   version. Do not claim Windows implementation or native verification from
   cross-compilation alone.
