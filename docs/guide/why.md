# Why WebUI?

## The Problem

Server-side rendering (SSR) re-parses templates on every request — tokenizing, building an AST, evaluating expressions, and serializing output. This work is redundant: the template structure hasn't changed since the last request, only the data has.

Client-side rendering (React, Angular, Vue) shifts the cost to the browser. The pipeline is sequential: download the JS bundle, parse it, compile it, execute it, fetch data, then render. Nothing paints until the bundle finishes compiling. On constrained devices — 4 GB of RAM, older mobile CPUs — this pipeline takes 1–2 seconds. The JavaScript bundle is a serializing bottleneck that blocks every pixel on screen.

Both approaches do redundant work on every request and scale poorly under load.

## The Insight

HTML template structure is static. It does not change between requests — only the data changes. This is the same insight that separates compiled languages from interpreted ones: move the expensive work (parsing, analysis, optimization) to build time, and keep runtime costs minimal.

WebUI applies this principle to HTML rendering. Templates are compiled once into a compact binary protocol. At runtime, a handler reads the protocol sequentially — emitting static fragments as-is and resolving dynamic fragments from a JSON state object. There is no parsing, no AST walking, no expression evaluation at request time.

```
HTML + CSS templates → webui build → .webui binary → handler (any lang) + JSON state → rendered HTML
```

## The Web Platform Bet

Historically, frameworks existed because the web platform lacked key primitives. That is no longer the case. Modern browsers ship with:

- **Declarative Shadow DOM** — server-renderable encapsulation without JavaScript
- **Web Components** — reusable custom elements with a standard lifecycle
- **CSS containment** — layout and paint isolation for predictable performance
- **Adopted stylesheets** — shared, constructable stylesheets across shadow roots
- **Navigation API** — client-side routing without framework abstractions

WebUI builds directly on these platform primitives rather than wrapping them in an abstraction layer. Templates use standard HTML and native web components. The optional client-side router uses the Navigation API. Styling uses adopted stylesheets and CSS containment. No proprietary component model, no virtual DOM, no framework runtime in the browser.

## Language Agnostic

The compiled protocol is a binary format — just bytes. Any language that can read bytes and write strings can render WebUI templates. Native handler implementations exist for Rust, Node.js (including Bun and Deno), C#, Python, and Go. Any other language can use the C FFI bindings.

No JavaScript runtime is required on the server. A Go service, a Rust microservice, and a C# API can all render the same compiled templates using the same protocol.

## Performance

Build-time compilation eliminates per-request template overhead, producing measurable gains:

| Benchmark | Result |
|-----------|--------|
| vs. Fastify (raw SSR) | 4.3× faster |
| vs. React SSR | 8.2× faster |
| Small pages | Sub-millisecond rendering |
| Large lists (1,000+ items) | Linear scaling |

These numbers follow directly from the architecture: static fragments are pre-serialized bytes copied to the output buffer, and dynamic fragments resolve to simple key lookups against a flat state object. There are no runtime allocations for template structure, no string concatenation for static content, and no interpretation overhead.

## Summary

WebUI exists because template parsing is redundant work. By compiling templates at build time, building on web platform primitives instead of framework abstractions, and keeping the runtime handler minimal and language-agnostic, WebUI eliminates the two dominant costs of modern web rendering: server-side template interpretation and client-side JavaScript compilation.
