# Inside WebUI: How a Compiled Protocol Replaces Your JavaScript Runtime

*This is Part 2 of our series on WebUI. [Part 1: "Why We Rebuilt Web Rendering From Scratch"](./blog-why-we-rebuilt-web-rendering.md) covered the motivation. Here, we open the hood.*

---

Most server-side rendering frameworks interpret templates at runtime. They load a template file, walk its AST, resolve variables, and produce HTML — on every single request. WebUI does something fundamentally different: it compiles your HTML templates into a Protocol Buffer binary at build time. At runtime, any backend in any language deserializes that binary, fills in state, and gets rendered HTML. No template engine running. No JavaScript runtime required. Just a data-driven render pass over a pre-compiled graph.

In this post, we walk through every layer of how that works — from the protocol schema to the streaming renderer to the plugin system that makes hydration framework-agnostic. We include code from the actual crates, benchmark numbers you can reproduce, and the design constraints we imposed on ourselves to keep things fast.

Let's get into it.

---

## 1. Architecture Overview

WebUI is built as a collection of Rust crates — 12 in total — each with a single, well-scoped responsibility. Four of these are the core pipeline:

- **`webui-protocol`**: The Protobuf-based structural representation of UI components. This crate defines what a compiled template *is* — a graph of typed fragments serialized as Protocol Buffers. Types are generated directly from `proto/webui.proto` using [prost](https://github.com/tokio-rs/prost).

- **`webui-parser`**: The build-time compiler. It takes HTML and CSS templates, walks them with [tree-sitter](https://tree-sitter.github.io/), and emits a `WebUIProtocol` binary. This is where the heavy lifting happens — and it only happens once.

- **`webui-handler`**: The runtime renderer. Given a compiled protocol and a JSON state tree, it produces HTML output through a streaming `ResponseWriter` trait. This is the crate that ships into your production server.

- **`webui-expressions`**: Evaluates conditional logic (`<if>` directives) at render time. Conditions are pre-compiled into an AST at parse time — the handler never touches raw expression strings.

Supporting crates handle integration and distribution:

| Crate | Purpose |
|-------|---------|
| `webui-state` | JSON path resolution with dot notation and array indexing |
| `webui-discovery` | Component discovery from npm packages and local directories |
| `webui-ffi` | C ABI for cross-language integration |
| `webui-node` | Node.js native addon via napi-rs |
| `webui-wasm` | WebAssembly build for browser/edge environments |

We enforced four design principles across every crate, documented in [DESIGN.md](https://github.com/microsoft/webui/blob/main/DESIGN.md):

1. **No recursion.** Every algorithm is iterative. Templates can nest to arbitrary depth without blowing the stack. We use explicit work stacks instead.
2. **No regular expressions.** tree-sitter handles parsing. Route matching uses iterative segment comparison. There is not a single `Regex` in the dependency tree.
3. **Minimal runtime computation.** Anything that can be decided at build time — fragment boundaries, expression ASTs, CSS token lists, component metadata — is decided at build time.
4. **Protocol Buffers for serialization.** Compact binary, language-agnostic deserialization, and schema-driven evolution. A compiled template is just bytes on disk.
5. **Buffer consolidation.** Adjacent static content is merged into single allocations at parse time, reducing the number of fragments the handler has to walk.

Here is how the crates depend on each other:

```
webui-cli ──────► webui (library) ◄────── webui-node
                    │                        │
                    ├── webui-parser          ├── webui-handler
                    ├── webui-handler         ├── webui-protocol
                    ├── webui-protocol        └── serde_json
                    └── webui-discovery

webui-ffi ──────► webui-handler ◄────── webui-wasm
                  webui-parser              webui-parser
                  webui-protocol            webui-protocol
```

The left column is for server-side use (CLI, FFI). The right column is for JavaScript environments (Node.js native addon, WASM fallback). Both converge on the same core: `webui-handler` + `webui-protocol`.

---

## 2. The Protocol: Templates as Data

The central idea of WebUI is that a template is not source code to be interpreted — it is *data* to be processed. We chose Protocol Buffers as the serialization format for three reasons: compact binary representation (no parsing overhead at runtime), language-agnostic deserialization (any language with a protobuf library can render), and schema-driven evolution (we can add fragment types without breaking existing consumers).

Types are generated directly from `proto/webui.proto` using prost. There is no separate domain type layer — the protobuf types *are* the domain types. This eliminates an entire class of mapping bugs and keeps the crate count down.

The root structure:

```rust
pub struct WebUIProtocol {
    pub fragments: HashMap<String, FragmentList>,   // fragment ID → fragment list
    pub tokens: Vec<String>,                         // sorted, deduped CSS token names
    pub components: HashMap<String, ComponentData>,  // tag name → client template + CSS
}
```

`fragments` is the heart of the protocol. Each key is a fragment ID (like `"root"` or `"contact-card-body"`), and each value is an ordered list of fragment nodes. The handler walks these lists sequentially to produce output.

Fragment types are the protocol's building blocks — each one represents a different kind of template construct:

- **Raw**: Static HTML chunks. These make up the majority of output in any real template. Adjacent raw fragments are consolidated at parse time, so `<div class="header"><h1>` becomes a single Raw fragment rather than three.

- **Signal**: A `{{variable}}` placeholder resolved from state via a dotted path like `user.profile.name`. The `raw` flag distinguishes double-brace `{{escaped}}` from triple-brace `{{{unescaped}}}` output.

- **Component**: A reference to another fragment subgraph by `fragment_id`. When a component appears inside a `<for>` loop, it accesses the current item's state directly — no moniker qualification needed.

- **ForLoop**: Declares an `item` name and a `collection` path, plus a child `fragment_id`. The handler iterates the collection and sets up scoped state for each item so that `{{item.name}}` resolves correctly.

- **IfCond**: A pre-compiled condition expression tree. This is *not* a string that gets parsed at render time — it is an AST of typed nodes, compiled once by the parser. More on this below.

- **Attribute**: Dynamic attributes with binding types. Simple bindings (`href="{{url}}"`), boolean bindings (`?disabled="{{expr}}"`), property bindings (`:config="{{settings}}"`), and mixed/template bindings (`class="item {{state}}"`) are all distinct types.

- **Route / Outlet**: Declarative nested routing. A `Route` has a path template, a fragment body, an exact-match flag, and nested children. An `Outlet` marks an insertion point where matched child routes render.

- **Plugin**: Opaque bytes passed from parser plugins to handler plugins. WebUI never interprets this data — each plugin pair defines its own binary contract. This is how framework-specific hydration markers flow through the system without polluting the core protocol.

The condition expression AST deserves a closer look because it illustrates our "no runtime parsing" principle:

```rust
pub enum Expr {
    Predicate(Predicate),             // left OP right (e.g., age > 18)
    Not(Box<NotCondition>),           // !condition
    Compound(Box<CompoundCondition>), // left AND/OR right
    Identifier(IdentifierCondition),  // truthiness check (e.g., isAdmin)
}
```

A condition like `isAdmin && age > 18` is compiled at parse time into a `Compound` node with an `Identifier` left child and a `Predicate` right child. The handler evaluates this tree iteratively — no string splitting, no operator precedence parsing, no surprises.

For reference, the contact-book example app (components, loops, conditions, nested routes) compiles to approximately 28KB of protocol binary. That 28KB replaces what would otherwise be a template engine plus all its source files.

---

## 3. The Parser: Build-Time Compilation

The parser's job is to transform HTML and CSS source into a `WebUIProtocol` binary. It uses tree-sitter for both HTML and CSS parsing — no regular expressions, no hand-written tokenizer. tree-sitter gives us a concrete syntax tree with precise byte ranges, which means we can extract raw text slices without re-parsing.

The `HtmlParser` walks the tree-sitter AST iteratively. Here is the core loop's logic:

1. **Buffer raw content** until a directive or signal is encountered. Static HTML text accumulates in a string buffer.
2. **When hitting `<for>`, `<if>`, or a component tag**, swap to a new buffer. This is what we call *buffer isolation* — the parent context's raw content gets flushed as a Raw fragment, and a fresh fragment list starts for the nested scope.
3. **Process children** into a separate fragment list with a unique ID (e.g., `"for-contacts-body"`).
4. **Restore the parent buffer** and continue. The parent now contains a ForLoop, IfCond, or Component fragment that references the child fragment ID.

This buffer-swapping approach means nested constructs never interfere with each other. A `<for>` inside an `<if>` inside a component each get their own fragment list, linked by ID references.

**Component discovery** pulls from two sources:

- **Local directories**: A recursive `WalkDir` scan finds HTML files with hyphenated names (the web component naming convention). CSS files with matching names are auto-paired.
- **npm packages**: The discovery module reads `package.json` exports looking for `./template-webui.html` and `./styles.css` entries. It also reads the [Custom Elements Manifest](https://custom-elements-manifest.open-wc.org/) for tag names. Scoped packages (`@scope/pkg`) trigger enumeration of all sub-packages.

Discovery results are cached at `~/.webui/cache/components/` and invalidated when `package.json` changes. On a warm cache, component resolution adds zero I/O to the parse step.

**CSS Token Hoisting** is a less obvious but important parser feature. The parser extracts `var(--name)` usages from component CSS and inline `<style>` tags. It excludes locally-defined custom properties (those declared in the same scope). The result — a sorted, deduplicated list of token names — is stored in the protocol's `tokens` field. This lets design system tooling know exactly which tokens a compiled template depends on, without re-parsing CSS at runtime.

**Parser plugins** hook into four extension points: `classify_attribute` (decide whether to keep, skip, or count an attribute), `finish_element` (emit Plugin fragment bytes after processing an element), `register_component_template` (customize how component templates are stored), and `into_artifacts` (produce additional build outputs like client-side metadata).

---

## 4. The Handler: Streaming Renderer

The handler is where state meets structure. Given a compiled `WebUIProtocol` and a JSON state tree, it walks the fragment graph and produces HTML. The key interface is `ResponseWriter`:

```rust
pub trait ResponseWriter {
    fn write(&mut self, content: &str) -> Result<()>;
    fn end(&mut self) -> Result<()>;
}
```

This trait is intentionally minimal. An HTTP server can implement it to flush chunks as they are produced — true streaming SSR with no buffering requirement. A `StringWriter` implementation collects everything into a `String` for simpler use cases.

Fragment processing is entirely iterative. For each fragment in a list:

- **Raw**: Write directly to output. No processing, no allocation — just a `writer.write()` call with the pre-consolidated static content.

- **Signal**: Resolve the variable from state using `find_value_by_dotted_path`, then HTML-escape the result (unless `raw: true`). Escaping uses a stack buffer, not `format!()`.

- **ForLoop**: Look up the collection in state, then iterate. For each item, the handler sets up scoped state so that `{{item.field}}` resolves against the current element. Child fragments are processed for each iteration. No intermediate `Vec` is allocated for the iteration — we walk the JSON array directly.

- **IfCond**: Evaluate the pre-compiled condition AST using the iterative expression walker (no recursion, no string parsing). If the condition is true, process the child fragments. If false, skip them entirely.

- **Component**: Push a new scope onto the state stack, process the referenced fragment subgraph, then pop the scope. Scope isolation ensures components do not leak state.

- **Route**: Match the path template against the request URL using iterative segment comparison. `:param` segments bind values into state. `*splat` captures remaining segments. Matched routes render with an `active` attribute; non-matched routes render with `style="display:none"`. This is a single-pass operation — no post-render DOM pruning.

- **Plugin**: Pass the opaque bytes to the handler plugin's `on_element_data` hook. The handler has no knowledge of what these bytes mean — that is the plugin's contract.

**State resolution** via `find_value_by_dotted_path` handles several patterns: dot notation (`user.profile.name`), array indexing (`users.0.name`), and the synthetic `.length` property on arrays. When resolving inside a `<for>` loop, local state from the enclosing item takes precedence over global state. This scoping rule is critical — it means `{{name}}` inside `<for item="contact">` resolves to `contact.name`, not the root-level `name`.

**Expression evaluation** walks the condition AST iteratively, using an explicit value stack. The evaluator supports `&&`, `||`, `!`, and comparison operators (`>`, `<`, `==`, `!=`, `>=`, `<=`). We enforce a maximum of 5 logical operators per expression and prohibit mixed `&&`/`||` at the same nesting level. These constraints keep evaluation O(1) in practice and prevent developers from embedding business logic in templates.

---

## 5. The Plugin System: Framework-Agnostic Hydration

Server-side rendering is only half the story. For interactive applications, the client needs to "hydrate" the server-rendered HTML — attaching event listeners, binding reactive state, and making the page interactive. Different frameworks do this differently. WebUI's plugin system makes the hydration strategy a build-time choice, not a framework lock-in.

Plugins operate at two stages:

```
Build time (Parser Plugin)         Runtime (Handler Plugin)
┌──────────────────────────┐       ┌──────────────────────────┐
│ Skip framework attrs     │       │ Inject hydration markers │
│ Track components         │  ───► │ Manage scope counters    │
│ Emit opaque Plugin data  │       │ Process Plugin data      │
│ Inject content at </body>│       │ Wrap bindings/repeats    │
└──────────────────────────┘       └──────────────────────────┘
```

The parser plugin decides what to *exclude* from the protocol (framework-specific attributes that are client-only) and what metadata to *attach* (as opaque Plugin fragment bytes). The handler plugin decides what markers to *inject* into the rendered HTML so the client runtime can efficiently locate dynamic content.

### FAST-HTML Plugin (`--plugin=fast`)

The FAST-HTML plugin targets Microsoft's [FAST](https://www.fast.design/) web component framework.

At **parse time**, the parser plugin skips attributes that are purely client-side concerns: `@click`, `f-ref`, `f-slotted`, `f-children`. It counts dynamic bindings per element and emits that count as 4 bytes (u32 little-endian) in a Plugin fragment. This count is all the handler plugin needs to generate correct attribute indices.

At **render time**, the handler plugin wraps signals and loops in comment markers:

```html
<!--fe-b$$start$$0$$name$$fe-b-->John Doe<!--fe-b$$end$$0$$name$$fe-b-->
```

Repeat items get their own markers:

```html
<!--fe-repeat$$start$$0$$fe-repeat-->
  <li>Item 1</li>
<!--fe-repeat$$end$$0$$fe-repeat-->
```

Elements with dynamic bindings receive `data-fe-b-N` attributes (binding index) and `data-fe-c-START-COUNT` attributes (consecutive binding ranges). These markers let FAST-HTML's client runtime scan the DOM once, build a binding map, and re-attach to server-rendered content without re-rendering.

### WebUI Framework Plugin (`--plugin=webui`)

The WebUI framework plugin takes a different approach. Instead of embedding markers for an external framework, it compiles templates into compact metadata objects stored in `window.__webui_templates[tagName]`.

Each metadata object has fields for: `h` (marker-free HTML), `tx[]` (text binding runs), `a[]` (attribute bindings), `c[]`/`r[]` (conditionals and repeats), and `e[]` (event bindings). The handler emits `<!--w-b:start:INDEX:NAME-->` / `<!--w-b:end:INDEX:NAME-->` markers and `data-w-b-N` / `data-w-c-START-COUNT` / `data-ev="COUNT"` attributes.

The Plugin fragment for the WebUI framework is 12 bytes: `[binding_count: u32, event_start_idx: u32, event_count: u32]`.

### Custom Plugins

Both `ParserPlugin` and `HandlerPlugin` are Rust traits. To build a custom plugin:

1. Implement `classify_attribute` to decide which attributes to keep, skip, or count.
2. Implement `finish_element` to emit Plugin fragment bytes.
3. Implement `on_element_data` on the handler side to process those bytes during rendering.

The core protocol never interprets Plugin fragment bytes — your plugin pair owns the contract end to end.

---

## 6. Multi-Language Integration

A compiled WebUI protocol is just bytes. The `webui-ffi` crate exposes 6 C functions that any language with a C FFI can call:

| Function | Purpose |
|----------|---------|
| `webui_render(html, data_json)` | Parse + render in one call (convenience) |
| `webui_handler_create()` | Create a reusable handler instance |
| `webui_handler_create_with_plugin(plugin_id)` | Create handler with a specific plugin |
| `webui_handler_render(handler, data, len, json, entry_id, request_path)` | Render pre-compiled protocol with state |
| `webui_render_partial(...)` | Produce JSON partial response for client navigation |
| `webui_free(ptr)` / `webui_last_error()` | Memory management and error reporting |

The memory model follows the POSIX `dlerror()` pattern: errors are stored in thread-local storage and retrieved with `webui_last_error()`. Render results are heap-allocated C strings — the caller is responsible for freeing them with `webui_free()`.

Here is what integration looks like in each language:

**Rust** (direct crate dependency — no FFI needed):

```rust
use webui_handler::{WebUIHandler, RenderOptions, StringWriter};

let handler = WebUIHandler::new();
let mut writer = StringWriter::new();
handler.handle(&protocol, &state, &options, &mut writer)?;
println!("{}", writer.into_string());
```

**Node.js** (native addon via napi-rs):

```js
import { render } from "@microsoft/webui";

const html = render("./templates", { name: "World" });
```

The `@microsoft/webui` npm package ships a native addon for each platform. On platforms where the native binary is not available, it falls back to the WASM build — same API, same output, slightly slower.

**C#** (P/Invoke):

```csharp
[DllImport("webui_ffi")]
static extern IntPtr webui_render(string html, string json);

[DllImport("webui_ffi")]
static extern void webui_free(IntPtr ptr);

IntPtr result = webui_render(html, json);
string output = Marshal.PtrToStringUTF8(result)!;
webui_free(result);
```

**Python** (ctypes):

```python
import ctypes

lib = ctypes.cdll.LoadLibrary("libwebui_ffi.so")
lib.webui_render.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
lib.webui_render.restype = ctypes.c_void_p
lib.webui_free.argtypes = [ctypes.c_void_p]

ptr = lib.webui_render(b"<h1>{{name}}</h1>", b'{"name":"World"}')
result = ctypes.cast(ptr, ctypes.c_char_p).value.decode()
print(result)
lib.webui_free(ptr)
```

**Go** (cgo):

```go
// #cgo LDFLAGS: -lwebui_ffi
// #include "webui_ffi.h"
import "C"
import "unsafe"

func Render(html, json string) string {
    cHtml := C.CString(html)
    cJson := C.CString(json)
    defer C.free(unsafe.Pointer(cHtml))
    defer C.free(unsafe.Pointer(cJson))

    ptr := C.webui_render(cHtml, cJson)
    defer C.webui_free(ptr)
    return C.GoString(ptr)
}
```

In every case, the pattern is the same: load the shared library, call `webui_render` or the handler API, read the result string, free the pointer. The protocol binary is identical regardless of which language produced or consumed it.

---

## 7. Routing: Server-First, Client-Enhanced

WebUI's routing model starts on the server and extends to the client — not the other way around. Routes are declared as nested `<route>` elements in your template, and the HTML nesting *is* the route tree:

```html
<route path="/" component="app-layout">
  <route path="contacts" component="contact-list" exact />
  <route path="contacts/:id" component="contact-detail" exact />
</route>
```

### Path Matching

Route matching uses iterative segment comparison — no regular expressions. The matcher walks both the pattern and the URL segment by segment:

- **Literal segments** must match exactly.
- **`:param` segments** bind the matched value into state (e.g., `:id` → `params.id`).
- **`*splat` segments** capture all remaining path segments.
- **`?` suffix** marks a segment as optional.

When multiple routes could match, the most specific one wins. Specificity is determined by: literal segments beat params, params beat splats, longer matches beat shorter ones.

### Server Rendering

On the initial page load, the handler renders the full route tree in a single pass. Matched routes get an `active` attribute on their `<webui-route>` element. Non-matched routes are rendered with `style="display:none"` — they are present in the HTML but hidden. This means the client has the full route tree available in the DOM without needing a second render pass.

Components use `<outlet />` in their templates to declare where child routes render. The `<route>` element's `component` attribute maps the URL segment to a specific web component tag name.

### Client-Side Navigation

For subsequent navigations, the `@microsoft/webui-router` package takes over:

1. **Intercept**: Link clicks are intercepted via the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API).
2. **Fetch**: The router requests the new URL from the server with `Accept: application/json`.
3. **Partial response**: The server returns `{ state, templates, inventory, chain }` — a JSON payload with the new state, any templates the client does not have yet, and the matched route chain. The server performs route matching, not the client.
4. **Diff**: The router compares the old route chain with the new one and finds the first level that changed.
5. **Mount**: Only the changed components are re-rendered and mounted. Parent components that appear in both chains stay mounted — their state is preserved.

The **inventory bitmask** is a key optimization. A `<meta name="webui-inventory">` tag in the initial HTML tracks which templates the client already has. On subsequent navigations, the server checks this bitmask and only sends templates the client is missing. This prevents duplicate template transfers across navigations.

For long-lived single-page applications, `Router.releaseTemplates()` lets you explicitly free templates that are no longer needed, preventing unbounded memory growth.

---

## 8. Benchmark Deep-Dive

We benchmark WebUI with [Criterion.rs](https://github.com/bheisler/criterion.rs) using 30-second measurement windows and 95% confidence intervals. All numbers below are reproducible with `cargo xtask bench all` from the repository root.

### SSR Performance Showdown

The `examples/integration/ssr-performance-showdown/` directory contains a head-to-head comparison of WebUI against Node.js frameworks on a realistic workload: ~2,400 tiles computed per request, comparable to the [fastify-html benchmark](https://github.com/nicolo-ribaudo/fastify-html). Load testing uses `autocannon -c 100 -d 10 -w 2` (100 concurrent connections, 10-second duration, 2-second warmup).

| Framework | Avg Latency | p50 | p99 | Req/Sec | Throughput |
|-----------|------------|-----|-----|---------|------------|
| **WebUI (Rust)** | **21.7ms** | **18ms** | **52ms** | **4,511** | **684 MB/s** |
| Fastify (Node.js) | 93.4ms | 92ms | 118ms | 1,061 | 209 MB/s |
| React SSR | 179.2ms | 180ms | 210ms | 552 | 78.5 MB/s |

WebUI handles 4.3× the throughput of Fastify and 8.2× that of React SSR. The p99 latency (52ms) is lower than Fastify's median (92ms). This is the direct result of pre-compiled templates — the handler is not parsing template syntax on every request.

### Contact Book Benchmark

The contact book is a more realistic app with components, `<for>` loops, `<if>` conditions, and nested state. The protocol compiles to ~28KB of binary.

| Workload | Render Time | HTML Output |
|----------|------------|-------------|
| Protocol parse | 0.05ms | 28KB binary |
| 10 contacts | 0.65ms | 25KB HTML |
| 100 contacts | 4.94ms | 56KB HTML |
| 1,000 contacts | 57.5ms | 363KB HTML |
| 1,000 contacts + FAST plugin | 59.5ms | ~375KB HTML |

The FAST hydration plugin adds approximately 2–3% overhead (59.5ms vs 57.5ms at 1,000 contacts). That overhead comes from the additional comment markers and data attributes the plugin injects — a fixed per-binding cost, not a per-byte cost.

### Per-Module Benchmarks

Each core crate has its own benchmark suite:

- **Parser**: Measures realistic app templates, attribute and directive scaling, and component resolution. Parse time scales linearly with template complexity.
- **Handler**: Measures loop scaling (10 → 2,000 items), state depth (1 → 8 levels of nesting), and nested component rendering. Loop scaling is near-linear thanks to the no-allocation iteration strategy.
- **Expressions**: Measures compound conditions, realistic auth and state patterns, and edge cases. Evaluation is constant-time for any expression within the 5-operator limit.
- **Protocol**: Measures serialize/deserialize round-trips and a 5 → 50 component size sweep. Deserialization cost is dominated by protobuf decode, which is proportional to binary size.

### Performance Rules

We enforce several performance rules in the codebase, documented in comments and enforced in code review:

- **No cloning large state trees.** State is passed by reference; closures capture borrows, not owned copies.
- **No `format!()` in writer output.** Sequential `writer.write()` calls avoid intermediate `String` allocations. Writing `"<div class=\""` then `class_value` then `"\">"` is three writes, zero allocations.
- **No `.collect::<Vec<_>>()` on splits.** When splitting a dotted path like `"user.profile.name"`, we iterate the split directly without collecting into a `Vec`.
- **No `String::from(ch)` in escape loops.** HTML escaping uses stack-allocated buffers (`&[u8]` slices) for replacement strings like `&amp;`.
- **No per-request template re-parsing.** The protocol is parsed once at server startup and reused across all requests. `webui_handler_create()` exists specifically for this pattern.

---

## 9. Try It Yourself

The fastest way to see WebUI in action:

```bash
npm install @microsoft/webui
```

Or if you prefer working directly in Rust:

```bash
cargo install microsoft-webui-cli
```

**Hello World in 30 seconds:**

```bash
mkdir my-app && cd my-app
echo '<h1>Hello, {{name}}!</h1>' > index.html
npx webui serve . --state '{"name": "World"}' --port 3000
```

Open `http://localhost:3000` and you will see your rendered page. Change the `--state` JSON and refresh — the handler re-renders with the new state instantly.

**Example apps** in the repository cover a range of complexity:

| Example | What it demonstrates |
|---------|---------------------|
| `hello-world` | Minimal template + state |
| `calculator` | Interactive components, event bindings |
| `todo-fast` | FAST-HTML hydration plugin |
| `todo-webui` | WebUI framework plugin |
| `commerce` | Components, loops, conditions, nested state |
| `routes` | Nested routing with outlets |

**Interactive playground**: Visit [microsoft.github.io/webui/playground/](https://microsoft.github.io/webui/playground/) to experiment with templates and state in the browser — powered by the WASM build.

**Run the benchmarks** yourself:

```bash
cargo xtask bench all
```

This runs the full Criterion.rs suite across all crates. Results are written to `target/criterion/` with HTML reports you can open in a browser.

**Contributing**: WebUI is MIT-licensed and open to contributions. A CLA bot will prompt you on your first PR. Before submitting, run:

```bash
cargo xtask check
```

This executes the full lint → test → build → docs pipeline and catches most issues before CI does.

We are building WebUI in the open because we believe server-side rendering should not be tied to a single language or framework. The protocol is the contract. Everything else is an implementation detail.

*— The Microsoft Edge WebUI Team*

---

*WebUI is open source at [github.com/microsoft/webui](https://github.com/microsoft/webui). Documentation at [microsoft.github.io/webui](https://microsoft.github.io/webui). File issues, ask questions, send PRs — we are here.*
