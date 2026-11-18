# microsoft-webui-node

Node.js native addon for the [WebUI](https://github.com/microsoft/webui) framework, built with [napi-rs](https://napi.rs).

## Overview

`microsoft-webui-node` compiles twice into separate platform-specific `.node`
addons without spawning a subprocess:

| Cargo feature | Packaged addon | API |
|---|---|---|
| `handler` (default) | `webui.node` | `Protocol`, `StreamingSession`, `inspect` |
| `build` | `webui-build.node` | `build`, `prepareBuild`, `PreparedBuild`, `bundleClient` |

Exactly one feature must be selected. Compile the build variant in a separate
target directory because both variants use the Cargo library name `webui_node`:

```bash
cargo build -p microsoft-webui-node --no-default-features --features handler
cargo build -p microsoft-webui-node --no-default-features --features build --target-dir target/node-build
```

`@microsoft/webui` loads only the handler addon, without linking the template
compiler or browser bundler. `@microsoft/webui/build.js` loads the build addon.
The build variant uses `webui::browser_build` through the optional
`microsoft-webui/browser-build` feature; semantic projection remains in
`microsoft-webui-projection`. Protocol buffers are ordinary owned bytes shared
between the addons, not addon-specific handles.

## Host-driven streaming

`Protocol.streamResponse(entry, requestPath, options)` returns a
`StreamingSession`. State and patch arguments are JSON strings. `start`,
`resume`, and `advance` return:

```js
{
  bytes: Buffer,
  done: boolean,
  boundary?: { instanceId, declarationId, owner, name, key }
}
```

| Method | Result |
|---|---|
| `start(stateJson)` | Shell bytes through the first descriptor or terminal |
| `resume(instanceId, stateJson, mode)` | Only the pending occurrence's bytes through its checkpoint |
| `advance()` | Following parent bytes through the next descriptor or terminal |
| `update(instanceId, patchJson)` | Projected state bytes for an updatable occurrence |

A descriptor means call `resume`; no descriptor with `done: false` means call
`advance`; `done: true` means complete. Boundary-only `resume` lets the host
flush the checkpoint immediately. `advance` renders following parent or tail
bytes, so no sibling boundary workaround is needed.

`mode` is `"final"` (the default) or `"updatable"`. A committed updatable
occurrence accepts `update(instanceId, patch)`, including between its `resume`
and `advance`, and returns a `Buffer`. Optional boundary keys preserve JSON
identity: string keys are JavaScript strings and finite numeric keys are
JavaScript numbers. A key is required when one component-owned declaration is
reached from multiple static callsites in one entry traversal. Boundaries are
discovered through entries, reusable components, runtime branches, and selected
routes. A boundary-bearing subtree under `<for>` fails with
`boundary-in-repeat`; one boundary may wrap the whole `<for>`. The step with
`done: true` already contains terminal and document-tail bytes.

## Benchmark

The runtime benchmark in
[`examples/integration/node-addon-bench`](../../examples/integration/node-addon-bench)
measures protocol construction, buffered rendering, and streaming callbacks across
the real V8/N-API boundary:

```bash
cargo xtask bench node-addon
```

It supports the repository-wide `--save-baseline NAME` and `--baseline NAME`
workflow documented in [`BENCHMARKS.md`](../../BENCHMARKS.md).

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT - Copyright (c) Microsoft Corporation.
