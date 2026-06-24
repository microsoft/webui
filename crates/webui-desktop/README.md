# microsoft-webui-desktop

Rust-native desktop runtime primitives for WebUI applications.

This crate owns the runtime-neutral pieces: desktop bundle metadata, safe
custom-protocol routing, startup rendering, bounded asset reads, and protobuf
IPC dispatch. The `webui-desktop` binary wires these primitives to the selected
native webview backend.

Rust-first packaged apps should load bundles with
`DesktopRuntime::from_bundle_config` so the executable can keep route providers
and typed IPC handlers in Rust while reusing immutable `protocol.bin` and asset
files from the bundle.
