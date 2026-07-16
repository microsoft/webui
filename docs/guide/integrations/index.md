# Language Integrations

WebUI runs anywhere you can render text. The protocol is compiled once with `webui build` and rendered by a language-specific **handler**, the bridge between the WebUI protocol and the final HTML output.

Pick the handler that matches your stack:

- [**Rust**](./rust), High-performance native rendering with the Rust programming language
- [**Node**](./node), Buffered and streaming SSR via a native addon built with napi-rs for Node, Bun, and Deno
- [**.NET**](/guide/installation#net), Managed `Microsoft.WebUI` NuGet bindings with transitive native runtime packages
- [**Electron**](./electron), Desktop apps via Electron with custom `webui://` protocol
- [**WebAssembly**](./wasm), Split parser, handler, and combined browser bundles
- [**C / FFI**](./ffi), Shared library for Go, C#, Python, and any language with C interop

## How Handlers Work

All WebUI handlers follow the same pattern:

1. They accept a loaded WebUI `Protocol`, decoded and indexed once at startup
2. They process the protocol with the provided state data
3. They render the final HTML output by evaluating directives and inserting dynamic content

This consistent approach ensures that the same template produces identical results across different programming languages and platforms.

Share the loaded `Protocol` across repeated requests. It avoids protobuf
decoding and deterministic index construction on every render.

## Common Handler Interface

While the specific implementation details vary between languages, all handlers provide a similar API:

```
render(protocol, state, options, writer)
```

Where:
- `protocol` is the WebUI protocol object
- `state` is the data object with values to be rendered
- `options` specifies the entry fragment and request path for [route matching](/guide/concepts/routing)
- `writer` is a callback or interface for writing the rendered output

## Plugin System

Handlers support an optional **plugin system** for injecting framework-specific behavior during rendering. Plugins receive lifecycle callbacks at key points, binding start/end, loop iteration, scope changes, and can write additional content to the output.

```
handler = Handler::with_plugin(plugin)
handler.render(protocol, state, options, writer)
```

When no plugin is configured, the handler renders plain HTML. When a plugin is loaded (e.g., `FastV3HydrationPlugin`), it injects markers that enable client-side hydration.

See [Plugins](/guide/concepts/plugins/) for the full plugin API and built-in plugins.

## Handler Customization

Each handler implementation can be extended or customized to fit specific requirements. See the individual handler documentation for language-specific details.
