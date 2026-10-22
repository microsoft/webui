---
description: Follow a WebUI template from compilation through native server rendering to optional browser hydration.
---

# How It Works

WebUI separates the files you author from the work each request and browser
must do. The important boundary is simple: **the server renders templates with
data, not by running your component TypeScript.**

<rendering-diagram></rendering-diagram>

The diagram describes responsibilities, not measured timings. In a normal SSR
deployment, `protocol.bin` stays on the server. The browser receives HTML and
the assets needed by the page.

## Build Phase

A small interactive app has these inputs:

| File | Responsibility |
| --- | --- |
| `src/index.html` | Page document and component placement |
| `src/my-counter/my-counter.html` | Component structure and bindings |
| `src/my-counter/my-counter.css` | Paired component styles |
| `src/my-counter/my-counter.ts` | Optional browser behavior |
| `src/index.ts` | Imports the component definitions used by the page |

`webui build` discovers templates, validates their syntax, and compiles them
into `protocol.bin`. It also prepares the chosen CSS output. Rebuild when those
sources change, not for each request.

```bash
npx webui build ./src --out ./dist --plugin webui
```

This command **does not bundle application TypeScript**. Use a JavaScript
bundler for the browser entry and load its output from the page. The
[first-app tutorial](/tutorials/hello-world#adding-interactivity) provides a
complete esbuild setup.

## Server Render Phase

Your production host loads the compiled protocol once at startup and reuses it
for requests. For each page it supplies JSON state and a request path. The
renderer combines static template content with bindings, conditions, and loops
to produce HTML.

The development command `webui serve` handles compilation and preview rendering
for you. In production, your [host integration](/guide/integrations) owns
data access, request handling, asset delivery, and error responses.

### What the handler does NOT do

- Reparse your HTML templates on every request.
- Run component constructors, hooks, or TypeScript to discover initial values.
- Bundle browser code or replace your backend's authorization and data layer.

Native rendering is still computation: state preparation, condition evaluation,
loops, output allocation, and transport all contribute to the request cost.

### Light DOM and Declarative Shadow DOM

Unwrapped component templates default to open Shadow DOM. A sole
`<template shadowrootmode="open">` makes that choice explicit. A sole bare
`<template>` selects Light DOM, and `--dom light` changes the fallback for
unwrapped components.

| Mode | Authoring consequence |
| --- | --- |
| Light | Content renders inside the custom-element host. CSS is global in its containing tree; use ordinary selectors such as `my-card .title`. |
| Shadow | Content renders in an open declarative Shadow root. CSS has a native boundary; `:host` and `<slot>` are available. |

Both can display server-rendered content without component JavaScript in a
browser with the required platform support. The choice affects styling and
composition, not whether the server can render data.

## Client Hydration Phase

Importing a component definition registers a custom element. With
`@microsoft/webui-framework`, that element connects its behavior to the HTML
already rendered by the server.

### How hydration works

From an author's perspective:

1. The page loads the bundled entry module.
2. The entry imports the interactive component definitions.
3. The runtime adopts their existing DOM and connects bindings and events.
4. Changes to observable values update the bound content.

Use `hydratedCallback()` for behavior that needs the component's DOM or refs.
Do not write initial display state in a constructor and expect the server HTML
to change.

### Islands Architecture

Components with no authored behavior need no matching module or empty class.
A scriptless page can omit the browser runtime altogether. In an interactive
application, scriptless hosts may activate later for state updates or
navigation.

Hydration policies can defer selected components, but nested components must
respect their parent's hydration boundary. See [Hydration](/guide/concepts/hydration)
for lifecycle rules and [lazy policies](/guide/concepts/directives/lazy) for
the tradeoff between startup cost and first-use latency.

## The SSR Mental Model

### The server is the source of truth for the initial render

Every value needed by the initial template must be available in server state
or supplied through the component's template attributes. A TypeScript field
initializer is not a server-side data source.

```json
{ "title": "My counter", "count": 0 }
```

Missing text paths render empty. A whole-binding attribute is omitted for a
missing or `null` value; `""`, `0`, and `false` remain attribute values.
A missing condition identifier is falsy.

### Derived state belongs in the server or the template

Text bindings look up paths; they do not execute arbitrary JavaScript.
Conditions support a limited expression language:

```html
<h1>{{title}}</h1>
<p>{{items.length}} items</p>
<if condition="items.length > 0">
  <p>Your list is ready.</p>
</if>
```

Compute formatting, arithmetic, and other complex initial values in your
backend. The [directive reference](/guide/concepts/directives/) lists the
supported syntax.

### The client handles interactivity after hydration

Event handlers can update observables, call APIs, or move focus. Those client
changes do not automatically persist to the server. Your application decides
how to save changes and handle failed requests.

Treat hydration state as public. Optional build-time state projection reduces
the data serialized for the browser; it is not a secrecy boundary. Never put
credentials or private tokens in render state.

## What Makes This Fast

Compilation removes template parsing from the request path. Reusing a loaded
protocol avoids repeated decoding. Scriptless components avoid authored
behavior, and hydration adopts existing DOM instead of rebuilding the initial
page.

These are mechanisms, not universal timing guarantees. Follow the
[performance guide](/guide/concepts/performance) to measure server and browser
work separately, or inspect the qualified [benchmark evidence](/benchmarks/).
