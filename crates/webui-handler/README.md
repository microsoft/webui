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

### Runtime-discovered streaming

`WebUIHandler::stream_response` returns a borrowed `StreamingResponse`.
`StreamingSession` is the owned byte-returning form used by host bindings.

```rust
let mut session = StreamingSession::new(handler, protocol, options)?;
let mut step = session.start(&initial_state)?;

while !step.done {
    step = match step.boundary.as_ref() {
        Some(boundary) => {
            let state = load_state(&boundary.owner, &boundary.name, boundary.key.as_ref())?;
            // Writes only this occurrence, through its checkpoint.
            session.resume(boundary.instance_id, &state, BoundaryMode::Final)?
        }
        // Writes the shell bytes up to the next occurrence or the terminal.
        None => session.advance()?,
    };
    write_and_flush(&step.bytes)?;
}
```

`start`, `resume`, and `advance` return `StreamStep { bytes, boundary, done }`,
and each step's bytes are one independently writable, independently flushed
segment. A descriptor contains response-local `instance_id`, stable
`declaration_id`, owner-local `name`, `owner`, and an optional string or numeric
`key`. `boundary: Some` waits for `resume`; `boundary: None` with `done: false`
means the committed occurrence flushed and the caller must `advance`; the done
step includes terminal and document-tail bytes.

`resume` is boundary-only: it writes the occurrence markers, body, checkpoint,
and sentinel, then returns. `advance` writes the parent or shell bytes that
follow through the next occurrence or terminal. This makes an early
component-local child independently flushable without an authored sibling
boundary.

Commit an occurrence as `BoundaryMode::Updatable` to call
`update(instance_id, patch)` later, including between its `resume` and the
following `advance`. Updates are projected state records and do not insert
markup. Component-local occurrences use generated parent spans so an early child
can hydrate before the parent tail.

A `<boundary>` may not appear inside a `<for>` repeat body (directly or through
a component, `<if>`, route, or outlet): the build rejects it with
`boundary-in-repeat`. Wrap the whole `<for>` in one boundary to pace the list as
a single region. Boundaries before and after a `<for>` are also valid. A
component-owned declaration reached from multiple static callsites in one entry
traversal still requires a unique string or numeric `key`.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
