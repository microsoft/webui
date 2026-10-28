---
description: When WebUI's native renderer and HTML-first component model fit an application, and which tradeoffs to consider.
---

# Why WebUI?

Choose WebUI when you want server-rendered HTML from a native backend and
explicit control over the browser code your application loads. It combines a
compiled template format, a Rust renderer, and optional Web Component behavior.

## Origin

WebUI started inside Microsoft Edge, where UI needed to fit a native Chromium
host rather than require a separate JavaScript rendering service. That
constraint shaped the framework: compile templates ahead of time, supply data
from the host, and render HTML without executing application JavaScript.

The same renderer can be used outside a browser host. See the
[integration guides](/build/integrations) for supported bindings and their
installation and distribution requirements.

## The Problem

Rendering choices move work between the build, server, and browser. A
client-rendered page may wait for JavaScript before showing its main content.
SSR can show that content earlier, but server execution, response size, and
hydration still need a budget.

There is no single "traditional SSR" implementation. Many frameworks compile
templates ahead of time. React supports SSR, and React Server Components can
keep component source out of browser bundles. Compare concrete applications and
deployment modes, not framework names alone.

WebUI's particular choice is to render compiled HTML templates natively,
without requiring a JavaScript engine for the rendering step.

## The Insight

The template structure is known at build time; request data is not.
`webui build` records the structure in `protocol.bin`. A host loads that
artifact once, then renders it with new data for each request.

That moves template parsing and expression compilation out of the request
path. It does not eliminate data fetching, state preparation, HTML generation,
or network delivery. [How It Works](/learn/how-it-works) explains the
authoring model.

## The Web Platform Bet

WebUI uses HTML templates, custom elements, and ordinary CSS. Shadow DOM offers
native style isolation and slots; Light DOM participates in the containing
document or shadow tree's normal cascade. Light styles are not automatically
scoped to a component.

This is still a framework. Interactive WebUI components use the optional
`@microsoft/webui-framework` runtime, and soft navigation uses a separate router.
Browser primitives do not make those responsibilities disappear.

## Islands Architecture

Add a same-named `.ts` or `.js` module only when a component needs events,
lifecycle work, or another browser API. A component that only displays
server-provided data can remain HTML and CSS.

Interactive components hydrate the existing server-rendered DOM. Nested
hydration respects parent ownership; islands are not a promise that every
component activates independently of its ancestors. Scriptless components can
also activate later for browser-applied state or navigation when the runtime
is present.

### The practical difference

On a product page, the description and price can render without authored
component code, while the cart control handles clicks. You choose that boundary
in the application. Inspect the built assets and state payload to see the
actual result.

Even scriptless content costs bytes, DOM memory, style calculation, layout,
and paint. Reducing JavaScript is one part of performance, not a substitute for
measuring the page.

## Performance

WebUI is designed to reduce request-time template work and unnecessary browser
behavior. Whether it improves a particular application depends on its host,
data, component structure, assets, and devices.

The [benchmark page](/reference/benchmarks/) keeps the current results explicitly
historical and limited. The available dataset has **one measured server round**
and **ten browser navigations**; three WebUI host rows were selectively
refreshed. Its advertised source repository was unavailable anonymously during
the site audit, and complete version and revision provenance could not be
established. These results do not support a current winner or multiplier claim.

For decisions in your app, use the separate
[performance guide](/build/performance) and measure the deployment
you intend to run.

## No Node.js Required

The renderer does not require Node.js. You can use it from a native host, or
choose the Node binding when that fits your backend. npm can install the CLI
and browser packages without making Node a production rendering requirement.

Ahead-of-time native code avoids renderer JIT warmup, but cold starts still
include process startup, artifact loading, caches, and data connections.
The first request is not guaranteed to match a warm one. Host runtimes and
application code can also introduce their own allocation and scheduling costs.

## Summary

**A good fit:** a new HTML-first application, native backend integration, and
a team willing to budget browser behavior deliberately.

**Consider the tradeoffs:** templates do not execute arbitrary JavaScript;
initial display data must come from the server; interactive code needs a
bundling step; React components, hooks, and React-specific libraries do not
transfer unchanged. Check the browser APIs and host packages your deployment
requires.

[Build a first page](/start/first-page) before choosing the framework for a
larger application. If you know React, start with
[the same concepts in WebUI](/learn/react-comparison).
