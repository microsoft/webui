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
`renderStream`, `renderPartial`, `renderComponentTemplates`, and `tokens`.
Streaming callbacks are coalesced around a 16 KiB target before crossing into
JavaScript.

## Building

```bash
cargo xtask build-wasm
```

This writes the three generated bundles under `docs/.webui-press/public/wasm/`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT - Copyright (c) Microsoft Corporation.
