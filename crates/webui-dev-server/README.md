# microsoft-webui-dev-server

Shared dev-server primitives for WebUI tooling.

This crate provides primitives that any WebUI dev server can compose:

- **`LiveReload`** — Server-Sent Events broadcaster. Cheap to clone, ships
  its own actix handler, generates the matching browser-side `<script>`,
  and injects it before `</body>`.
- **`watch::spawn_watcher`** — Debounced filesystem watcher backed by
  `notify`. Calls a closure once per debounce window.
- **`spawn_rebuild_worker`** — Coalesces watcher ticks and runs rebuilds on an
  owned `RebuildWorker`. Give the watcher a sender from `worker.sender()`, keep
  the worker alive while serving, then stop the watcher and call
  `worker.shutdown()` to wait for an active rebuild. Dropping the worker also
  stops and joins it; pending ticks are discarded.
- **`path::*`** — URL path utilities for serving files under a `basePath`
  prefix without traversal vulnerabilities.

Used by `microsoft-webui-press` (static-site preview) and the `webui serve`
CLI command (HMR for app development).

This crate is not intended for direct end-user consumption; its public API
follows the WebUI workspace versioning rather than semver.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.