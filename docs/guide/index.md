# What is WebUI Framework?

WebUI is a **language-agnostic server-side rendering framework** that compiles HTML templates into a binary protocol at build time and uses **Web Components Islands Architecture** for client-side interactivity. There is no JavaScript runtime on the server - rendering is Rust-native, sub-millisecond, and available from any backend language.

At its core, WebUI separates **static content** from **dynamic content** at build time. At runtime, the server fills in state data. On the client, Web Components hydrate as **interactive islands** - only components that need interactivity ship JavaScript. Static content stays static.

> **New here?** Jump straight into the [Playground](/playground/) or follow the [Hello World Tutorial](/tutorials/hello-world). For the design rationale, see [Why WebUI?](./why).

## How WebUI Works

WebUI splits the work into three phases:

### 1. Build - Compile templates to binary

The CLI (`webui build`) parses your HTML templates, discovers Web Components, evaluates static content, and compiles everything into a compact **Protocol Buffer binary**. Static fragments are pre-serialized. Dynamic fragments (expressions, conditionals, loops) are recorded as lightweight instructions. This happens **once**, ahead of time.

### 2. Render - Fill in state data from any backend

Your backend handler loads the pre-compiled protocol, receives state data, and produces rendered HTML. No parsing, no AST walking, no expression compilation - just sequential reads through the protocol, emitting static bytes directly and resolving dynamic fragments from the state object. This is what makes rendering sub-millisecond and language-agnostic.

### 3. Hydrate - Interactive islands come alive

On the client, only **Web Components marked as interactive** hydrate. Each component is an island - self-contained with its own Shadow DOM, styles, and behavior. A page with 10 components where only 2 need click handlers ships JavaScript for just those 2. The other 8 remain server-rendered HTML with zero client-side cost.

## Key Concepts

- **Protocol Buffer binary** - Templates compile to a compact binary format. The handler reads fragments sequentially - static fragments are emitted as-is, dynamic fragments are resolved from state. See [How It Works](/guide/concepts/how-it-works).

- **Islands Architecture** - Each Web Component is an interactive island. Static content is server-rendered with no JavaScript. Only components that need interactivity hydrate on the client. See [Interactivity](/guide/concepts/interactivity).

- **Explicit startup state boundary** - A same-named `.ts` or `.js` module opts a component into authored hydration state. Scriptless templates remain dormant at startup and activate only when browser use requires them. See [Hydration](/guide/concepts/hydration).

- **Language agnostic** - Native handlers for Rust, Node/Bun/Deno, C#, Python, and Go. Any other language can use the C FFI bindings. See [Language Integrations](/guide/integrations/ffi).

## Quick Overview

```
                        BUILD TIME                              RUNTIME
  ┌──────────────────────────────────────┐    ┌──────────────────────────────────────┐
  │                                      │    │                                      │
  │  ┌────────────┐    ┌──────────────┐  │    │  ┌────────────┐    ┌──────────────┐  │
  │  │ HTML + CSS │───▶│ webui build  │  │    │  │ JSON state │───▶│   handler    │  │
  │  │ templates  │    │  (compile)   │  │    │  │ (per req)  │    │  (any lang)  │  │
  │  └────────────┘    └──────┬───────┘  │    │  └────────────┘    └──────┬───────┘  │
  │                           │          │    │                           │          │
  │                    ┌──────▼───────┐  │    │                    ┌──────▼───────┐  │
  │                    │ .webui bin   │──╋────╋───────────────────▶│ rendered HTML│  │
  │                    │ (protocol)   │  │    │                    └──────┬───────┘  │
  │                    └──────────────┘  │    │                           │          │
  └──────────────────────────────────────┘    └──────────────────────────────────────┘
                                                                         │
                        CLIENT                                           ▼
  ┌──────────────────────────────────────────────────────────────────────────────────┐
  │                                                                                  │
  │  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐     │
  │  │  static HTML  │  │ 🏝️ island    │  │  static HTML  │  │ 🏝️ island    │     │
  │  │  (no JS)      │  │ (hydrates)   │  │  (no JS)      │  │ (hydrates)   │     │
  │  └───────────────┘  └───────────────┘  └───────────────┘  └───────────────┘     │
  │                                                                                  │
  └──────────────────────────────────────────────────────────────────────────────────┘
```

## Ready to Try It?

- **[Playground](/playground/)** - Experiment in the browser with zero setup.
- **[Installation Guide](./installation)** - Set up WebUI locally.
- **[Hello World Tutorial](/tutorials/hello-world)** - Build your first WebUI app step by step.
- **[Why WebUI?](./why)** - Understand the architecture and performance benefits in depth.