# WebUI Framework Technical Specification

## Overview
WebUI Framework is a high-performance server-side rendering framework that operates without JavaScript runtimes. It separates static and dynamic content at build time, creating an efficient protocol that enables fast server-side rendering in any language (Rust, Go, C#, PHP, Ruby, etc.). On the client, Web Components hydrate as interactive islands — only components that need interactivity ship JavaScript.

### Core Architecture

The framework consists of four primary modules:

- **Protocol:** Defines the structural representation of UI components
- **Parser:** Processes HTML/CSS templates into protocol structures
- **Expression:** Evaluation*: Handles conditional logic evaluation
- **Handler:** Renders protocol with state data into final HTML output

### Performance Principles
- No recursion (all algorithms must be iterative)
- No regular expressions
- Minimal runtime computation
- Protocol Buffers for compact binary serialization
- Buffer consolidation for reduced allocations
- Strict context isolation during processing
- Proactive error handling with actionable messages

## Protocol Specification (webui-protocol)
The protocol defines the serializable structure representing UI templates. At runtime, the protocol uses protobuf for efficient binary serialization. Types are generated directly from `proto/webui.proto` using prost — there is no separate domain type layer.

### Data Types
```rust
/// The root protocol structure representing a complete webpage configuration.
/// Generated from protobuf `message WebUIProtocol`.
pub struct WebUIProtocol {
    /// Map of fragment identifiers to their associated fragment lists.
    pub fragments: HashMap<String, FragmentList>,
    /// Sorted, deduplicated CSS custom property names used across all
    /// components and entry page styles (without `--` prefix).
    pub tokens: Vec<String>,
    /// Per-component data keyed by tag name (client template + CSS).
    pub components: HashMap<String, ComponentData>,
    /// Build-wide CSS delivery strategy (Link, Style, or Module).
    pub css_strategy: CssStrategy,
    /// Build-wide DOM encapsulation strategy (Shadow or Light).
    pub dom_strategy: DomStrategy,
}

/// Per-component metadata populated by the active parser plugin at build time.
/// Framework-neutral: each plugin populates the fields it needs.
/// Generated from protobuf `message ComponentData`.
pub struct ComponentData {
    /// Client-side template string for hydration. Populated by the active
    /// parser plugin in whatever format that plugin's runtime expects.
    pub template: String,
    /// Component CSS content for the Module strategy.
    pub css: String,
    /// External stylesheet href for the Link CSS strategy.
    /// Default format is `<component-name>.css`, but build-time naming
    /// templates can produce hashed names (e.g. `my-card-a1b2c3d4.css`) and/or
    /// prepend a CDN/public base URL.
    /// Always set when CssStrategy::Link is active and the component has CSS.
    /// Empty for Style/Module strategies and for components without CSS.
    /// The handler uses `css_strategy` and `dom_strategy` on `WebUIProtocol` to
    /// decide what to emit in `<head>`:
    ///   Link + Shadow → `<link rel="preload">` (shadow root has the stylesheet)
    ///   Link + Light  → `<link rel="stylesheet">` (no shadow root to host it)
    pub css_href: String,
}

/// A list of fragments (needed because protobuf maps cannot have repeated values directly).
pub struct FragmentList {
    pub fragments: Vec<WebUIFragment>,
}

/// A mapping of unique fragment identifiers to their corresponding fragment lists.
pub type WebUIFragmentRecords = HashMap<String, FragmentList>;

/// A single fragment — one of several types.
/// Generated from protobuf `message WebUIFragment` with a `oneof fragment` field.
pub struct WebUIFragment {
    pub fragment: Option<web_ui_fragment::Fragment>,
}

/// The fragment oneof variants.
pub enum Fragment {
    Raw(WebUIFragmentRaw),
    Component(WebUIFragmentComponent),
    ForLoop(WebUIFragmentFor),
    Signal(WebUIFragmentSignal),
    IfCond(WebUIFragmentIf),
    Attribute(WebUIFragmentAttribute),
    Plugin(WebUIFragmentPlugin),
    Route(WebUIFragmentRoute),
    Outlet(WebUIFragmentOutlet),
}
```
### Fragment Types
#### Raw Fragment
```rust
pub struct WebUIFragmentRaw {
    /// The content to render.
    pub value: String,
}
```
#### Component Fragment
```rust
pub struct WebUIFragmentComponent {
    /// The identifier for the associated fragment record.
    pub fragment_id: String,
}
```
#### For Loop Fragment
```rust
pub struct WebUIFragmentFor {
    /// The name representing a singular item (e.g., "person").
    pub item: String,
    /// The collection name (e.g., "people").
    pub collection: String,
    /// The identifier for the fragment to render for each item.
    pub fragment_id: String,
}
```
#### Signal Fragment
```rust
pub struct WebUIFragmentSignal {
    /// The value or identifier of the signal.
    pub value: String,
    /// Determines if the value should be rendered as raw content.
    pub raw: bool,
}
```
#### Conditional Fragment
```rust
pub struct WebUIFragmentIf {
    /// The condition expression to evaluate.
    pub condition: Option<ConditionExpr>,
    /// The identifier for the fragment record to render if true.
    pub fragment_id: String,
}
```
#### Attribute Fragment
Attribute fragments represent dynamic HTML attributes with various binding types:
```rust
pub struct WebUIFragmentAttribute {
    /// The attribute name (may include `:` prefix for complex attributes).
    pub name: String,
    /// For simple dynamic attributes, the signal name.
    pub value: String,
    /// For mixed (template) attributes, the sub-stream fragment ID.
    pub template: String,
    /// True for `:`-prefixed complex attributes.
    pub complex: bool,
    /// True for the first dynamic attribute on a component element.
    pub attr_start: bool,
    /// True for skipped attributes (class, style, role, data-*, aria-*).
    pub attr_skip: bool,
    /// True for static attribute values on components.
    pub raw_value: bool,
    /// For `?`-prefixed boolean attributes, the condition tree.
    pub condition_tree: Option<ConditionExpr>,
}
```

##### Attribute Name Mapping

Some HTML attributes use concatenated lowercase names that do not follow
standard camelCase-to-kebab-case conversion rules. The canonical lookup
table lives in `webui-protocol` (`webui_protocol::attrs`) and covers two
categories:

1. **Multi-word ARIA attributes** — e.g., `aria-describedby` ↔
   `ariaDescribedBy`, `aria-activedescendant` ↔ `ariaActiveDescendant`,
   per the [ARIAMixin](https://w3c.github.io/aria/#ARIAMixin) specification.
2. **HTML global/element attributes** — e.g., `readonly` ↔ `readOnly`,
   `tabindex` ↔ `tabIndex`, `contenteditable` ↔ `contentEditable`.

The handler and parser both call into `webui_protocol::attrs` — there is
no duplicated table. The framework (`toKebabCase` in `decorators.ts`)
maintains a TypeScript copy of the same table for client-side use.

Attributes that follow standard conversion (e.g., `aria-label` ↔ `ariaLabel`,
`data-title` ↔ `dataTitle`) use the generic algorithm and do not require
the lookup table.

#### Plugin Fragment
Plugin fragments carry opaque data from parser plugins to handler plugins. WebUI does
not interpret this data — each parser/handler plugin pair defines its own binary contract.
```rust
pub struct WebUIFragmentPlugin {
    /// Opaque plugin-specific binary data.
    pub data: Vec<u8>,
}
```

#### Route Fragment
Route fragments define declarative URL-based routes linking path templates to fragment bodies.
The parser emits these from `<route>` elements; the handler uses them for server-side route matching.
```rust
pub struct WebUIFragmentRoute {
    pub path: String,                          // URL path template (e.g., "sections/:id")
    pub fragment_id: String,                   // Fragment containing the route body
    pub exact: bool,                           // Require exact path match
    pub children: Vec<WebUIFragmentRoute>,     // Nested child routes
    pub allowed_query: String,                 // Comma-separated allowlist of query params forwarded as attributes
    pub keep_alive: bool,                      // Preserve component across navigations
    pub cache_tags: Vec<String>,               // Cache tag templates (e.g., "thread:{threadId}")
    pub invalidates: Vec<String>,              // Tags to auto-invalidate after mutation actions
    pub pending_component: String,             // Component tag for loading UI (build-time validated)
    pub error_component: String,               // Component tag for error boundary (build-time validated)
}
```

**Cache tags:** Declared via `cache-tags="thread:{threadId},inbox"` on `<route>`. Placeholders
like `{param}` reference route path parameters from the current route or any ancestor route,
and are resolved at render time with actual values. The parser validates placeholders against
the accumulated params from the full route ancestry at build time. The handler includes resolved
tags in the JSON partial response `cacheTags` array. The client caches responses tagged with
these values and uses them for tag-based invalidation.

**Invalidation tags:** Declared via `invalidates="inbox,sent,counts"` on `<route>`. After a
mutation action (`static action()`) on this route, these tags are auto-invalidated from the
client cache. Supports `{param}` placeholders (including ancestor params). The compiler knows
the full invalidation graph - developers cannot forget to invalidate related data.

**Pending component:** Declared via `pending="mail-skeleton"` on `<route>`. The compiler validates
the component exists at build time. During slow navigations (>150ms), the router mounts this
component as a loading indicator. Skip for keep-alive and cached routes.

**Error component:** Declared via `error="error-page"` on `<route>`. The compiler validates
the component exists at build time. When a navigation fetch fails, the router mounts this
component with error details as state (`{ error, status, path }`).

There is no global route registry or route tree — routes are inline in the fragment graph
via `WebUIFragmentRoute` nesting.

#### Outlet Fragment
Outlet fragments mark where matched child route content renders inside a parent route component.
The parser emits these from `<outlet />` elements.
```rust
pub struct WebUIFragmentOutlet {}
```

Components use `<outlet />` in their templates to declare insertion points:
```html
<h1>Title</h1>
<main><outlet /></main>
```

**Route declaration:** Routes are declared as nested `<route>` elements in the entry HTML.
Child paths are relative to their parent (no leading `/`). The HTML nesting IS the route tree:

```html
<route path="/" component="app-shell">
  <route path="sections/:sectionId" component="section-page">
    <route path="topics/:topicId" component="topic-page">
      <route path="lessons/:lessonId" component="lesson-page" exact />
    </route>
  </route>
  <route path="compose" component="compose-page" query="action,to,subject" exact />
</route>
```

The optional `query` attribute declares which URL query parameters are forwarded as HTML attributes
on the component (deny-by-default). Routes without `query` forward no query params. Route path
params always take priority over query params to prevent URL-based attribute injection.

**Route matching:** The handler uses an iterative path template matcher (no regex). Segments are
compared left-to-right: `:param` binds a value, `*splat` captures remaining segments, `?` marks
optional parameters. Exact matches (most literal segments) take precedence over parameterized ones.

**Server-side rendering:** When the handler encounters `Fragment::Route`:
1. Pre-scan siblings, pick the best match by specificity.
2. Matched route: emit `<webui-route path="..." component="..." active data-ri="N">` (where N is the route chain index), render component, recurse into children. Attributes emitted on matched routes: `path`, `component`, `active`, `exact`, `pending`, `error`, `data-ri`. Routing metadata (`query`, `keep-alive`, `cache-tags`, `invalidates`) is **not** emitted as DOM attributes — it is included in the SSR `window.__webui` chain JSON instead.
3. Non-matched routes: emit `<webui-route ... style="display:none">`.

For the WebUI framework path, matched route components do **not** receive route
state as scalar attributes or `data-state`. Initial SSR state comes from the
rendered DOM plus hydration markers, and client-side navigations apply fresh
state through the partial-response `setState(...)` path.

When the handler encounters `Fragment::Outlet`:
1. Take children from the currently active route.
2. Match children against the request path (relative to route base).
3. Emit `<webui-outlet>` containing matched child `<webui-route>` with component, and hidden stubs for siblings.

The handler also emits a `<meta name="webui-nonce">` tag in `<head>` for backward compatibility,
and a nonce'd inline `<script>` containing a `window.__webui` object with the SSR metadata:

```js
window.__webui = {
  chain: RouteChainEntry[],  // matched route chain with component, path, params, exact,
                              // allowedQuery, keepAlive, pendingComponent, errorComponent,
                              // invalidates
  inventory: string,          // hex-encoded component bitmask (rendered components only)
  nonce: string,              // CSP nonce value
  css: string[],              // CSS link hrefs emitted during SSR
  styles: string[],           // module CSS specifiers emitted during SSR
  state: object,              // SSR state for hydration (consumed by framework on load)
  templates: Record<string, TemplateMetadata>,  // component template metadata (populated by IIFEs)
};
```

This replaces the previous `<meta name="webui-inventory">` and `<script id="webui-chain">` tags
with a single consolidated object. The client router reads `window.__webui` at startup instead
of querying the DOM for metadata elements. Note that **templates** and
**CSS module definitions** are emitted for all **reachable** components (including those in false
`<if>` blocks), not just rendered ones — this ensures client-side conditional activation works
without a server round-trip.

**Key elements:**
- `<webui-route>` — light DOM custom element, structural routing wrapper with no shadow DOM
- `<webui-outlet>` — light DOM custom element, marks insertion point for child route content

**Client-side navigation:**
1. On initial load, the router reads `window.__webui` for the SSR chain, inventory, and nonce. It hydrates matched `<webui-route>` elements using the `data-ri` attribute for O(1) indexed lookup instead of DOM walking.
2. `RouterConfig` supports `ssrFresh?: boolean` (default `true`) — when set, the router skips the initial loader replay because SSR state is authoritative. Components can opt into loader replay at startup by declaring `static ssrLoader = true`.
3. On navigation, fetches a partial response (`Accept: application/x-ndjson, application/json`) from the server.
4. The server returns the matched route chain — the client does NOT perform route matching.
5. Reconciles old vs new chain — finds first changed level.
6. Mounts components at changed levels, creates `<webui-route>` stubs at outlet positions.
7. Parent components and their state are preserved.

**Partial response:** `render_partial()` returns `{ templateStyles, templates, inventory, path, chain, cacheTags, cacheControl }`. The caller adds application state to the response (e.g. as a top-level `state` field for non-streaming, or as an NDJSON Chunk 2 for streaming):
- `state`: (added by caller) route-scoped application data — the router applies it to components via `setState()`
- `templateStyles`: CSS module definition tags (`<script type="importmap">{"imports":{"...":"data:text/css,..."}}</script>` strings - see [CssStrategy::Module](#css-strategy)) for newly shipped components. Empty array for Link/Style modes. The client appends these to `<head>` before evaluating template scripts so adopted stylesheets are available
- `templates`: client template script/markup payloads the client doesn't already have (filtered by inventory bitmask). Format depends on the active parser plugin
- `inventory`: updated hex bitmask of loaded templates
- `chain`: matched route chain array — each entry has `component`, `path`, optional `params`, `exact`, `allowedQuery`, `keepAlive`, `pendingComponent`, `errorComponent`, and `invalidates`
- `cacheTags`: resolved cache tags from the full route chain (union of all levels, deduplicated). The client tags its cache entry with these values for tag-based invalidation

**NDJSON streaming:** For servers that support it, the partial can be split into two NDJSON lines. Chunk 1 (chain + templates) flushes immediately for instant navigation commit. Chunk 2 (per-component states) arrives when the backend data is ready. The router reads Chunk 1, commits navigation, then applies Chunk 2 states in the background.

**Cache control:** The server can include `cacheControl: { staleTime: number }` in the partial response to override the client's default stale time for this specific route.

**Navigation cache:** The client router maintains a tagged navigation cache. Partial responses are stored keyed by request path and tagged with `cacheTags`. On revisit within `staleTime`, the cache is used and the network fetch is skipped. After a mutation action, `Router.invalidateTags()` evicts all entries whose tags overlap with the invalidated tags. Configuration: `Router.start({ cache: { staleTime, gcTime, maxEntries } })`.

**Mutation actions:** Components can declare `static action(ctx: RouteActionContext)` as the write counterpart to `static loader()`. The router intercepts `<form method="post">` submissions, finds the nearest route component's `static action()`, calls it, and auto-invalidates the cache using both the action's returned tags and the route's build-time `invalidates` attribute. This ensures the compiler-declared invalidation graph is always respected — developers cannot forget.

**Pending UI:** Routes with a `pending` attribute show a loading component during slow navigations (>150ms). The pending component is a normal WebUI component — SSR'd and build-time validated. Keep-alive and cached routes skip pending (no delay to show).

**Error boundaries:** Routes with an `error` attribute show an error component when the navigation fetch fails. The error component receives `{ error, status, path }` as state and can call `Router.navigate()` to recover.

**Request headers sent by the client router:**

| Header | Value | Purpose |
|--------|-------|---------|
| `Accept` | `application/x-ndjson, application/json` | Requests NDJSON streaming or JSON partial instead of full HTML |
| `X-WebUI-Inventory` | Hex bitmask | Templates already loaded — server skips re-sending them |

The `chain` field is produced by `render_partial()` in the handler, which walks the
fragment graph and matches routes at each nesting level using a `ProtocolIndex` for
cached route matching (see [ProtocolIndex](#protocolindex)). The function returns
chain + templates without state — the caller adds state to the response. Host servers
call this once per partial response. Available via FFI as `webui_render_partial()` for
C/.NET/Node hosts.

**Partial-template selection:** During client navigation, servers derive template names from the
normal render fragment graph starting at the persistent entry fragment. The traversal is
route-aware but state-agnostic:

- follow `component`, `if`, `for`, and attribute-template edges conservatively without evaluating
  request-time state
- when a fragment list contains sibling `<route>` fragments, follow only the single best match for
  the current request path using the same specificity rules as SSR
- recurse through nested matched route groups so the active route chain is included
- skip unvisited sibling route branches entirely; later navigations will request those templates if
  needed
- filter the discovered component set against the client's inventory bitmask before returning
  templates

This intentionally over-ships inactive conditional and loop-driven templates inside the active
route chain rather than trying to mirror a transient server-side state snapshot.

**Attribute types:**
- **Simple dynamic:** `href="{{url}}"` → `{ name: "href", value: "url" }`
- **Boolean (`?` prefix):** `?disabled={{isDisabled}}` → `{ name: "disabled", condition_tree: identifier("isDisabled") }` — rendered only if condition is truthy; silently dropped if value is not a pure handlebars expression.
- **Pass-through / property (`:` prefix):** `:config="{{settings}}"` or `:value="{{searchQuery}}"` → `{ name: ":config", value: "settings", complex: true }` — reserved for direct pass-through/property bindings, including live form-control values.
- **Mixed/template:** `value="hello {{world}}"` → `{ name: "value", template: "attr-1" }` with sub-stream `attr-1: [raw("hello "), signal("world")]`
#### Condition Expressions
Condition expressions are protobuf messages with a `oneof expr` field:
```rust
/// A condition expression tree (protobuf message with oneof).
pub struct ConditionExpr {
    pub expr: Option<condition_expr::Expr>,
}

pub enum Expr {
    Predicate(Predicate),
    Not(Box<NotCondition>),
    Compound(Box<CompoundCondition>),
    Identifier(IdentifierCondition),
}

pub struct NotCondition {
    pub condition: Option<Box<ConditionExpr>>,
}

pub struct CompoundCondition {
    pub left: Option<Box<ConditionExpr>>,
    pub op: i32, // LogicalOperator enum value
    pub right: Option<Box<ConditionExpr>>,
}

pub struct IdentifierCondition {
    pub value: String,
}
```
#### Operators
```rust
/// Logical operators for compound conditions (protobuf enum, i32 repr).
pub enum LogicalOperator {
    Unspecified = 0,
    And = 1,
    Or = 2,
}

/// Comparison operators for predicates (protobuf enum, i32 repr).
pub enum ComparisonOperator {
    Unspecified = 0,
    GreaterThan = 1,     // >
    LessThan = 2,        // <
    Equal = 3,           // ==
    NotEqual = 4,        // !=
    GreaterThanOrEqual = 5, // >=
    LessThanOrEqual = 6,   // <=
}
```
#### Predicates
```rust
pub struct Predicate {
    /// The left-hand side value.
    pub left: String,
    /// The operator used in comparison (ComparisonOperator as i32).
    pub operator: i32,
    /// The right-hand side value.
    pub right: String,
}
```
#### Serialization Requirements
- Protobuf binary serialization/deserialization as the primary format, using `prost` for direct encode/decode with no conversion layer
- Types are generated from `proto/webui.proto` at build time via `prost-build`
- JSON output supported via `webui inspect` for debugging only (using serde derives on generated types)
- Support for custom error types and propagation
- Validation of protocol structure during deserialization
- Performance optimizations for large protocol structures
- Support for fragment reference validation
- Attribute names starting with '?' are treated as boolean attributes using the `Attribute` fragment type with a `condition_tree`. The attribute is rendered only if the condition evaluates to true.

## State Management (webui-state)
### Path Resolution
The `find_value_by_dotted_path_ref` function provides the render-time state lookup contract:
```rust
pub fn find_value_by_dotted_path_ref<'a>(path: &str, state: &'a Value) -> Option<Cow<'a, Value>>
```
Existing JSON values are returned as `Cow::Borrowed` so handler and expression hot paths do not clone the state tree. Synthetic values, currently string and array `.length`, are returned as `Cow::Owned`. The owned `find_value_by_dotted_path(path, state) -> Option<Value>` wrapper is retained for API boundaries that must materialize an owned `serde_json::Value`.

### Requirements
- Dot notation support (e.g., user.profile.name)
- Special length property support for arrays and strings (e.g., users.length)
- Numeric array indexes are not resolved by dotted path lookup; loops bind array items by moniker instead
- Nullable path handling via `Option`
- Missing paths return `None`; handler text and attribute bindings render empty, and missing condition values evaluate as false

## Expression Evaluation (webui-expressions)
### Core Function
```rust
pub fn evaluate(condition: &ConditionExpr, state: &Value) -> Result<bool, ExpressionError>
```
### Evaluation Requirements
- **No recursion:** All evaluation must be iterative
- **No parentheses:** Expression grouping is handled by the ConditionExpr structure
- **Logical operators:** Support for && (AND) and || (OR) only
- **Comparison operators:** Support for >, <, ==, !=, >=, <= only
- **Negation:** Support for ! operator
- **No mixed operators:**  Cannot mix AND and OR in the same expression level
- **Operator limit:**  Maximum of 5 logical operators per expression
- **Error handling:**  Clear, actionable error messages for invalid expressions

### Error Types
```rust
pub enum ExpressionError {
    MixedOperators,
    TooManyOperators(usize),
    ValueNotFound(String),
    TypeMismatch { expected: String, found: String },
    InvalidComparison(String),
    // Other error types...
}
```

## Handler Implementation (webui-handler)
### Core API
```rust
pub struct WebUIHandler {
    plugin: Option<Box<dyn HandlerPlugin>>,
}

/// Options controlling how the handler renders a protocol.
pub struct RenderOptions<'a> {
    /// The fragment ID to start rendering from (e.g., `"index.html"`).
    pub entry_id: &'a str,
    /// The URL path to match routes against (e.g., `"/contacts/42"`).
    pub request_path: &'a str,
    /// Optional CSP nonce reflected into the `<meta name="webui-nonce">`
    /// tag and onto every SSR-emitted inline `<script>` tag (bootstrap
    /// scripts and CSS-module importmaps - see [CssStrategy::Module](#css-strategy)).
    pub nonce: Option<&'a str>,
    /// Optional HTML emitted at the structural `head_end` boundary —
    /// see [Per-Render HTML Injection](#per-render-html-injection).
    pub head_inject: Option<&'a str>,
    /// Optional HTML emitted at the structural `body_end` boundary —
    /// same contract as `head_inject`.
    pub body_inject: Option<&'a str>,
}

impl<'a> RenderOptions<'a> {
    pub fn new(entry_id: &'a str, request_path: &'a str) -> Self;
    pub fn with_nonce(self, nonce: &'a str) -> Self;
    pub fn with_head_inject(self, html: &'a str) -> Self;
    pub fn with_body_inject(self, html: &'a str) -> Self;
}

impl WebUIHandler {
    pub fn new() -> Self;
    pub fn with_plugin(plugin: Box<dyn HandlerPlugin>) -> Self;

    pub fn handle(
        &mut self,
        protocol: &WebUIProtocol,
        state: &Value,
        options: &RenderOptions<'_>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```

#### ProtocolIndex

`ProtocolIndex` is a pre-computed index over a `WebUIProtocol` that accelerates
repeated render and partial operations. It is built once from a protocol and
reused across requests:

```rust
pub struct ProtocolIndex {
    /// Maps component tag name → bit position in the inventory bitmask.
    pub component_index: HashMap<String, u32>,
    /// Pre-compiled route segment patterns for O(1) route matching.
    pub route_cache: CompiledRouteCache,
}

impl ProtocolIndex {
    pub fn new(protocol: &WebUIProtocol) -> Self;
}
```

`CompiledRouteCache` stores pre-compiled route segment patterns extracted from
the protocol's `WebUIFragmentRoute` tree. Route matching via `match_route_cached()`
uses these compiled patterns instead of re-parsing path templates on every request:

```rust
pub struct CompiledRouteCache { /* internal */ }

/// Match a request path against compiled route patterns. Returns the matched
/// route and extracted path parameters, or `None` if no route matches.
pub fn match_route_cached(
    routes: &[WebUIFragmentRoute],
    request_path: &str,
    cache: &CompiledRouteCache,
) -> Option<RouteMatch>;
```

#### Partial and Action Response Functions

```rust
/// Produce a JSON partial response for client-side navigation.
/// `protocol_index` provides cached route matching and component indices.
pub fn render_partial(
    handler: &mut WebUIHandler,
    protocol: &WebUIProtocol,
    protocol_index: &mut ProtocolIndex,
    options: &RenderOptions<'_>,
    inventory_hex: &str,
) -> Result<PartialResponse, HandlerError>;

/// Produce a response for a mutation action (POST).
/// `protocol_index` provides cached route matching and component indices.
pub fn render_action_response(
    handler: &mut WebUIHandler,
    protocol: &WebUIProtocol,
    protocol_index: &mut ProtocolIndex,
    options: &RenderOptions<'_>,
    inventory_hex: &str,
) -> Result<ActionResponse, HandlerError>;

/// Emit client template scripts/markup for the given components.
/// `protocol_index` provides the component index for inventory tracking.
pub fn render_component_templates(
    handler: &WebUIHandler,
    protocol: &WebUIProtocol,
    protocol_index: &ProtocolIndex,
    components: &[String],
) -> Vec<String>;
```

#### Component Inventory Functions

```rust
/// Parse a hex inventory bitmask into a byte vector.
pub fn parse_inventory(hex: &str) -> Result<Vec<u8>, HandlerError>;

/// Determine which components the client needs and return a filtered list
/// plus an updated inventory hex string.
pub fn filter_needed_components(
    needed: &[String],
    inventory: &[u8],
    component_index: &HashMap<String, u32>,
) -> Result<(Vec<String>, String), HandlerError>;

/// Get the list of components needed for a set of route entries.
pub fn get_needed_components(
    chain: &[RouteChainEntry],
    component_index: &HashMap<String, u32>,
) -> Vec<String>;

/// Get the list of components needed for a specific request path.
pub fn get_needed_components_for_request(
    protocol: &WebUIProtocol,
    request_path: &str,
    component_index: &HashMap<String, u32>,
) -> Vec<String>;
```

**Route-aware rendering:** The handler performs server-side route matching during
rendering. When processing `Fragment::Route`, the handler matches the route's path
template against `options.request_path`:
- **Matched route**: rendered visible (`active` attribute) with component content.
- **Non-matched routes**: rendered hidden (`style="display:none"`) and empty.

When processing `Fragment::Outlet`, the handler takes children from the active route,
matches them against the request path relative to the current route base, and emits
`<webui-outlet>` containing the matched child and hidden stubs for siblings.

This eliminates the need for post-render HTML pruning — the handler produces
correct route output in a single pass.
### Writer Interface
```rust
pub trait ResponseWriter {
    /// Write content to the output
    fn write(&mut self, content: &str) -> Result<()>;
    /// Finalize the output
    fn end(&mut self) -> Result<()>;
}
```

### Streaming Response Writers (`webui::streaming`)

Hosts that support HTTP response streaming can render directly into a
network-bound channel instead of buffering the full HTML in memory.
The `webui::streaming` module provides:

- **`StreamingWriter`** — coalesces writes into ~4 KB chunks and pushes
  them through a **bounded** `tokio::sync::mpsc::Sender<Bytes>`. The
  bound (`DEFAULT_CHANNEL_CAPACITY = 4` chunks) provides backpressure
  via `blocking_send`: a slow client parks the render thread instead
  of letting unbounded chunks accumulate. A configurable flush
  deadline (`with_flush_timeout`) caps the maximum time a producer
  thread can be parked, bounding the slow-loris DoS surface. When the
  receiver is dropped (client disconnect) or the deadline elapses,
  `write` returns a typed error (`HandlerError::ClientDisconnected` /
  `HandlerError::StreamTimeout`) so the handler aborts the render
  rather than waste CPU producing bytes that have nowhere to go.

- **`ChunkPool`** — lock-free shared pool of chunk buffers. Used via
  `StreamingWriter::new_pooled` to recycle the per-flush `Vec<u8>`
  across requests, eliminating per-flush heap allocation in
  steady-state high-RPS workloads.

### Per-Render HTML Injection

For HTML that must be spliced at the structural `</head>` or `</body>`
close (image preload `<link>` tags, dev livereload `<script>`, CSP
nonce reflections, analytics, etc.), use `RenderOptions::with_head_inject`
/ `with_body_inject`. The parser already synthesises `head_end` and
`body_end` signal fragments at the structural boundaries; the handler
emits the inject HTML there with **zero scan cost** and **no risk of
mis-firing on `</head>` / `</body>` literals appearing inside HTML
comments, `<iframe srcdoc>`, or inline scripts** (which a byte-level
scanner could).

**Safety contract — the host owns escaping.** Both inject fields
accept **raw HTML**; the handler writes them verbatim. Callers MUST
ensure the content is fully trusted (typically `&'static str` such as
a dev livereload script, or build-time-derived bytes such as image
preload `<link>` tags). Passing user-controlled content here is a
direct cross-site scripting (XSS) vector. If your call path may
include untrusted data, escape it with the host's HTML escaper (e.g.
`webui_handler::encode_safe`, re-exported from `webui_handler` for
exactly this use) **before** calling `with_head_inject` /
`with_body_inject`.

**Defensive dedup.** The handler emits each inject (and the built-in
nonce `<meta>`, CSS preload `<link>` tags, hydration `<script>`)
**exactly once per render** even when the protocol contains duplicate
`head_end` / `body_end` signals. This protects against malformed
protocols emitting a 1 MiB inject N times to amplify resource use.

**Zero-allocation borrow.** The inject fields are stored as
`Option<&'a str>` on both `RenderOptions<'a>` and the per-render
context — no `String::from` clone. A host passing a `&'static str`
pays zero per-render allocation for these tags.

**Usage (actix-web):**
```rust
let (tx, rx) = tokio::sync::mpsc::channel(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
let pool = Arc::clone(&app_state.chunk_pool); // shared, startup-time
actix_web::rt::task::spawn_blocking(move || {
    let mut writer = StreamingWriter::new_pooled(tx, pool)
        .with_flush_timeout(Duration::from_secs(30));
    let opts = RenderOptions::new(&entry, &request_path)
        .with_head_inject(preload_html)   // optional
        .with_body_inject(livereload_html); // optional
    if let Err(e) = handler.handle(&proto, &state, &opts, &mut writer) {
        log::error!("render failed: {e}");
        let _ = ResponseWriter::write(&mut writer, "<!-- webui: render error -->");
    }
    let _ = ResponseWriter::end(&mut writer);
});
let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
    .map(Ok::<bytes::Bytes, actix_web::Error>);
HttpResponse::Ok()
    .content_type("text/html; charset=utf-8")
    .insert_header(("Cache-Control", "no-store"))
    .streaming(stream)
```

**Trade-offs:**

- **Status committed before render.** Streaming sets `200 OK` and headers
  before the first chunk is generated. Render errors cannot become
  HTTP errors; hosts must `log::error!` (and ideally increment a
  `render_errors_total` metric) so ops sees them. A fixed-string
  `<!-- webui: render error -->` HTML comment is appended to the
  partial body — never the error message itself, to prevent attacker-
  controlled error text from breaking out of the comment via `-->`.
- **Streaming has a small CPU cost** vs buffering (channel sends,
  mpsc round-trips) — `StreamingWriter` adds ~5 % over a `String`
  baseline. Per-render HTML injection via `head_inject` / `body_inject`
  adds essentially zero cost (one `writer.write(inject)` call inside
  the existing `head_end` / `body_end` handler hook). The benefit
  (lower TTFB on slow renders) outweighs the cost for any render long
  enough that first-chunk latency matters; for sub-millisecond renders
  served over loopback it doesn't help. See `BENCHMARKS.md` for the
  full measurement suite (criterion + custom-allocator + HTTP-level +
  Playwright browser).

### Handler Plugin System
The handler supports framework-specific hydration plugins. Plugins receive lifecycle
callbacks during rendering and write marker formats for their framework, while shared
completion work such as rendered-component template emission stays in handler core.

```rust
pub trait HandlerPlugin {
    fn push_scope(&mut self);
    fn pop_scope(&mut self);
    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_start(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;
    fn write_route_component_state(
        &self,
        state: &serde_json::Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```

**Hook invocation points:**
- **Signal**: `on_binding_start` before, `on_binding_end` after (same scope)
- **For loop**: `on_binding_start/end` around entire loop; `on_repeat_item_start/end` + `push_scope/pop_scope` per item
- **If condition**: `on_binding_start/end` around condition; `push_scope/pop_scope` if condition is true
- **Component**: `push_scope/pop_scope` around component body
- **Plugin fragment**: `on_element_data` with parser-produced hydration bytes from protocol
- **Matched route component**: `write_route_component_state` before the opening tag closes

**Selecting handler plugins**

The CLI and host APIs select handler plugins by name (passed as a string). No plugin
is loaded by default; output is plain SSR HTML unless a plugin is selected.

The set of available plugin names is implementation-defined; refer to the CLI and
crate documentation for the current list. Each plugin emits its own framework-specific
hydration markers and attributes; WebUI itself does not interpret them.

**Usage:**
```rust
let handler = WebUIHandler::with_plugin(|| Box::new(MyHydrationPlugin::new()));
handler.handle(&protocol, &state, &options, &mut writer)?;
```
### Fragment Processing
- **Raw fragments:** Write value directly to output
- **Signal fragments:**
  - Resolve value from state using `find_value_by_dotted_path`
  - Escape value if `raw` is false, otherwise write as-is
- **Attribute fragments:**
  - **Boolean (with `condition_tree`):** Evaluate condition; if truthy, render attribute name only. If false, omit entirely.
  - **Simple dynamic (with `value`):** Resolve signal from state, render as `name="resolved_value"`.
  - **Template (with `template`):** Render `name="`, process referenced sub-stream, render closing `"`.
  - **Pass-through / property (with `complex: true`):** Same as simple dynamic on the SSR output, but reserved for `:` prefixed direct pass-through/property bindings.
- **If fragments:**
  - Evaluate condition using `evaluate`
  - If true, process referenced fragment
  - Track false conditions for template generation
  - When the `If` fragments are enclosed in one or more `For` fragments it can access the states of those `For` fragments'
    current item thorugh their corresponding item monikers. It can also access global state.
  - `If` fragment conditions can have tokens from different state objects i.e. local states from enclosing `For` fragment
    items and/or the global state mixed in the condition expression.
- **For fragments:**
  - Iterate over collection from state
  - Process referenced fragment for each item with current item's state accessible thorugh a moniker and the global state
    as a fallback.
- **Component fragments:** Process referenced fragment directly. `Component` fragments enclosed in a For fragment has access to
    the fields of the current item being looped over and the global state. The `Component` fragment doesn't need to use
    the `For` fragment item moniker and can access the fields without the qualification. If the `Component` fragment is
    nested in multiple `For` fragments only the closest enclosing `For` fragment item's state is accessible to it.
- **Plugin fragments:** Pass opaque `data` bytes to the handler plugin's `on_element_data` hook. Skipped silently when no plugin is configured.

### State Management
- Global state refers to the global application state that is available to all fragments at all times.
- Local state refers to the state corresponding to the current item being looped over in a `For` fragment.
- When nested `For` fragments are present local state of the current item being looped over for any of the `For` fragment in the
  hierarchy can be accessed through the corresponding item moniker with an exception for `Component` fragments.
- For `Component` fragments only the closest enclosing `For` fragment's current item state is available and can be accessed
  directly without the item moniker qualification. `Component` fragments also have access to the global application state.

### Error Handling
- Report missing fragment references
- Handle state resolution failures
- Propagate writer errors
- Validate protocol before processing
- Maximum recursion depth protection

## Parser Modules (webui-parser)
### Component Registry
```rust
pub struct ComponentRegistry {
    components: HashMap<String, Component>,
}

pub struct Component {
    pub name: String,
    pub html_content: String,
    pub css_content: Option<String>,
    /// Sorted, deduplicated CSS token names extracted from css_content.
    pub css_tokens: Vec<String>,
}
```

#### Registration Methods
```rust
pub fn register_component(&mut self, name: &str, html: &str, css: Option<&str>) -> Result<(), ParserError>
pub fn register_directory(&mut self, directory: &Path) -> Result<(), ParserError>
pub fn contains(&self, name: &str) -> bool
pub fn get(&self, name: &str) -> Option<&Component>
```

#### Requirements
- Validate component names (must contain hyphen)
- Lazy loading of component content
- Directory scanning with file matching
- Cache optimization for repeated lookups

### External Component Discovery (webui-discovery)

The `webui-discovery` crate discovers components from external sources. It has **no dependency on `webui-parser`** — it returns plain data structs that callers register into their component registry. This makes it reusable by CLI, FFI, and other host integrations.

#### Source Classification
```rust
enum ComponentSource {
    /// npm package: starts with `@` (scoped) or is a bare identifier
    NpmPackage(String),
    /// Local filesystem path: starts with `.`, `/`, `\`, or drive letter
    Path(PathBuf),
}
```

#### Public API
```rust
/// Discover components from a single source.
pub fn discover_source(source: &str, search_dir: &Path) -> Result<DiscoveryResult>

/// Collect resolved local paths for file watching.
pub fn collect_watch_paths(sources: &[String], search_dir: &Path) -> Vec<PathBuf>

/// A discovered component (tag name, HTML template, optional CSS).
pub struct DiscoveredComponent {
    pub tag_name: String,
    pub html_content: String,
    pub css_content: Option<String>,
    pub source: String,
}
```

#### npm Package Resolution
1. Walk up from the search directory to find `node_modules/` (Node.js-style resolution)
2. For scoped packages (`@scope`), enumerate all sub-directories
3. For each package, read `package.json`:
   - `exports["./template-webui.html"]` → template HTML path
   - `exports["./styles.css"]` → styles CSS path (optional)
   - `customElements` → path to Custom Elements Manifest
4. Parse the Custom Elements Manifest for `modules[].declarations[].tagName`
5. Return `DiscoveredComponent` structs (callers handle registration)

Conditional exports are resolved with deterministic priority: `default` → `import` → `require`.

#### Security
- **Path traversal**: Export paths are validated — absolute paths and `..` components are rejected
- **Symlink resolution**: Package symlinks are resolved via `fs::canonicalize()` to support pnpm, npm workspaces, and yarn link layouts. Path traversal safety is enforced on `package.json` export paths (not on the symlink target)
- **File size limits**: Manifests and templates are capped at 10 MB to prevent denial-of-service

#### Discovery Cache
- Location: `~/.webui/cache/components/`
- Cache key: hash of source identifier + resolved path
- Invalidation: hash of `package.json` content (re-discover on change)
- Atomic writes: temp file + rename to prevent corruption from concurrent builds
- Corrupt cache files are silently ignored (graceful fallback)

#### Local Path Resolution
Local paths perform a recursive WalkDir scan for HTML files with hyphenated names, pairing matching CSS files — the same convention used by the parser's `ComponentRegistry`.

### HTML Parser
```rust
pub struct HtmlParser {
    component_registry: ComponentRegistry,
    css_parser: CssParser,
    condition_parser: ConditionParser,
    handlebars_parser: HandlebarsParser,
    css_strategy: CssStrategy,
    // Other fields...
}
```

#### CSS Strategy
```rust
/// Strategy for how component CSS is delivered in rendered output.
pub enum CssStrategy {
    /// Emit `<link rel="stylesheet" href="./component.css">` tags for
    /// components that actually have discovered CSS (default).
    Link,
    /// Embed CSS content inline in `<style>` tags within the shadow DOM template.
    Style,
    /// Register each component's CSS module via a `<script type="importmap">`
    /// data-URI definition (one per component, deduped) and reference it via
    /// `shadowrootadoptedstylesheets` on each shadow root `<template>`.
    Module,
}
```

- **Link** (default): Emits `<link>` tags referencing external `.css` files only for components whose discovery/registration data included CSS. Used by the CLI for production builds where CSS files are served separately. Output filenames are configurable with a naming template (`[name]`, `[hash]`, `[ext]`), defaulting to `[name].[ext]`. `[hash]` is SHA-256 truncated to 8 hex chars. An optional public base prefix can be applied so protocol `css_href` values point to CDN URLs. The resolved href is used consistently for handler-emitted head links and parser/plugin-generated component template stylesheet links.
- **Style**: Embeds the full CSS content in `<style>` tags inside the shadow DOM template. Used when all files are needed in-memory.
- **Module**: Registers each component's CSS as a CSS Module via an [Import Map](https://html.spec.whatwg.org/multipage/webappapis.html#import-maps) entry whose value is a `data:text/css,...` URI. During SSR, the handler emits a `<script type="importmap">{"imports":{"component-name":"data:text/css,..."}}</script>` in each component's light DOM on first render (e.g., `<my-comp><script type="importmap">...</script><template ...>`) and adds `shadowrootadoptedstylesheets="component-name"` to each shadow root `<template>`. Components inside false `<if>` blocks or empty `<for>` loops that were not rendered during SSR get their importmap definitions emitted at `body_end`, so client-side activation can adopt them. CSS bytes are percent-encoded as needed to survive the `data:` URI parser (`%`, `#`, `"`, whitespace, and non-ASCII / control bytes); the importmap JSON object is built via `serde_json` so the specifier and URI value are correctly JSON-escaped. **Requires browser support for [Multiple Import Maps](https://github.com/WICG/import-maps/blob/main/proposals/multiple-import-maps.md) (Chrome 133+)** so each component's importmap can be emitted independently and merged into the document-level resolution table by the browser. When a CSP nonce is configured (via `RenderOptions::with_nonce` / `webui_handler_set_nonce`), the SSR-emitted `<script type="importmap">` tags include `nonce="VALUE"` (in `type`, `nonce` order) so strict `script-src 'nonce-...'` policies allow them, matching the existing nonce treatment of inline `<script>` tags. The browser registers the CSS module globally and shares a single `CSSStyleSheet` across all shadow roots that adopt it. No external CSS files are produced. During SPA partial navigation, definitions for newly needed components are sent in the `templateStyles` array as `<script type="importmap">{"imports":{...}}</script>` strings (without a `nonce` attribute - the router materializes each tag client-side and applies the per-request nonce when appending to `<head>` before executing template scripts). WebUI Framework compiled metadata carries the adopted stylesheet specifier (`sa`) so client-created components can adopt the registered stylesheet on their shadow root.

Set at construction time with `HtmlParser::with_options(ParserOptions::try_new(...))`.

#### Primary Method
```rust
pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<(), ParserError>
pub fn into_fragment_records(self) -> WebUIFragmentRecords
```

### Parser Plugin System
The parser supports a framework-aware plugin system. Plugins classify framework-owned
attributes, capture finalized component templates, and emit per-element hydration
metadata without requiring the build layer to downcast concrete plugin types.

```rust
pub trait ParserPlugin {
    fn start_fragment(&mut self, fragment_id: &str) {}
    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        processed_template: &str,
    ) -> Result<()>;
    fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction;
    fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;
    fn into_artifacts(self: Box<Self>) -> ParserPluginArtifacts;
}
```

**Hook invocation points:**
- **Fragment start**: `start_fragment` runs before each `HtmlParser::parse(...)` call so plugins can reset fragment-local counters
- **Attribute loop**: `classify_attribute` decides whether framework-owned attrs are kept, skipped, or skipped-and-counted as bindings
- **Element completion**: `finish_element` runs with the final binding count after all attrs are processed; returned bytes are emitted as a `Plugin` fragment
- **Component registration**: `register_component_template` receives the final processed component template HTML
- **Artifact extraction**: `into_artifacts` returns post-parse outputs such as client component templates without `Any` downcasts

**Selecting parser plugins**

The CLI and host APIs select parser plugins by name (passed as a string). The set
of available plugin names is implementation-defined; refer to the CLI and crate
documentation for the current list. Each plugin defines:

- Which framework-owned attributes it skips, keeps, or counts as bindings
- The opaque `Plugin` fragment payload it emits per element
- Any post-parse artifacts (e.g., client component templates) it injects at `</body>`
- Any template-syntax conversions it performs inside component templates

WebUI itself does not interpret plugin-emitted bytes; each parser plugin pairs with
a matching handler plugin that consumes them at render time. See [packages/webui-framework/README.md](packages/webui-framework/README.md)
for the WebUI Framework's public authoring model.

**Usage:**
```rust
let mut parser = HtmlParser::with_plugin(Box::new(MyParserPlugin::new()));
parser.parse("index.html", &html)?;
```

**CLI integration:**
```bash
webui build ./templates --out ./dist --plugin=<name>
webui build ./templates --out ./dist --css-file-name-template="[name]-[hash].[ext]" --css-public-base="https://cdn.example.com/assets"
webui serve ./templates --state ./data/state.json --plugin=<name>
```

`webui serve` performs a preflight bind check on its configured HTTP port and
fails before the initial build if that port is already in use, returning an
actionable message so stale dev processes can be stopped explicitly.

#### Content Processing

##### Raw Content
- Buffer content until directive or signal encountered
- Consolidate adjacent raw content
- Flush buffer when transitioning to non-raw content

##### Directive Processing
- **<for>:** Extract item/collection pair and process children into separate fragment. Empty `<for>` bodies (no children) are silently skipped.
- **<if>:** Extract and parse condition, process children into separate fragment
- **<body>:** Injects `body_start` and `body_end` raw signals around the body content
- **Components:** Check component registry, process as component if found

##### Element Processing
- Maintain proper tag structure
- Process children recursively (iterative implementation)
- Handle attributes and special elements
- Omit closing tags when the HTML parser produces no end tag (void elements, etc.)
- Handle self-closing tags (`/>` syntax) for SVG and other elements

#### Buffer Management
- **Buffer Isolation:** Isolate directive content from parent context
- **Buffer Swapping:** Save/restore parent buffer during directive processing
- **Final Flush:** Ensure all content is captured

### Handlebars Parser
```rust
pub struct HandlebarsParser;

impl HandlebarsParser {
    pub fn parse(&self, text: &str) -> Result<Vec<WebUIFragment>, ParserError>
}
```
#### Requirements
- Parse `{{variable}}` as escaped signal
- Parse `{{{variable}}}` as raw (unescaped) signal
- No support for nested handlebars expressions
- Iterative implementation (no recursion)
- Proper error handling for malformed expressions

### Condition Parser
```rust
pub struct ConditionParser;

impl ConditionParser {
    pub fn parse(&self, input: &str) -> Result<ConditionExpr, ParserError>
}
```
#### Expression Types

- **Identifiers:** Simple variable names (e.g., `isAdmin`)
- **Predicates:** Comparisons (e.g., `age > 18`)
- **Not Expressions:** Negations (e.g., `!isBlocked`)
- **Compound Expressions:** Combined with logical operators (e.g., `isAdmin && isActive`)

#### Requirements
-- No parentheses support
-- String literal support with both single and double quotes
-- Proper operator precedence
-- No recursion in implementation
-- Comprehensive error messages

### CSS Parser
```rust
pub struct CssParser {
    parser: Parser,
}

impl CssParser {
    pub fn parse(&mut self, css_content: &str) -> Result<WebUIFragmentRecords, ParserError>
    pub fn process_css(&mut self, css_content: &str, fragments: &mut WebUIFragmentRecords) -> Result<(), ParserError>
    pub fn parse_inline_css(&mut self, style_content: &str) -> Result<String, ParserError>
    pub fn extract_tokens(&mut self, css_content: &str) -> Result<HashSet<String>, ParserError>
}
```

#### Requirements
- Process CSS variables
- Extract dynamic variables with --webui- prefix
- Convert dynamic variables to signals
- Handle nested variable references
- Process inline and external CSS

### CSS Token Hoisting

CSS Token Hoisting extracts the set of CSS custom properties (tokens) that are **used** across all components and entry page styles at build time. The sorted, deduplicated list is included in the protocol's `tokens` field, enabling host runtimes to resolve only the design tokens the application actually needs.

#### Token Extraction (`CssParser::extract_tokens`)

The `extract_tokens` method uses tree-sitter-css to iteratively walk the CSS AST and extract custom property **usages** from `var()` calls, while **excluding** locally-defined custom properties.

**Extracted (hoisted):**
- `var(--colorPrimary)` → token `"colorPrimary"`
- `var(--a, var(--b, var(--c)))` → tokens `"a"`, `"b"`, `"c"` (nested fallbacks)
- `var(--size, 16px)` → token `"size"` (literal fallbacks ignored)

**Excluded (not hoisted):**
- `--bar: 12px` — local custom property definitions
- `var(--bar)` when `--bar` is defined in the same CSS file

The iterative walker visits each `call_expression` node independently, so nested `var()` fallbacks (which are separate `call_expression` nodes in the tree-sitter AST) are naturally handled.

#### Token Collection During Parsing

The `HtmlParser` maintains a `token_store: HashSet<String>` that accumulates tokens from two sources:

1. **Component CSS** — when a component is first encountered during parsing, its pre-extracted `css_tokens` (stored in the `Component` struct at registration time) are merged into the token store.
2. **Inline `<style>` tags** — when the parser processes a `style_element` node, it calls `extract_tokens` on the CSS content and merges the result.

After parsing completes, `HtmlParser::take_tokens()` returns the sorted, deduplicated token list for inclusion in the protocol.

#### Comment-Based Signal Bindings

HTML comments containing handlebars expressions are parsed as signal fragments:

```html
<!--{{tokens}}-->        → Signal { value: "tokens", raw: false }
<!--{{{tokens}}}-->      → Signal { value: "tokens", raw: true }
<!--{{tokens.light}}-->  → Signal { value: "tokens.light", raw: false }
<!-- regular comment -->  → Raw (preserved as-is)
```

This mechanism is general-purpose (not limited to `tokens`) and enables comment-based placeholders for runtime value injection in HTML files. The existing handlebars parser is reused for expression parsing within comment delimiters.

### Design Token Resolution (`webui-tokens`)

The `webui-tokens` crate provides serve-time resolution of design token values. While the parser extracts token **names** into the protocol at build time, the token resolver loads token **values** from a theme file and generates CSS declarations for injection into state.

#### Theme File Format

A multi-theme JSON file maps theme names to flat token-name → CSS-value objects:

```json
{
  "themes": {
    "light": { "surface-page": "#ffffff", "text-primary": "#111827" },
    "dark":  { "surface-page": "#171717", "text-primary": "#fafafa" }
  }
}
```

Token names omit the `--` prefix (matching the `protocol.tokens` format). Flat single-theme files (without the `"themes"` wrapper) are also supported.

#### Resolution Pipeline

```
load_token_file(path) → TokenFile
    ↓
resolve_tokens(protocol.tokens, token_file) → ResolvedTokens { css, warnings }
    ↓
inject_token_css(state, css) → state["tokens"]["light"] = "..."
```

1. **Filter**: Only tokens in `protocol.tokens` are kept.
2. **Dependency closure**: Token values referencing other tokens via `var(--x)` trigger transitive inclusion. Uses an iterative BFS expansion followed by DFS cycle detection.
3. **CSS generation**: Sorted `--name: value;` declarations. Output is deterministic.
4. **State injection**: Per-theme CSS strings are set on `state.tokens.<theme>`, where `/*{{{tokens.<theme>}}}*/` signals resolve them during rendering.

#### Package Resolution (`resolve_theme_path`)

The CLI `--theme` flag accepts a file path or an npm package name:

```bash
webui serve ./src --theme=@microsoft/webui-examples-theme
webui serve ./src --theme=./my-theme.json
```

Package names are resolved by walking up from `search_root` looking for `node_modules/<pkg>/tokens.json`. Scoped packages (`@scope/name`) and explicit subpaths (`@scope/name/custom.json`) are supported.

### Error Handling
```rust
#[derive(Debug, Error)]
pub enum ParserError {
    #[error("HTML parsing error: {0}")]
    Html(String),

    #[error("CSS parsing error: {0}")]
    Css(String),

    #[error("Directive parsing error: {0}")]
    Directive(String),

    #[error("Expression parsing error: {0}")]
    Parse(String),

    #[error("Component error: {0}")]
    Component(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```
## WebUI Framework Plugin

This section specifies only the cross-crate wire contract for `--plugin=webui`: the metadata emitted by `webui-parser`, the SSR markers emitted by `webui-handler`, and the hydration/runtime expectations consumed by `@microsoft/webui-framework`.

It intentionally does **not** duplicate package tutorials or framework API docs. Use the canonical sources instead, WebUI Framework public API, decorators, and component authoring: [packages/webui-framework/README.md](packages/webui-framework/README.md)

### Metadata object format

Each component's compiled template is registered in `window.__webui.templates[tagName]` as a marker-free metadata object consumed by the browser runtime:

| Field | Type                              | Description                                        |
|-------|-----------------------------------|----------------------------------------------------|
| `h`   | `string`                          | Marker-free static HTML for client-created DOM, including baked-in `<link>` / `<style>` nodes for link/style CSS strategies |
| `tx`  | `[slot, parts][]`                 | Client text runs inserted at precompiled slots     |
| `a`   | `CompiledAttrMeta[]`              | Attribute binding metadata                         |
| `ag`  | `[elementPath, start, count][]`   | Attribute-target groups for `a[]`                  |
| `c`   | `[ConditionExpr, blockIndex][]`   | Conditional blocks                                 |
| `cl`  | `SlotPath[]`                      | Conditional anchor slots aligned to `c[]`          |
| `r`   | `[collection, itemVar, blockIndex][]` | Repeat blocks                                  |
| `rl`  | `SlotPath[]`                      | Repeat anchor slots aligned to `r[]`               |
| `e`   | `[event, handler, needsEvent][]`  | Body events                                        |
| `el`  | `NodePath[]`                      | Event target element paths aligned to `e[]`        |
| `b`   | `TemplateBlockMeta[]`             | Nested compiled block table referenced by `c` / `r` |
| `sa`  | `string`                          | Optional module-mode adopted stylesheet specifier copied from `shadowrootadoptedstylesheets` |
| `re`  | `[event, handler, needsEvent][]`  | Root events, attached to the host element          |

All arrays are optional — omitted from the output when empty to minimize payload.

`ConditionExpr` in compiled framework metadata reuses the protocol condition AST in a compact tuple form:

- `[0, path]` — identifier / truthy path lookup
- `[1, left, operator, right]` — predicate comparison
- `[2, condition]` — logical NOT
- `[3, left, operator, right]` — logical compound (`AND` / `OR`)

Comparison operators match the protocol enum values:

- `1` = `GREATER_THAN`
- `2` = `LESS_THAN`
- `3` = `EQUAL`
- `4` = `NOT_EQUAL`
- `5` = `GREATER_THAN_OR_EQUAL`
- `6` = `LESS_THAN_OR_EQUAL`

Logical operators also match the protocol enum values:

- `1` = `AND`
- `2` = `OR`

`a[]` uses compact tuple forms to avoid runtime parsing:

- `[name, 0, path]` — simple attribute binding, e.g. `href="{{url}}"`
- `[name, 1, path]` — pass-through/property binding, e.g. `:config="{{settings}}"` or `:value="{{searchQuery}}"`
- `[name, 2, ConditionExpr]` — boolean attribute binding, e.g. `?disabled="{{expr}}"`
- `[name, 3, parts]` — mixed/template attribute binding, e.g. `class="item {{state}}"`

### Compilation rules

The Rust compiler (`generate_compiled_template` in `webui-parser/src/plugin/webui.rs`) transforms the HTML template in a single forward pass, then finalizes it into marker-free client HTML plus locator metadata:

| Source syntax                        | Metadata field(s)      | Client `h` result                 |
|--------------------------------------|------------------------|-----------------------------------|
| `{{expr}}`, `{{{expr}}}`, mixed text | `tx[]`                 | dynamic text run removed          |
| `href="{{url}}"`                     | `a[]` + `ag[]`         | element kept marker-free          |
| `class="item {{state}}"`             | `a[]` + `ag[]`         | element kept marker-free          |
| `?disabled="{{expr}}"`               | `a[]` + `ag[]`         | element kept marker-free          |
| `:config="{{settings}}"`, `:value="{{searchQuery}}"` | `a[]` + `ag[]` | element kept marker-free |
| `<if condition="expr">body</if>`     | `c[]` + `cl[]` + `b[]` | block removed; anchor slot stored |
| `<for each="v in coll">body</for>`   | `r[]` + `rl[]` + `b[]` | block removed; anchor slot stored |
| `@event="{handler(e)}"`              | `e[]` + `el[]`         | element kept marker-free          |
| `@event` on `<template>` wrapper     | `re[N]`                | *(stripped)*                      |
| `w-ref="name"`                       | *(stays)*              | *(unchanged)*                     |
| `<outlet />`                         | *(stays)*              | `<outlet></outlet>`               |

`tx[]` stores text runs as `[slot, parts]`, where `parts` reuse the compact attribute-part encoding (`string` for static text, `[path]` for dynamic text). Client-created DOM inserts one runtime `Text` node per run instead of scanning compiled marker comments.

Attribute bindings are recorded in `a[]`, while `ag[]` points at the owning element and the contiguous `[start, count)` range inside `a[]`. The compiled client HTML never embeds `data-w-*` markers; those remain SSR-only handler markers.

Nested `<if>` / `<for>` blocks are recursively compiled into the shared `b[]` block table. The client runtime instantiates compiled child blocks directly and evaluates precompiled condition AST tuples — it does not parse raw template syntax or condition strings from repeat or conditional body content.

The private workspace package `packages/webui-test-support` (`@microsoft/webui-test-support`) exists to build this metadata shape in JS-side tests without duplicating tuple encodings or fixture infrastructure across `webui-framework` and `webui-router`. It centralizes fixture builders such as `buildTemplate`, `registerCompiledTemplate`, and the condition AST helpers, and it also provides shared Node-side fixture bundling/server helpers so browser fixture apps and Playwright servers stay aligned with the runtime/compiler contract as that contract evolves.

### Plugin data and SSR hydration markers

The current WebUI parser emits a 12-byte `Plugin` fragment (`WebUIElementData`) for each element that has attribute bindings or `@event` handlers:

```
Bytes 0–3:  binding_count   (u32 LE)  — number of dynamic attribute bindings
Bytes 4–7:  event_start_idx (u32 LE)  — global index into metadata `e[]`
Bytes 8–11: event_count     (u32 LE)  — number of @event attrs on this element
```

The handler decodes this in `on_element_data` and emits SSR-only markers:

- `data-w-b-N` for one bound attribute, or `data-w-c-START-COUNT` for multiple `a[]` entries on the same element
- `data-ev="COUNT"` once per element, where `COUNT` is the number of consecutive entries in the metadata `e[]` array that belong to that element

For compatibility during mixed parser/handler rollouts, the handler also accepts the legacy 4-byte binding-only payload and upgrades it to `event_count = 0`.

WebUI SSR marker formats are:

| Marker | Format | Notes |
|--------|--------|-------|
| Repeat block start | `<!--wr-->` | Opens a `<for>` loop region |
| Repeat block end | `<!--/wr-->` | Closes the `<for>` loop region |
| Repeat item | `<!--wi-->` | Marks each iteration boundary inside a repeat |
| Conditional start | `<!--wc-->` | Opens an `<if>` block |
| Conditional end | `<!--/wc-->` | Closes the `<if>` block |

The WebUI handler plugin emits only these five comment markers. Text bindings, attribute bindings, and event handlers are resolved from compiled metadata path indices at hydration time - no DOM attribute markers are needed. The handler only emits markers in active child scopes; the root page scope remains marker-free. During hydration the framework keeps `<!--wr-->` and `<!--wc-->` as runtime anchors and removes `<!--/wr-->`, `<!--/wc-->`, and `<!--wi-->` markers.

### Runtime contract

`@microsoft/webui-framework` consumes the metadata object above plus the SSR markers emitted by `WebUIHydrationPlugin`. This follows an Islands Architecture approach: the server delivers fully-rendered HTML, and only interactive Web Components hydrate on the client — leaving static content untouched.

- SSR hydration uses one DOM walk to discover `<!--wr-->`, `<!--wi-->`, and `<!--wc-->` comment markers, wire the relevant bindings using compiled metadata path indices, then remove SSR-only markers.
- Client-created DOM never reparses template syntax; it clones marker-free `h` and resolves `tx`, `ag`, `cl`, `rl`, and `el` locators directly.
- Events are resolved from compiled `e[]` and `el[]` metadata entries using path indices. The runtime installs one delegated listener per event type on the shadow root. Root events from `re[]` attach directly to the host element.
- The full package entrypoint supports repeat metadata (`r[]` / `rl[]`). The additive `@microsoft/webui-framework/element-no-repeat` entrypoint preserves the same public `WebUIElement` API but must reject compiled templates that contain repeat metadata.

Detailed component examples, decorators, and package entrypoint guidance live in [packages/webui-framework/README.md](packages/webui-framework/README.md) rather than being duplicated in this design spec.

## Integration and Testing
### Test Suite Requirements
- Unit tests for each module
- Integration tests for complete pipeline
- Performance benchmarks
- Error case coverage

### Project Structure
```
webui/
├── crates/
│   ├── webui/                # Programmatic library API (build, inspect, re-exports)
│   ├── webui-cli/            # CLI build tool (binary: "webui")
│   ├── webui-dev-server/     # Shared dev-server toolkit (watcher, livereload, static serving) used by webui-cli and webui-press
│   ├── webui-discovery/      # External component discovery (npm, paths)
│   ├── webui-expressions/    # Expression evaluation engine
│   ├── webui-ffi/            # C-compatible FFI bindings
│   ├── webui-handler/        # Protocol handler implementation
│   ├── webui-node/           # Node.js native addon (napi-rs)
│   ├── webui-parser/         # HTML/CSS/template parser
│   ├── webui-press/          # Markdown-driven docs site generator + dev server
│   ├── webui-protocol/       # Protocol definition
│   ├── webui-state/          # State management
│   ├── webui-test-utils/     # Testing utilities
│   └── webui-wasm/           # WebAssembly bindings
├── packages/
│   ├── @microsoft/
│   │   ├── webui/            # npm package (CLI + programmatic JS API)
│   │   ├── webui-darwin-arm64/   # Platform binary (macOS ARM64)
│   │   ├── webui-darwin-x64/     # Platform binary (macOS x64)
│   │   ├── webui-linux-x64/      # Platform binary (Linux x64)
│   │   ├── webui-linux-arm64/    # Platform binary (Linux ARM64)
│   │   ├── webui-win32-x64/      # Platform binary (Windows x64)
│   │   └── webui-win32-arm64/    # Platform binary (Windows ARM64)
│   ├── webui-framework/      # WebUI Framework client runtime (@microsoft/webui-framework)
│   ├── webui-router/         # SPA router for WebUI Framework (@microsoft/webui-router)
│   └── webui-test-support/   # Private shared JS test metadata helpers (@microsoft/webui-test-support)
├── examples/                 # Example applications (todo-fast, todo-webui, routes, …)
├── docs/                     # Documentation (VitePress)
├── tests/                    # Integration tests
└── benchmarks/               # Performance benchmarks
```

### Crate Dependency Graph

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

The `webui` library crate is the primary API surface for programmatic use.
It re-exports `WebUIHandler`, `ResponseWriter`, and `WebUIProtocol` from their
respective crates and provides `build()`, `build_to_disk()`, and `inspect()`
functions with `BuildStats` (duration, fragment/component/CSS counts, protocol size).

### npm Distribution

The `@microsoft/webui` npm package follows the esbuild single-package model:
- `bin: { "webui": "bin/webui" }` — CLI binary via platform-specific `optionalDependencies`
- `main: "lib/main.js"` — Programmatic API that loads the `.node` native addon directly
- WASM fallback for render when native addon is unavailable (one-time warning logged)
### Documentation Guidelines
- Using `vitepress` in `docs/`
- API documentation for all public interfaces
- Technical explanations of algorithms
- Performance considerations
- Error handling guidelines
- Examples for all major features

## FFI Bindings (webui-ffi)

The FFI crate exposes WebUI to host languages via a C-compatible ABI. The generated
header is at `crates/webui-ffi/include/webui_ffi.h`.

### Functions

| Function | Description |
|----------|-------------|
| `webui_render(html, data_json)` | Parse + render in one call (requires `parser` feature; returns `NULL` when absent). Returns heap-allocated string (caller frees with `webui_free`). |
| `webui_handler_create()` | Create a reusable handler (no plugin). |
| `webui_handler_create_with_plugin(plugin_id)` | Create a handler with a named plugin. Returns `NULL` on error. Refer to the CLI/crate docs for the current list of plugin identifiers. |
| `webui_handler_render(handler, data, len, json, entry_id, request_path)` | Render a pre-compiled protocol with route matching. `request_path` controls which route is active. Returns heap-allocated string. |
| `webui_render_partial(protocol_data, len, entry_id, request_path, inventory_hex)` | Produce a JSON partial response (templateStyles, templates, inventory, path, matched route chain, cacheTags, cacheControl) in a single call. Uses an internal `ProtocolIndex` for cached route matching. Caller adds state. Returns heap-allocated JSON string. |
| `webui_handler_destroy(handler)` | Destroy a handler. `NULL` is a safe no-op. |
| `webui_free(ptr)` | Free a string returned by any render function. `NULL` is a safe no-op. |
| `webui_last_error()` | Return per-thread error message. Caller must **not** free. |

### Error Model
Thread-local error storage following the POSIX `dlerror()` pattern. After any
function returns `NULL`, call `webui_last_error()` for a human-readable diagnostic.

## CLI Tool (webui-cli)

The CLI specification and usage details are maintained in [crates/webui-cli/README.md](crates/webui-cli/README.md).

## Example Workflow

Examples and end-to-end walkthroughs are maintained in [examples/README.md](examples/README.md)
