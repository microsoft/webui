# Why WebUI?

## Origin

WebUI started inside Microsoft Edge.

Edge is built on Chromium, a C++ codebase. When the Edge team needed to render dynamic UI for in-browser surfaces, the first answer was a client-rendered application: React, then Web Components. The migration to Web Components improved First Contentful Paint and Largest Contentful Paint, but the experience still felt slow. The reason was structural. When HTML and CSS are bundled inside a JavaScript application, the browser cannot paint until that JavaScript downloads, parses, and executes.

The fix was server-side rendering. The conventional answer for SSR was "spin up a Node.js process and run Next.js, Nuxt, or Remix." That answer never made it past the architecture review.

A C++ application embedding a JavaScript engine, to render JavaScript components, that ultimately produce HTML, is three layers of indirection too many. It is slow to start, heavy on memory, and operationally expensive. Edge needed something native: code that could run *inside* Chromium's process model and produce rendered HTML without spawning a JavaScript runtime at all.

So the team asked a different question. **What if server-side rendering did not need JavaScript on the server?**

The answer was already in the platform. Web Components are native to the browser, and they are fast because they are native. Declarative Shadow DOM makes encapsulated, server-rendered components possible without any JavaScript on the page. The piece that did not exist was a renderer: a tiny, native, language-agnostic component that could fill HTML templates with per-request data without parsing, without an AST, and without a JavaScript engine.

That renderer is WebUI.

It runs inside Chromium. It also runs in Rust services, Go binaries, Python web apps, C# APIs, and on every JavaScript runtime worth caring about: Node.js, Bun, and Deno. Because the protocol it streams is just bytes, the host language does not matter. The framework that had to exist for Microsoft Edge turned out to be the framework everyone else has been waiting for.

## The Problem

What Edge ran into is the wall every SSR-heavy team eventually hits. Conventional server-side rendering carries two costs that compound at scale.

**On the server**, traditional SSR frameworks re-parse templates on every request - tokenizing, building an AST, evaluating expressions, and serializing output. This work is redundant: the template structure hasn't changed since the last request, only the data has. JavaScript-based SSR (Next.js, Nuxt, Remix) adds a second layer of overhead by requiring a Node.js runtime - with garbage collection pauses, JIT warmup costs, and memory pressure that grow worse under load.

**On the client**, modern frameworks ship entire component trees as JavaScript bundles. The pipeline is sequential and blocking: download the JS bundle → parse it → compile it → execute it → fetch data → render. Nothing paints until the bundle finishes. On constrained devices - 4 GB of RAM, older mobile CPUs - this pipeline takes 1–2 seconds. Every component in the tree ships JavaScript, even components that never handle user interaction and never update.

Both approaches do redundant work on every request, ship unnecessary code to the browser, and scale poorly under load.

## The Insight

HTML template structure is **static**. It does not change between requests - only the data changes. This is the same insight that separates compiled languages from interpreted ones: move the expensive work (parsing, analysis, optimization) to build time, and keep runtime costs minimal.

WebUI applies this principle to web rendering. Templates are compiled **once** into a compact Protocol Buffer binary. At runtime, a handler reads the protocol sequentially - emitting static fragments as-is and resolving dynamic fragments from a state object. There is no parsing, no AST walking, no expression compilation at request time.

```
HTML + CSS templates → webui build → protocol.bin → handler (any lang) + state → rendered HTML
```

## The Web Platform Bet

Historically, frameworks existed because the web platform lacked key primitives. That is no longer the case. Modern browsers ship with:

- **Web Components** - reusable custom elements with a standard lifecycle, no framework runtime required
- **Declarative Shadow DOM** - server-renderable encapsulation without JavaScript
- **CSS containment** - layout and paint isolation for predictable rendering performance
- **Adopted stylesheets** - shared, constructable stylesheets across shadow roots
- **Navigation API** - client-side routing without framework abstractions

WebUI builds directly on these platform primitives rather than wrapping them in an abstraction layer. Templates use standard HTML and native Web Components. The optional client-side router uses the Navigation API. Styling uses adopted stylesheets and CSS containment. No proprietary component model, no virtual DOM, no framework runtime in the browser.

