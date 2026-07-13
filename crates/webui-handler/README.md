# microsoft-webui-handler

High-performance template renderer for the [WebUI](https://github.com/microsoft/webui) framework. Consumes the compiled binary protocol and produces HTML output at request time.

## Overview

`microsoft-webui-handler` evaluates expressions, resolves state bindings, and renders full or partial HTML responses from pre-compiled WebUI protocol buffers — with no JavaScript runtime required.

## Key Functions

### `route_handler::render_partial`
Returns a complete client-navigation response with active-route projected
state. Use `render_partial_metadata` only when producing state-free NDJSON
chunk 1.

### `route_handler::render_component_templates`
Returns compiled templates and CSS for specific components by tag name. Used for on-demand loading of components not in the route tree (dialogs, overlays). Supports inventory-based deduplication.

```rust
let result = route_handler::render_component_templates(
    &protocol,
    &["settings-dialog", "notification-panel"],
    &inventory_hex,
);
// Returns: { templates: [...], templateStyles: [...], inventory: "..." }
```

Available via all bindings: Rust (direct), Node (`renderComponentTemplates`), WASM (`render_component_templates`), FFI (`webui_render_component_templates`), npm (`@microsoft/webui` — `renderComponentTemplates`).

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
