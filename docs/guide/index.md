# Introduction

## What is WebUI?

WebUI is a server-side rendering framework that compiles HTML templates into a binary protocol at build time. At runtime, your backend simply applies state data to the pre-compiled protocol and gets rendered HTML back — no template parsing, no JavaScript runtime, minimal computation.

## The Problem

Traditional server-side rendering re-parses and re-evaluates templates on every request. JavaScript-based SSR frameworks require a Node.js runtime on the server. Both approaches add latency and complexity that scales poorly.

## How WebUI Works

WebUI splits the work into two phases:

1. **Build time** — The CLI parses your HTML templates, discovers web components, evaluates static content, and compiles everything into a compact Protocol Buffer binary. Static and dynamic content are separated. This happens once.

2. **Runtime** — Your backend handler loads the pre-compiled protocol, receives JSON state data, and produces rendered HTML. No parsing, no AST walking, no expression compilation — just fill in the blanks.

The result: rendering is fast, predictable, and language-agnostic.

## Key Concepts

- **Protocol Buffer binary** — Templates compile to a compact binary format. The handler reads fragments sequentially — static fragments are emitted as-is, dynamic fragments are resolved from state.

- **Language agnostic** — Native handlers for Rust, Node/Bun/Deno, C#, Python, and Go. Any other language can use the C FFI bindings.

- **Web Components** — Templates are standard HTML with native web components and Shadow DOM. No proprietary syntax beyond a few template directives (`<if>`, `<for>`, <code v-pre>{{}}</code>).

- **Routing** — The `<route>` directive defines URL-to-component mappings. The server renders the matched route on first load; the optional [`@microsoft/webui-router`](/guide/concepts/routing) package adds client-side navigation with lazy loading.

- **Server-side expressions** — Conditionals and expressions are evaluated on the server. Template logic doesn't leak into the browser.

- **Plugin system** — Parser and handler plugins for hydration, adding reactivity to interactive islands, custom directives, and framework-specific behavior.

## Quick Overview

```
┌──────────────┐    ┌───────────────┐    ┌───────────────┐
│  HTML + CSS  │───▶│  webui build  │───▶│ .webui binary │
│  templates   │    │  (build time) │    │  (protocol)   │
└──────────────┘    └───────────────┘    └───────┬───────┘
                                                 │
                    ┌───────────────┐            │
                    │  JSON state   │────────────┤
                    │  (runtime)    │            │
                    └───────────────┘            ▼
                                         ┌───────────────┐
                                         │    handler    │
                                         │   (any lang   │
                                         └───────┬───────┘
                                                 │
                                                 ▼
                                         ┌───────────────┐
                                         │ rendered HTML │
                                         └───────────────┘
```

Ready to try it? Start with the [Playground](/playground/) to experiment in the browser, then follow the [installation guide](./installation) or the [Hello World tutorial](/tutorials/hello-world) to build locally.