# microsoft-webui-wasm

WebAssembly bindings for the [WebUI](https://github.com/microsoft/webui) framework, built with `wasm-bindgen`.

## Overview

`microsoft-webui-wasm` can be built as three browser bundles:

| Feature | Bundle | Exports |
|---------|--------|---------|
| `handler` | `webui_wasm_handler.js` | `Protocol` |
| `parser` | `webui_wasm_parser.js` | `build_protocol` |
| `all` | `webui_wasm_all.js` | Parser and handler exports |

The default feature is `all`, which powers the online playground. Consumers that only need to render prebuilt protobuf protocol bytes should use the handler bundle to avoid shipping parser code.

Construct `Protocol` once from protocol bytes. It exposes `render`,
`renderStream`, `renderPartial`, `renderComponentTemplates`, `tokens`, and
`streamResponse`.
Streaming callbacks are coalesced around a 16 KiB target before crossing into
JavaScript.

`Protocol.streamResponse(entry, requestPath, options)` returns a host-driven
`StreamingSession`. State and patch arguments are JSON strings. `start`,
`resume`, and `advance` return:

```js
{
  bytes: Uint8Array,
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
enqueue the checkpoint immediately. `advance` renders following parent or tail
bytes, so no sibling boundary workaround is needed.

`mode` is `"final"` (the default) or `"updatable"`. A committed updatable
occurrence accepts `update(instanceId, patch)`, including between its `resume`
and `advance`, and returns a `Uint8Array`. Optional boundary keys preserve JSON
identity: string keys are JavaScript strings and finite numeric keys are
JavaScript numbers. A key is required when one component-owned declaration is
reached from multiple static callsites in one entry traversal. Boundaries are
discovered through entries, reusable components, runtime branches, and selected
routes. A boundary-bearing subtree under `<for>` fails with
`boundary-in-repeat`; one boundary may wrap the whole `<for>`. The step with
`done: true` already contains terminal and document-tail bytes.

## Building

```bash
cargo xtask build-wasm
```

This writes the three generated bundles under `docs/.webui-press/public/wasm/`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT - Copyright (c) Microsoft Corporation.
