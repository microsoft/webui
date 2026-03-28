# Why We Rebuilt Web Rendering From Scratch

*Published on the [Microsoft Edge Dev Blog](https://blogs.windows.com/msedgedev/)*

---

## The Problem We Couldn't Ignore

Last year, we published ["An Even Faster Microsoft Edge"](https://blogs.windows.com/msedgedev/2024/05/28/an-even-faster-microsoft-edge/) — a post about the performance problems we'd found in Edge's own UI. The response was enormous, but the post was intentionally high-level. This is the deeper story.

Here's what we were dealing with: Edge's internal pages — Settings, History, Favorites, and dozens of other features — all ran on a client-side rendering stack. When you opened one of these pages, the browser had to download a JavaScript bundle, parse it, JIT-compile it, execute it, build a DOM, and *then* paint pixels to the screen. Every time. On a modern laptop, you barely notice. On a budget Windows laptop with 4 GB of RAM and a spinning disk, or a five-year-old corporate desktop, you feel every one of those steps.

We profiled it. The bundles were too large. The framework we relied on needed JavaScript to render even the most basic static content. A settings page that's 95% static text was still blocked on a full JS execution pipeline before the user saw anything.

The root cause wasn't our code — it was the architecture. Client-side rendering pays a tax on every page load: download → parse → compile → execute → render. That tax is fixed overhead, and it hits hardest on the devices that can least afford it.

So we started asking a different question. We knew that raw HTML and CSS, delivered as a server response, renders faster than any framework running on top of the platform. Browsers are *extremely* good at parsing HTML and applying CSS — decades of optimization have gone into that pipeline. The question was: **could we get framework-level developer productivity with platform-level performance?**

That question led to [WebUI](https://github.com/microsoft/webui) — an open-source, server-side rendering framework built in Rust that compiles HTML templates to a binary protocol at build time. At runtime, any backend language fills in state data and gets fully rendered HTML back. No template parsing. No JavaScript runtime. No framework overhead standing between your HTML and the user's screen.

This is the story of how we got here.

## The Insight: Compile Once, Render Everywhere

Think about how traditional SSR works. Whether you're using EJS, Handlebars, Pug, or even React Server Components and JSX, every request involves parsing a template. The server reads template syntax, walks a tree, evaluates expressions, and concatenates strings to produce HTML. Template engines are essentially interpreters — they re-read and re-evaluate the same structural instructions on every single request.

But here's the thing: **the structure of an HTML template doesn't change between requests.** The `<div>` tags, the CSS classes, the component hierarchy, the conditional branches — all of that is known at build time. The only thing that changes is the data: the user's name, the list of items, whether a feature flag is on or off.

This is the same insight that separates compiled languages from interpreted ones. A C compiler doesn't re-parse your source code every time the program runs. It compiles once to machine code, and that binary runs directly. WebUI applies the same principle to HTML templates.

At build time, WebUI parses your templates, resolves components, extracts CSS tokens, and encodes everything into a Protocol Buffer binary. This binary is a compact, pre-processed representation of your template's structure — the static HTML skeleton with typed slots for dynamic data.

At runtime, you feed that binary a JSON state object. WebUI walks the pre-compiled structure, fills in your data, evaluates conditions, expands loops, and emits rendered HTML. No parsing. No string concatenation. No tree walking over template syntax.

```
┌──────────────┐    ┌───────────────┐    ┌───────────────┐
│  HTML + CSS  │───▶│  webui build  │───▶│ .webui binary │
│  templates   │    │  (build time) │    │  (protocol)   │
└──────────────┘    └───────────────┘    └───────┬───────┘
                                                 │
                    ┌───────────────┐             │
                    │  JSON state   │─────────────┤
                    │  (runtime)    │             │
                    └───────────────┘             ▼
                                         ┌───────────────┐
                                         │    handler    │
                                         │  (any lang)   │
                                         └───────┬───────┘
                                                 │
                                                 ▼
                                         ┌───────────────┐
                                         │ rendered HTML │
                                         └───────────────┘
```

The result is a rendering path with almost no wasted work. Every CPU cycle at request time goes toward producing output — not toward figuring out what output to produce.

## No JavaScript Runtime Required

This is the part that surprises people the most: **WebUI doesn't need Node.js to render HTML on the server.**

When you run React SSR, Next.js, or Nuxt in production, your server is running a full V8 JavaScript engine. That's a garbage-collected runtime with JIT compilation, an event loop, and all the overhead that comes with running a general-purpose programming language just to concatenate strings into HTML. For many applications that works fine. But when you're rendering thousands of pages per second, or running on constrained infrastructure, that overhead adds up.

WebUI's core renderer is written in Rust. It operates directly on the pre-compiled Protocol Buffer binary — no interpreter, no VM, no garbage collector in the hot path. But we didn't build this to be a Rust-only tool. The renderer exposes a C FFI, which means any language that can call C functions can use it:

- **Rust** — native, zero-overhead
- **Node.js / Bun / Deno** — via native addon (N-API)
- **C# / .NET** — via P/Invoke
- **Python** — via ctypes
- **Go** — via cgo
- Or any language with C interop

This isn't an anti-JavaScript stance. JavaScript is the right tool for plenty of server-side work — business logic, API orchestration, database queries. The argument is simpler: **template rendering is mechanical work, and mechanical work belongs in a compiled, optimized code path.**

WebUI splits the work into three layers, each handled by the right tool:

- **Build time:** Template parsing, CSS token extraction, component discovery, validation — all done once by the Rust compiler.
- **Server runtime:** State interpolation, condition evaluation, loop expansion, route matching — done by the native renderer, called from whatever backend language you prefer.
- **Client:** Hydration for interactive islands — only where the page actually needs JavaScript. Most of the page ships as static HTML.

Here's what it looks like from Node.js:

```js
import { build, render } from "@microsoft/webui";

const result = build({ appDir: "./src" });
const html = render(result.protocol, { name: "World", items: ["a", "b"] });
```

Two function calls. The `build` step runs once (or at startup). The `render` call is what happens per-request — and it's calling into native Rust under the hood.

## Betting on the Web Platform

There's a deeper reason we built on standard HTML, CSS, and Web Components rather than inventing a proprietary rendering layer: **portability.**

WebUI templates are standard HTML. The output is standard HTML. Components use Declarative Shadow DOM — a web platform primitive, not a framework abstraction. This means a WebUI application runs anywhere the web platform runs: in a browser tab, as an installed PWA, inside a WebView2 desktop shell, or embedded in an Electron or Tauri app.

This matters because the conventional wisdom has been that you choose between web (portable, slower) and native (fast, platform-locked). WebUI challenges that tradeoff. When your rendering layer is 4–8× faster than mainstream SSR frameworks, the performance gap between "web app" and "native app" narrows dramatically — especially for content-heavy UI where most of the screen is text, lists, and layout. You get web-platform portability without the web-platform performance tax.

Inside Edge, this is exactly how we use it. Edge's internal pages are rendered by the browser's built-in C++ backend using the WebUI protocol. They feel native because the rendering is fast enough that users can't tell the difference — and they're portable because they're just HTML.

## The Numbers Speak

Claims about performance mean nothing without benchmarks. We ran WebUI against two established baselines — Fastify with fastify-html (one of the fastest Node.js SSR setups available) and React SSR — using the same workload and methodology.

**Methodology:** [autocannon](https://github.com/mcollina/autocannon), 100 concurrent connections, 10-second duration, 2-second warmup. The workload renders approximately 2,400 tiles in a spiral pattern — the same benchmark used by the fastify-html project to validate their own performance claims. Same hardware, same network conditions, same content.

### SSR Performance Showdown

| Framework | Requests/sec | Avg Latency | Throughput |
|---|---:|---:|---:|
| **WebUI (Rust)** | **4,511** | **21.7 ms** | **684 MB/s** |
| Fastify + HTML (Node.js) | 1,061 | 93.4 ms | 209 MB/s |
| React SSR (Node.js) | 552 | 179.2 ms | 78.5 MB/s |

WebUI is **4.3× faster than Fastify** and **8.2× faster than React SSR**.

But averages hide tail latencies, and tail latencies are what your users actually feel. Here's the percentile breakdown:

| Framework | p50 Latency | p99 Latency |
|---|---:|---:|
| **WebUI** | **18 ms** | **52 ms** |
| Fastify + HTML | 92 ms | 118 ms |
| React SSR | 180 ms | 210 ms |

WebUI's p99 latency (52 ms) is lower than Fastify's *median* latency (92 ms). That means WebUI's worst-case response is faster than Fastify's typical response.

We also ran a more realistic benchmark — a contact book application with nested components, conditional rendering, and loops:

### Contact Book Benchmark

| Contacts | Render Time | Output Size |
|---:|---:|---:|
| 10 | 0.65 ms | 25 KB |
| 100 | 4.94 ms | 56 KB |
| 1,000 | 57.5 ms | 363 KB |

These are single-render times, not throughput numbers. Sub-millisecond rendering for a small page, under 5 ms for a hundred items, and still under 60 ms for a thousand-item list with full component expansion.

One concern we hear often: "What about hydration? Doesn't the hydration plugin add overhead?" It does — but less than you might expect. With the hydration plugin enabled, 1,000 contacts renders in 59.5 ms versus 57.5 ms without it. That's roughly 2–3% overhead for full client-side interactivity support.

What does this mean in practice? Fewer servers to handle the same traffic. Lower cloud bills. Better tail latencies for end users on slow connections. And headroom — when your rendering layer is 4–8× faster, you can spend that budget on business logic, database queries, and features instead of on string concatenation.

## How It Works (High-Level)

WebUI templates are standard HTML. There's no JSX, no proprietary syntax language, no build-step-specific file format. You write HTML, CSS, and a small set of directives that the compiler understands.

Here's a basic template:

```html
<h1>{{title}}</h1>
<if condition="showGreeting">
  <p>Hello, {{name}}!</p>
</if>
<ul>
  <for each="item in items">
    <li>{{item.label}}: {{item.value}}</li>
  </for>
</ul>
```

`{{signals}}` interpolate state values. `<if>` evaluates a condition from your state object. `<for>` iterates over arrays. That's most of what you need for server-rendered content.

Components use **Declarative Shadow DOM** — a web platform standard, not a proprietary component model. You define a component with a `<template shadowrootmode="open">` and WebUI handles the rest. No custom element registry, no client-side class definitions required for server rendering. The browser natively understands Declarative Shadow DOM, so components render without any JavaScript on the client.

Building and serving is straightforward:

```bash
# Compile templates to protocol binary
npx webui build ./my-app --out ./dist

# Dev server with live reload
npx webui serve ./my-app --state ./data/state.json --watch
```

The `build` command walks your app directory, discovers components, parses templates, and emits a `.webui` binary. The `serve` command starts a development server that watches for file changes, rebuilds automatically, and serves rendered HTML using a local state file.

**Routing** is handled through nested `<route>` elements with server-side matching. Routes are declared in your templates, resolved at build time, and matched at runtime. Client-side navigation can take over after the initial server render for SPA-like transitions where you want them.

**Interactivity** follows an islands architecture. Most of the page is static HTML — fast to render, fast to parse, zero JavaScript cost. For the parts that need client-side behavior — a search input, a dropdown, a live-updating counter — WebUI's plugin system enables selective hydration. Only interactive components ship JavaScript to the client. You can use FAST-HTML or the WebUI Framework for client-side reactivity in those islands.

The key design principle: **the default is zero JavaScript.** You opt in to interactivity per-component, not per-page. The result is pages that load fast by default and stay fast as they grow.

## Open Source and What's Next

WebUI is open source under the MIT license. The code is at [github.com/microsoft/webui](https://github.com/microsoft/webui), and the documentation is at [microsoft.github.io/webui](https://microsoft.github.io/webui).

Getting started takes one command:

```bash
npm install @microsoft/webui
```

If you want to explore before installing anything, try the [interactive playground](https://microsoft.github.io/webui/playground/) — edit templates, supply state, and see rendered output in your browser.

Inside Edge, we're already using this architecture across an expanding set of internal pages — each migration has validated the same pattern: faster load times, lower memory usage, and simpler infrastructure. We're continuing to roll this out across more Edge features.

This post covered the *why* — the problem, the insight, and the results. In the next post, ["Inside WebUI: How a Compiled Protocol Replaces Your JavaScript Runtime"](./blog-inside-webui-technical-deep-dive.md), we'll go deeper into the *how*: the Protocol Buffer encoding format, the rendering algorithm, the component resolution pipeline, and the decisions we made (and reconsidered) along the way.

We'd love your feedback. [File an issue](https://github.com/microsoft/webui/issues), open a PR, or just [star the repo](https://github.com/microsoft/webui) if this approach resonates with you. We built WebUI to solve a real problem inside Edge, and we open-sourced it because we think it can solve the same problem for a lot of other teams.

---

*This is Part 1 of a four-part series on WebUI. [Part 2](./blog-inside-webui-technical-deep-dive.md) covers the engine internals. [Part 3](./blog-building-interactive-apps.md) shows how to build interactive apps. [Part 4](./blog-from-react-to-btr.md) tells the story of Edge's migration from React to BTR.*
