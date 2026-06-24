# microsoft-webui-desktop-cli

Desktop sidecar backend for running and packaging WebUI applications.

This crate provides the `webui-desktop` sidecar binary used by
`webui desktop ...`. It is not the user-facing CLI brand; users should invoke
desktop tooling through `webui desktop`. Keeping this sidecar separate keeps the
default `webui` dependency tree lean for build/serve/inspect users.

Use `webui desktop package --runner <path>` for Rust-first apps that register
route providers or typed IPC commands in an app-specific executable. Omitting
`--runner` packages the generic sidecar for file-backed/static seed-state
bundles.

For app roots, the public command remains:

```bash
webui desktop package ./my-app --target macos-app --out ./packages
```

The sidecar reads `webuiDesktop` from `package.json`, runs configured web build
scripts, builds the app-specific Cargo runner crate, stages non-generated assets,
builds the bundle, and packages the runner-backed app.
