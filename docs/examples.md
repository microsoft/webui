---
title: WebUI examples
description: Start with a working WebUI page, add interactive components, and explore routing, streaming, and native backends in runnable examples.
layout: page
---

# Build something with WebUI

Start with a small working page. The examples below make one part of the
framework concrete; they are learning projects, not claims about production
deployments.

## Your first page

[Build your first app](/tutorials/hello-world/) with an HTML template and state,
then add a browser interaction. Use the
[template playground](/playground/) when you want to try HTML, CSS, and JSON
without setting up a project.

## A component with behavior

The [Todo tutorial](/tutorials/todo-app/) combines lists, component attributes,
custom events, and reactive state. Its
[source example](https://github.com/microsoft/webui/tree/main/examples/app/todo-webui)
contains the complete application and browser tests.

If you already use React, read [Coming from React](/guide/concepts/react-comparison/)
alongside the example. It explains which ideas transfer and which syntax and
state rules change.

## Explore a specific capability

| Build | What to inspect | Source |
| --- | --- | --- |
| Nested routes | A shared layout, route state, and client navigation | [Routes](https://github.com/microsoft/webui/tree/main/examples/app/routes) |
| Progressive responses | Boundaries around independently ready content | [Streaming](https://github.com/microsoft/webui/tree/main/examples/app/streaming) |
| A native backend | A Rust service rendering a complete commerce application | [Commerce](https://github.com/microsoft/webui/tree/main/examples/app/commerce) |
| Browser-hosted rendering | A service worker loading a compiled artifact and rendering public state with WASM | [Service worker](https://github.com/microsoft/webui/tree/main/examples/app/service-worker) |
| A desktop application | A native window, components, and host-side application state | [Contact book](https://github.com/microsoft/webui/tree/main/examples/app/contact-book-manager) |

Repository examples use the workspace toolchain. Follow each example's README
and package scripts; they may need additional prerequisites beyond the first
app tutorial.

## Connect your backend

Choose a [language integration](/guide/integrations/) for a reusable renderer,
request state, and response handling. The integration pages describe the actual
installation and package availability for each host.

Before adopting a pattern, measure your own application. The
[performance guide](/guide/concepts/performance/) explains what to optimize;
the [benchmark page](/benchmarks/) keeps comparison data and its limitations
separate.
