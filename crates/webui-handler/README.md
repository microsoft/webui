# microsoft-webui-handler

High-performance template renderer for the [WebUI](https://github.com/microsoft/webui) framework. Consumes the compiled binary protocol and produces HTML output at request time.

## Overview

`microsoft-webui-handler` evaluates expressions, resolves state bindings, and renders full or partial HTML responses from pre-compiled WebUI protocol buffers — with no JavaScript runtime required.

## Key Functions

### `Protocol::render_partial`
Returns a complete client-navigation response with active-route projected
state.

### `Protocol::render_component_templates`
Returns compiled templates and CSS for specific components by tag name. Used for on-demand loading of components not in the route tree (dialogs, overlays). Supports inventory-based deduplication.

```rust
let result = protocol.render_component_templates(
    &["settings-dialog", "notification-panel"],
    &inventory_hex,
);
// Returns: { templates: [...], templateStyles: [...], inventory: "..." }
```

Available via all bindings: Rust (`Protocol::render_component_templates`), Node/WASM/npm (`Protocol.renderComponentTemplates`), and FFI (`webui_protocol_render_component_templates`).

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