When you build on the web platform, you inherit its improvements for free. Every browser performance optimization, every new CSS feature, every platform API lands in your app without a framework upgrade.

## Islands Architecture

WebUI uses an **Islands Architecture** where each Web Component is an independent island of interactivity.

The idea is simple: most of a web page is static content - headings, paragraphs, images, layout. Only specific parts of the page need to respond to user interaction - buttons, forms, search boxes, interactive widgets. Why ship JavaScript for all of it?

With WebUI's Islands Architecture:

- **Static content** is server-rendered HTML. It arrives fully formed in the initial response. No JavaScript is shipped, no hydration occurs, no client-side processing is needed. It is just HTML and CSS - the fastest thing a browser can render.

- **Interactive components** are Web Components that hydrate on the client. Each island is self-contained with its own Shadow DOM, encapsulated styles, and TypeScript behavior. Islands hydrate independently - they don't wait for each other or for a global framework to initialize.

- **You control the boundary.** The hydration plugin system lets you decide exactly which components are interactive islands and how they hydrate (on load, on visible, on interaction).

### The practical difference

Consider a product page with 10 components: a header, navigation, breadcrumbs, product title, product image, description, price display, "Add to Cart" button, reviews list, and a review form.

| Approach | JavaScript shipped |
|----------|-------------------|
| Traditional SPA | All 10 components ship JS, hydrate, and re-render on the client |
| WebUI Islands | Only 2 components ship JS - the "Add to Cart" button and the review form. The other 8 are static HTML with zero client-side cost |

The result: dramatically less JavaScript, faster time-to-interactive, and better performance on constrained devices.

## Performance

Build-time compilation eliminates per-request template overhead. Islands Architecture eliminates unnecessary client-side JavaScript. Together, they produce measurable gains:

| Benchmark | Result |
|-----------|--------|
| vs. Fastify (raw SSR) | **4.3× faster** |
| vs. React SSR | **8.2× faster** |
| Small pages | **Sub-millisecond** rendering |
| Large lists (1,000+ items) | **Linear scaling** |

These numbers follow directly from the architecture:

- **Static fragments** are pre-serialized bytes copied directly to the output buffer. No string concatenation, no template interpretation.
- **Dynamic fragments** resolve to simple key lookups against a flat state object. No expression compilation at runtime.
- **No runtime allocations** for template structure - the protocol binary is read sequentially, and output is written to a pre-allocated buffer.
- **No client-side framework runtime** - islands hydrate independently using native Web Component lifecycle hooks, not a framework reconciliation pass.

## No Node.js Required

This is not a minor implementation detail - it is a fundamental architectural advantage.

Traditional SSR frameworks require a Node.js process on every server. That means:

- **V8 engine overhead** - garbage collection pauses cause latency spikes under load
- **JIT warmup** - the first requests are slow while V8 compiles hot paths
- **High memory baseline** - a Node.js process starts at ~30–50 MB before your application loads
- **Single-threaded event loop** - CPU-bound rendering blocks all other requests

WebUI's Rust-native handler eliminates all of this:

- **No garbage collector** - deterministic memory management, no pauses
- **No JIT warmup** - compiled ahead of time, first request is as fast as the millionth
- **Minimal memory footprint** - the handler is a small, statically-linked binary
- **Multi-threaded** - handles concurrent requests across all CPU cores without contention

The practical result: fewer servers, lower memory consumption, more predictable latency, and lower cloud bills. A single WebUI server can handle the load that previously required multiple Node.js instances behind a load balancer.

## Summary

WebUI exists because modern web rendering does too much redundant work — on the server and in the browser.

| Problem | WebUI's Answer |
|---------|----------------|
| Full JS bundles shipped to browser | Islands Architecture — only interactive components ship JS |
| Framework abstractions over the platform | Direct use of Web Components, Shadow DOM, Navigation API |
| Node.js runtime required on server | Rust-native rendering, no JavaScript runtime, no GC pauses |

The result is a framework that is extremely fast, renders pages in **sub-millisecond time**, ships **minimal JavaScript to the browser**, and works from **any backend language** — without a JavaScript runtime on the server.
