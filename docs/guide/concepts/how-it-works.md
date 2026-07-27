# How It Works

WebUI is a **language-agnostic server-side rendering framework** that splits template processing into three distinct phases: build, server render, and client hydration. Each phase is optimized for its role, eliminating redundant work and keeping runtime overhead to a minimum.

```
Build Time              Server Render             Client Hydration
──────────────────      ──────────────────        ──────────────────
HTML + CSS + TS    →    Protocol + JSON      →    Web Components
webui build             state → HTML              hydrate as islands
```

## Build Phase

The `webui build` CLI command transforms your template source files into an optimized binary format. This is a one-time cost - the output is reused for every request.

During the build, WebUI:

1. **Parses HTML templates** - scans component directories for `.html`, `.css`, and `.ts` files
2. **Discovers web components** - identifies custom elements by their hyphenated tag names
3. **Compiles expressions** - resolves `{{bindings}}`, `<if>` conditions, `<for>` loops, and attribute directives into indexed slots
4. **Separates static from dynamic content** - static HTML becomes pre-serialized byte fragments; dynamic content becomes keyed slots that map to state values
5. **Emits output artifacts**:
   - `protocol.bin` - binary Protocol Buffer containing the compiled template structure
   - CSS files - scoped styles for each component
   - JS bundles - client-side Web Component classes for hydration

```bash
webui build ./src --out ./dist --plugin=webui
```

The binary protocol is the key to WebUI's performance. By moving parsing, expression compilation, and template analysis to build time, the server never repeats this work.

## Server Render Phase

At runtime, the server constructs one loaded `Protocol` from the compiled bytes
at startup and reuses it for every request. Rust, Node, WASM, C, and .NET all
follow this explicit lifecycle. Protocol decoding and deterministic index
construction never occur on the request path.

For each incoming request, the handler:

1. Receives a **JSON state object** containing the data for that page
2. Walks the compiled protocol fragments in order
3. Copies **static fragments** directly to the output buffer (no processing needed)
4. Resolves **dynamic fragments** by looking up keys in the JSON state
5. Includes only the browser state needed by components on the active route
6. Emits the final HTML response

```
┌──────────────┐     ┌────────────────┐     ┌──────────────┐
│ protocol.bin │ ──→ │    Handler     │ ──→ │  HTML output  │
└──────────────┘     │  + JSON state  │     └──────────────┘
                     └────────────────┘
```

### What the handler does NOT do

- **No template parsing** - already done at build time
- **No AST walking** - the protocol is a flat list of fragments
- **No expression compilation** - expressions are pre-compiled to key lookups
- **No JavaScript runtime** - the server is pure native code (Rust, C, or any FFI host)

### Declarative Shadow DOM

The rendered HTML includes [Declarative Shadow DOM](https://developer.chrome.com/docs/css-ui/declarative-shadow-dom) markup. This means the browser can display fully styled component content **before any JavaScript loads**:

```html
<my-card>
  <template shadowrootmode="open">
    <style>/* scoped styles */</style>
    <h2>Card Title</h2>
    <p>Content rendered on the server</p>
  </template>
</my-card>
```

The user sees rendered content immediately - no blank page, no loading spinner.

## Client Hydration Phase

After the browser renders the server HTML, authored Web Components **hydrate**
as independent islands of interactivity. HTML-only components remain as
server-rendered content unless later navigation or state updates need them.

### How hydration works

1. **Custom elements upgrade** - the browser calls `connectedCallback` for each registered Web Component
2. **Shadow root detection** - the framework finds the existing Declarative Shadow DOM root (it does **not** recreate the DOM)
3. **Bindings wired** - template expressions (`{{count}}`, `?disabled`) are connected to class properties
4. **Events connected** - `@click`, `@keydown`, and other handlers are attached with their compiled argument scopes
5. **Reactive state activated** - `@observable` properties become live; changes trigger targeted DOM updates

### Islands Architecture

Each Web Component is a **self-contained island**. Components hydrate independently - a `<search-bar>` can be interactive while a `<footer-links>` component stays as static server-rendered HTML.

```
┌─────────────────────────────────────────────┐
│                  Page                        │
│                                              │
│  ┌──────────────┐   ┌────────────────────┐  │
│  │  search-bar  │   │   product-grid     │  │
│  │  (hydrated)  │   │   (hydrated)       │  │
│  └──────────────┘   └────────────────────┘  │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │          footer-links                 │   │
│  │          (static - no JS)             │   │
│  └──────────────────────────────────────┘   │
│                                              │
└─────────────────────────────────────────────┘
```

Only components that need interactivity ship JavaScript. Static content stays as plain HTML with zero JS cost.

## The SSR Mental Model

Understanding the relationship between server and client is critical for building correct WebUI applications.

### The server is the source of truth for the initial render

Every value bound in a template - `{{expression}}`, `<for each="item in items">`, `<if condition="expr">` - must have a corresponding key in the server state JSON. The handler resolves bindings by looking up keys in this JSON object. If a key is missing, the binding renders empty or the condition evaluates to false.

### Derived state belongs in the server or the template

If a value must appear in the initial HTML, it must come from the server state JSON. Use template expressions for simple derivations:

```html
<!-- Template expressions work on both server and client -->
<span>{{firstName}} {{lastName}}</span>
<if condition="items.length">{{items.length}} items</if>
```

For complex derived values, compute them on the server and include them in the state JSON.

State that participates in hydration is client-facing. When enabled with a
bundler manifest, route-scoped projection reduces serialization work and
response bytes, but it is not a secrecy boundary. Without a manifest, WebUI
preserves full state. Do not put secrets in browser render state.

### The client handles interactivity after hydration

Once components hydrate, user interactions are handled entirely on the client. Sorting a list, filtering results, toggling a panel - these operations mutate `@observable` properties directly, and the framework updates the DOM reactively.

```
Server: "Here's the initial HTML with all the data."
Client: "Got it. I'll hydrate the interactive parts and take over from here."
```

## What Makes This Fast

WebUI's architecture is designed so that the most common operation - rendering a page - does the least possible work.

| Technique | Impact |
|-----------|--------|
| **Pre-serialized static fragments** | Copied byte-for-byte to the output buffer - no processing |
| **Key-based dynamic resolution** | Simple hash map lookup against JSON state - no expression parsing |
| **Loaded protocol reuse** | Decode protobuf and build deterministic indices once at startup |
| **No JavaScript on the server** | Native code (Rust/C) handles rendering - no VM startup, no GC pauses |
| **Declarative Shadow DOM** | Browser renders content before JS loads - no white flash |
| **Islands Architecture** | Only interactive components ship JS - static content has zero client cost |
| **Binary Protocol Buffer** | Compact build artifact with no runtime template parsing |

The result: server render times measured in microseconds, not milliseconds. First Contentful Paint that doesn't depend on JavaScript. And client-side interactivity that activates without rebuilding the DOM.
