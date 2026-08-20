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
`StreamingSession`. Call `start(stateJson)` once, then
`resume(instanceId, stateJson, mode)` for every discovered occurrence. State
and patch arguments are JSON strings. Both step methods return:

```js
{
  bytes: Uint8Array,
  done: boolean,
  boundary?: { instanceId, declarationId, owner, name, key }
}
```

`mode` is `"final"` (the default) or `"updatable"`. A committed updatable
occurrence accepts `update(instanceId, patch)`, which returns a `Uint8Array`.
Boundary keys preserve JSON identity: string keys are JavaScript strings and
finite numeric keys are JavaScript numbers. Boundaries are discovered through
entries, reusable components, runtime branches, loops, and selected routes.
The step with `done: true` already contains terminal and document-tail bytes.

## Building

```bash
cargo xtask build-wasm
```

This writes the three generated bundles under `docs/.webui-press/public/wasm/`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT - Copyright (c) Microsoft Corporation.
