---
title: "Language integrations"
description: "Choose a native binding or example host, load a compiled protocol once, and render HTML with request-specific JSON state."
---

# Language Integrations

Compile templates with `webui build`, then render the resulting `protocol.bin`
through the binding for your backend. The renderer is Rust; it does not require
a JavaScript SSR engine. Your host supplies state, HTTP responses, and asset
delivery.

Choose an integration with a compatible runtime and native target:

| Host | Distribution and scope |
| --- | --- |
| [Rust](./rust) | `microsoft-webui` on crates.io, imported as `webui`; build and render APIs |
| [Node.js and Bun](./node) | `@microsoft/webui` on npm; requires its platform-specific native addon |
| [.NET](./dotnet) | `Microsoft.WebUI` NuGet package with matching native runtime packages |
| [Python](./python) | Native `microsoft-webui` wheels and source distribution on GitHub Releases; not yet on PyPI |
| [C / FFI](./ffi) | Build the shared library and use its C ABI; includes Go and Python `ctypes` examples |
| [WebAssembly](./wasm) | Generated handler-only, parser-only, or combined bundles for browser-hosted use |
| [Electron](./electron) | Repository example: render a prebuilt app once and serve it through a custom protocol |
| [Tauri](./tauri) | Repository example: a Rust snapshot viewer, not a full application backend |

For the native WebUI desktop SDK rather than an Electron or Tauri example, see
[Desktop Apps](/build/desktop).

## How Handlers Work

1. Load `protocol.bin` once at host startup.
2. Reuse the loaded `Protocol` (or Python `Renderer`) with fresh JSON state and
   render options for each request.
3. Send the returned HTML, or drive a streaming session while honoring transport
   backpressure.
4. Release native handles according to the binding's ownership rules.

Do not decode the protocol per request. Keep the compiler, renderer, and
optional browser framework versions compatible. For normal native SSR, the
browser receives HTML and assets, not `protocol.bin`.

## Common Handler Interface

The conceptual rendering operation is:

```
render(protocol, state, options, writer)
```

Where:
- `protocol` is the WebUI protocol object
- `state` is the data object with values to be rendered
- `options` specifies the entry fragment and request path for [route matching](/build/routing)
- `writer` is a callback or interface for writing the rendered output

## Plugin System

Select the same hydration plugin when building and rendering an interactive
app. Custom Rust integrations can implement the public plugin traits.

```
handler = Handler::with_plugin(plugin)
handler.render(protocol, state, options, writer)
```

When no plugin is configured, the handler renders plain HTML. When a plugin is loaded (e.g., `FastV3HydrationPlugin`), it injects markers that enable client-side hydration.

See [Plugins](/reference/plugins/) for the full plugin API and built-in plugins.

## Handler Customization

Choose buffered output for a complete response, partial responses for the
[router](/build/routing#server-contract), or host-driven streaming for
[boundaries](/learn/directives/boundary). The binding guides document
their concrete options, threading, errors, and memory ownership.
