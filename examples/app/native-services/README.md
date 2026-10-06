# Native services demo

This WebUI 0.0.30 app demonstrates the opt-in Rust-host directory picker,
native dialogs, visible-webview PNG capture, and verified PNG clipboard write.
It deliberately uses four narrow same-origin HTTP actions rather than granting
the page generic native IPC or filesystem access. Each launch generates a
private capability in the native startup URL, removes it from the visible URL
after load, and requires it on every action so another local process cannot
invoke the loopback endpoints by copying a fixed request header.

On macOS or Windows, from the repository root:

```bash
pnpm --dir examples/app/native-services start
```

The clipboard action becomes available after a successful capture. Linux
renders the app but returns the SDK's typed unsupported errors for these
platform-specific capabilities.

The browser tests mock only the OS boundary so loading, cancellation, success,
preview, and error behavior remain deterministic:

```bash
cargo xtask e2e
```
