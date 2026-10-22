---
description: Start a new WebUI app with HTML templates, server-provided data, and optional browser interactivity.
---

# What is WebUI Framework?

WebUI is an HTML-first framework for native server-side rendering. You write
templates and CSS, your backend supplies JSON data, and a Rust renderer produces
HTML. Add TypeScript and the browser runtime where you need interactions.

**Start with [your first app](/tutorials/hello-world).** You'll render a page
using the locally installed npm CLI, then add a working counter. You do not need
Rust, a repository checkout, or a production server to follow along.

Coming from React? Components, state, events, and composition will be familiar.
The [React learning bridge](/guide/concepts/react-comparison) explains what
transfers and what changes. WebUI is for building a new application, not running
existing JSX or React hooks unchanged.

## How WebUI Works

### 1. Build - Compile templates to binary

`webui build` compiles HTML templates into `protocol.bin` and prepares their
styles. If the app has browser code, your JavaScript bundler builds it
separately.

### 2. Render - Fill in state data from any backend

A supported host loads the compiled protocol once and supplies data for each
request. WebUI's renderer does not execute application JavaScript to produce
HTML. The host still owns data access, authentication, and HTTP delivery.

### 3. Hydrate - Interactive islands come alive

The browser can display server-rendered content before application JavaScript
loads. Interactive components use `@microsoft/webui-framework` to connect
behavior to that HTML. Components without authored behavior need no component
module; HTML, CSS, and DOM still have transfer, rendering, and memory costs.

## Key Concepts

| Concept | What you author |
| --- | --- |
| [Templates](/guide/concepts/directives/) | HTML with bindings, conditions, and loops, not arbitrary JavaScript expressions |
| [Components](/guide/concepts/components) | A hyphenated `.html` filename, paired CSS, and an optional same-named `.ts` file |
| [State](/guide/concepts/state-management/) | The data needed for the initial HTML; browser behavior can update observable values after hydration |
| [Styling](/guide/concepts/how-it-works#light-dom-and-declarative-shadow-dom) | Ordinary global CSS in Light DOM, or native isolation in Shadow DOM |
| [Server integration](/guide/integrations) | A host around the native renderer, using a direct binding or the C ABI |

## Quick Overview

Choose a starting point:

- **Build a new app:** [Hello World](/tutorials/hello-world), then the
  [Todo tutorial](/tutorials/todo-app).
- **Try template syntax:** the [playground](/playground/) edits HTML, CSS, and
  JSON state. It is not a TypeScript application runner.
- **Evaluate the fit:** [Why WebUI?](/guide/why) describes the tradeoffs;
  [benchmarks](/benchmarks/) explains the available evidence and its limits.
- **Understand execution:** [How It Works](/guide/concepts/how-it-works)
  separates build, server, and browser responsibilities.

## Ready to Try It?

[Build your first app](/tutorials/hello-world), or check
[installation options](/guide/installation) for your toolchain.
