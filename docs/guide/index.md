# What is WebUI Framework?

WebUI is a **language-agnostic server-side rendering framework** that compiles HTML templates into a binary protocol at build time and uses **Web Components Islands Architecture** for client-side interactivity. There is no JavaScript runtime on the server - rendering is Rust-native, sub-millisecond, and available from any backend language.

At its core, WebUI separates **static content** from **dynamic content** at build time. At runtime, the server fills in state data. On the client, Web Components hydrate as **interactive islands** - only components that need interactivity ship JavaScript. Static content stays static.

## The Problem

Traditional server-side rendering re-parses and re-evaluates templates on every request - tokenizing, building an AST, compiling expressions - even though the template structure hasn't changed. JavaScript-based SSR frameworks compound this by requiring a Node.js runtime on the server, introducing garbage collection pauses, JIT warmup costs, and high memory overhead.

On the client side, modern frameworks ship entire component trees as JavaScript bundles. The browser must download, parse, compile, and execute these bundles before anything becomes interactive - even for components that never handle a click or update their state. This sequential pipeline blocks the main thread on every page load.

Both approaches do redundant work on every request, ship unnecessary code to the browser, and scale poorly under load.

For a deeper look at the motivation behind WebUI, see [Why WebUI?](./why).

## How WebUI Solves It

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

- **Declarative templates** - HTML for structure, CSS for styling, TypeScript for behavior - in separate files. No JSX, no template literals, no CSS-in-JS. Template directives (`<if>`, `<for>`, `{{}}`) handle dynamic content declaratively.

- **Language agnostic** - Native handlers for Rust, Node/Bun/Deno, C#, Python, and Go. Any other language can use the C FFI bindings. See [Language Integrations](/guide/integrations/ffi).

- **Web Components & Shadow DOM** - Templates are standard HTML with native Web Components and Declarative Shadow DOM for style encapsulation. No virtual DOM, no proprietary component model.

- **Routing** - The `<route>` directive defines URL-to-component mappings. The server renders the matched route on first load; the optional [`@microsoft/webui-router`](/guide/concepts/routing) package adds client-side navigation with lazy loading.

- **Plugin system** - Parser and handler plugins control hydration strategies, custom directives, and framework-specific behavior. See [Plugins](/guide/concepts/plugins/).

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