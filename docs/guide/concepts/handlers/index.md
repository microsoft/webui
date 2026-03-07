# WebUI Handlers

WebUI handlers are the bridge between the WebUI protocol and the final rendered HTML output. They process the protocol and render the content in a specific programming language.

## Available Handlers

WebUI provides official handlers for several popular programming languages (other languages coming soon):

- [**Rust**](./rust) - High-performance native rendering with the Rust programming language
- [**Node**](./node) - Streaming SSR via a native addon built with napi-rs for Node, Bun, and Deno.
- [**Electron**](./electron) - Desktop apps via Electron with custom `webui://` protocol
- [**WebAssembly**](./wasm) - In-browser rendering for playgrounds and client-side use
- [**FFI (C API)**](./ffi) - Shared library for Go, C#, Python, and any language with C interop

## How Handlers Work

All WebUI handlers follow the same pattern:

1. They accept a WebUI protocol object (parsed from protobuf binary)
2. They process the protocol with the provided state data
3. They render the final HTML output by evaluating directives and inserting dynamic content

This consistent approach ensures that the same template produces identical results across different programming languages and platforms.

## Common Handler Interface

While the specific implementation details vary between languages, all handlers provide a similar API:

```
handle(protocol, state, writer)
```

Where:
- `protocol` is the WebUI protocol object
- `state` is the data object with values to be rendered
- `writer` is a callback or interface for writing the rendered output

## Plugin System

Handlers support an optional **plugin system** for injecting framework-specific behavior during rendering. Plugins receive lifecycle callbacks at key points — binding start/end, loop iteration, scope changes — and can write additional content to the output.

```
handler = Handler::with_plugin(plugin)
handler.handle(protocol, state, writer)
```

When no plugin is configured, the handler renders plain HTML. When a plugin is loaded (e.g., `FastHydrationPlugin`), it injects markers that enable client-side hydration.

See [Plugins](/guide/concepts/plugins/) for the full plugin API and built-in plugins.

## Handler Customization

Each handler implementation can be extended or customized to fit specific requirements. See the individual handler documentation for language-specific details.
