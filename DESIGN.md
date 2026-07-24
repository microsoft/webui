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
    /// Per-component data keyed by tag name (client template metadata + CSS).
    pub components: HashMap<String, ComponentData>,
    /// Build-wide CSS delivery strategy (Link, Style, or Module).
    pub css_strategy: CssStrategy,
    /// Build-wide DOM encapsulation strategy (Shadow or Light).
    pub dom_strategy: DomStrategy,
    /// Full initial state or WebUI per-component projection.
    pub initial_state_strategy: InitialStateStrategy,
}

/// Per-component metadata populated by the active parser plugin at build time.
/// Framework-neutral: each plugin populates the fields it needs.
/// Generated from protobuf `message ComponentData`.
pub struct ComponentData {
    /// Non-WebUI client-side template payload, such as FAST `<f-template>` HTML.
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
    /// WebUI plugin JSON-safe component metadata.
    pub template_json: String,
    /// WebUI plugin component-local JavaScript condition closure array.
    pub template_functions: String,
    /// Sorted, deduplicated keys used when `hydration_mode` is `Keys`.
    pub hydration_keys: Vec<String>,
    /// Sorted, deduplicated keys used when `navigation_mode` is `Keys`.
    pub navigation_keys: Vec<String>,
    /// `None`, `Keys`, or correctness-safe `All` initial hydration state.
    pub hydration_mode: StateProjectionMode,
    /// `None`, `Keys`, or correctness-safe `All` partial-navigation state.
    pub navigation_mode: StateProjectionMode,
}

pub enum InitialStateStrategy {
    Full = 0,
    Components = 1,
}

pub enum StateProjectionMode {
    None = 0,
    Keys = 1,
    All = 2,
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

For plugins that participate in client routing, the handler also emits a
`<meta name="webui-nonce">` tag in `<head>` for CSP nonce discovery, an inert
`<script type="application/json" id="webui-data">` data block containing shared
non-executable SSR metadata:

```html
<script type="application/json" id="webui-data">
{
  "chain": [{ "component": "app-shell", "path": "/" }],
  "inventory": "0c",
  "nonce": "abc123",
  "css": ["todo-app.css"],
  "styles": ["todo-app"],
  "state": { "title": "Todo List" }
}
</script>
```

This is the single metadata startup contract. The client packages first read any existing `window.__webui`, then
lazily parse and remove `#webui-data` into `window.__webui` when metadata is needed. Note that
**CSS module definitions** are emitted for all **reachable** components (including those in false
`<if>` blocks), not just rendered ones.

`initial_state_strategy` controls the `state` field. Default and non-WebUI
plugin builds use `Full` and serialize the complete state object. WebUI builds
use `Components`: the handler walks components reachable from the active entry
and request route and combines their explicit `hydration_mode` values.
`All` immediately selects complete state, `Keys` contributes its sorted key
list, and `None` contributes nothing. Unknown numeric enum values also select
complete state. Components behind active-route `<if>` and `<for>` branches
remain conservatively reachable; inactive sibling routes are excluded. If every
reachable WebUI component is exactly `None`, the handler writes `"state":{}`
without serializing the state value.

Projection keeps startup cost proportional to the active proven hydration
surface. It is a payload boundary, not a secrecy boundary: any state sent for
initial hydration or partial navigation is client-facing. Hosts must never put
secrets in browser render state. See
[Hydration keys and state projection](#hydration-keys-and-state-projection).

Plugins can still emit executable side-channel data after the inert block. The WebUI framework
plugin uses that extension point to install component-local
`window.__webui.templateFns[tagName]` closure arrays, paired with JSON-safe `templates` in
`webui-data`. FAST plugins emit their own `<f-template>` payloads and hydration markers, so they
use the shared router metadata (`chain`, `inventory`, `nonce`, `css`, `styles`, `state`) but do not
emit WebUI `templates` or `templateFns`.

**Key elements:**
- `<webui-route>` — light DOM custom element, structural routing wrapper with no shadow DOM
- `<webui-outlet>` — light DOM custom element, marks insertion point for child route content

**Client-side navigation:**
1. On initial load, the router reads `window.__webui` for the SSR chain, inventory, and nonce. It hydrates matched `<webui-route>` elements using the `data-ri` attribute for O(1) indexed lookup instead of DOM walking. While active, it installs a nonce-bearing `@view-transition { navigation: none; }` override and removes it on `destroy()`. This disables automatic cross-document transitions without affecting explicit same-document `document.startViewTransition()` commits.
2. `RouterConfig` supports `ssrFresh?: boolean` (default `true`) — when set, the router skips the initial loader replay because SSR state is authoritative. Components can opt into loader replay at startup by declaring `static ssrLoader = true`.
3. On navigation, fetches a partial response (`Accept: application/x-ndjson, application/json`) from the server.
4. The server returns the matched route chain; the client does NOT perform route matching.
5. Newly received templates are registered and published through `webui:templates-registered`, allowing the framework to define compiler-owned hosts before commit.
6. Configured authored loaders run. If the destination tag is still unregistered, the router performs document navigation.
7. Otherwise, the router reconciles old vs new chain — finds first changed level.
8. Mounts components at changed levels, creates `<webui-route>` stubs at outlet positions.
9. Parent components and their state are preserved.

**Partial response:** `Protocol::render_partial()` returns the complete response
with projected top-level `state`. Raw-state input is validated
with a streaming serde visitor that enforces `serde_json::Value` numeric limits,
skips unselected values without materializing them, and borrows selected raw
values into the response. FFI, Node, WASM, and .NET expose only the complete
`renderPartial` contract.

- `state`: route-scoped navigation data projected with each reachable component's `navigation_keys`; included by complete-response host APIs or supplied as NDJSON Chunk 2 by a streaming host. The router applies it to components via `setState()`
- `templateStyles`: CSS module definition tags (`<script type="importmap">{"imports":{"...":"data:text/css,..."}}</script>` strings - see [CssStrategy::Module](#css-strategy)) for newly shipped components. Empty array for Link/Style modes. The client appends these to `<head>` before installing template closure arrays so adopted stylesheets are available
- `templates`: JSON-safe authored and compiler-owned template metadata keyed by component tag, filtered by inventory bitmask
- `templateFunctions`: JavaScript condition closure array strings keyed by component tag, filtered alongside `templates`; omitted or empty for templates with no conditions
- `inventory`: updated hex bitmask of loaded templates
- `chain`: matched route chain array. Each entry has `component`, `path`, optional `params`, `exact`, `allowedQuery`, `keepAlive`, `pendingComponent`, `errorComponent`, and `invalidates`
- `cacheTags`: resolved cache tags from the full route chain (union of all levels, deduplicated). The client tags its cache entry with these values for tag-based invalidation

**NDJSON streaming:** For servers that support it, the partial can be split into two NDJSON lines. Chunk 1 (chain + templates) flushes immediately for instant navigation commit. Chunk 2 (per-component states) arrives when the backend data is ready. The router reads Chunk 1, commits navigation, then applies Chunk 2 states in the background.

**Cache control:** The server can include `cacheControl: { staleTime: number }` in the partial response to override the client's default stale time for this specific route.

**Static component assets:** `webui build --emit-component-assets mail-thread,compose-page`
emits CDN-loadable component asset files next to `protocol.bin`. The flag is a
strict comma-separated allowlist of root component tags; every tag must be a
discovered lowercase kebab-case component with WebUI template metadata. Static
component asset runtimes are framework-owned: the WebUI Framework loader lives at
`@microsoft/webui-framework/component-asset.js`; a FAST runtime should define its
own asset loader rather than making the core `@microsoft/webui` package know
plugin details. Asset roots are parsed into the protocol through synthetic
non-entry fragments, so they do not become reachable from the SSR entry tree and
are not included in the initial SSR bootstrap unless the entry graph also
references them. `webui serve --emit-component-assets` parses and validates the
same roots on every dev build — surfacing their HTML and theme-token errors even
though they are outside the SSR tree — and serves the compiled modules from
memory. Asset generation is parallelized across requested roots. Each root produces one
standard ESM module, `<tag>.webui.js`, by default. Use
`--asset-file-name-template "[name]-[hash].[ext]"` for CDN-cacheable CSS and
component asset names; `[hash]` is the emitted file's SHA-256 content hash
truncated to 8 hex characters and `[ext]` resolves to `webui.js` for component
assets. Programmatic Rust builds expose the rendered files through
`BuildResult::component_asset_files`; `build_to_disk()` and the CLI validate
protocol/CSS/component-asset filenames as one output set before writing any
file. The module default-exports:

```js
export default {
  type: "webui-component-asset",
  version: 1,
  components: ["mail-thread", "mail-message"],
  templateStyles: [],
  templates: {},
  templateFunctions: {
    "mail-thread": [function(v, s) { return !!v("hasMessages", s); }]
  }
};
```

The component list is the conservative dependency closure for the requested root:
component edges, `<if>`, `<for>`, attribute-template edges, and all nested
`<route>` branches are followed without evaluating runtime state. The JSON file
is inert data and intentionally omits `inventory`: a build-time static asset does
not know the page's current loaded bitset, so consumers must not replace
`window.__webui.inventory` with asset-local state. Component-local condition
closures are carried in the same ESM request as `templateFunctions`, so the
template asset, component class chunk, and component data request can all start
in parallel from the manifest. CSS module importmaps still use the page's current
CSP nonce when materialized by the optional
`@microsoft/webui-framework/component-asset.js` `defineComponentAssets()`
manifest loader. The manifest loader exposes `preload(tag)` to start asset,
module, and data work, and `create(tag)` to create the element after
template/module work is ready. This loader is not re-exported from the framework
root package entrypoint, keeping it out of normal framework bundles unless an app
imports the optional subpath. The loader uses the manifest tag as the
registered-template fast path, so hashed asset filenames still skip importing when
`window.__webui.templates[tag]` already exists. Otherwise it deduplicates
in-flight imports by resolved asset URL and deduplicates module-style importmaps
against `window.__webui.styles` plus previously injected asset styles.
`create(tag)` waits for the asset/module, mounts without blocking on data by
default, and applies data later; callers can opt into bounded data blocking with
`{ awaitData: true, dataTimeoutMs }`.

FAST plugin builds can emit the same ESM asset shape with trusted `<f-template>`
payloads in `templates`; those assets require a FAST-owned runtime loader.

**Navigation cache:** The client router exposes an optional tagged navigation
cache tier. The default `Router.start()` path does not import or instantiate the
cache. `Router.start({ cache: { staleTime, gcTime, maxEntries } })` enables
path-keyed partial-response caching with server-provided `cacheTags`; revisits
within `staleTime` skip the network. `Router.start({ preload: true })` also
loads the cache tier because hover preloads store speculative responses there.
After a mutation action, `Router.invalidateTags()` evicts all entries whose tags
overlap with the invalidated tags.

**Mutation actions:** Components can declare `static action(ctx: RouteActionContext)` as the write counterpart to `static loader()`. `Router.start({ actions: true })` opts into the action runtime; otherwise the router core does not import form interception code. When enabled, the router intercepts `<form method="post">` submissions, finds the nearest route component's `static action()`, calls it, and auto-invalidates the cache using both the action's returned tags and the route's build-time `invalidates` attribute. This ensures the compiler-declared invalidation graph is always respected — developers cannot forget.

**Pending UI:** Routes with a `pending` attribute show a loading component during slow navigations (>150ms). The pending component is a normal WebUI component — SSR'd and build-time validated. Keep-alive and cached routes skip pending (no delay to show).

**Error boundaries:** Routes with an `error` attribute show an error component when the navigation fetch fails. The error component receives `{ error, status, path }` as state and can call `Router.navigate()` to recover.

**Request headers sent by the client router:**

| Header | Value | Purpose |
|--------|-------|---------|
| `Accept` | `application/x-ndjson, application/json` | Requests NDJSON streaming or JSON partial instead of full HTML |
| `X-WebUI-Inventory` | Hex bitmask | Templates already loaded — server skips re-sending them |

The `chain` field is produced by `Protocol::render_partial()`, which walks the
fragment graph and matches routes at each nesting level using request-local
route state and the protocol's startup-built component index. All host
`renderPartial` surfaces return the complete JSON response.

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
    pub fn with_plugin(factory: fn() -> Box<dyn HandlerPlugin>) -> Self;

    pub fn render(
        &self,
        protocol: &Protocol,
        state: &Value,
        options: &RenderOptions<'_>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```

#### Runtime Protocol

`Protocol` is the one public runtime protocol type. It owns a decoded
`WebUIProtocol`, a deterministic component index, and a lazily populated
template-metadata cache. Construct it once when the server loads
`protocol.bin`, then share it across full renders, partial navigation,
component-template requests, and token queries.

The wire/build model and runtime model remain separate:

- `WebUIProtocol` is the mutable protobuf wire/build model. Builders populate
  it, `prost` serializes it, tests compare it, and callers may construct one
  directly before encoding `protocol.bin`.
- `Protocol` is the immutable runtime wrapper. Its component index,
  locks, and lazy JSON caches are process-local implementation details that must
  never be serialized into the protobuf or rebuilt for every request.

Putting runtime caches on `WebUIProtocol` would make the generated wire type
non-serializable and introduce locks into build-time mutation. Removing the
wrapper would force byte-oriented hosts to decode the protobuf and rebuild
indices on each request. `Protocol` therefore contains, rather than replaces,
`WebUIProtocol`.

```rust
pub struct Protocol {
    /* decoded protocol + component index + RwLock<template metadata> */
}

impl Protocol {
    pub fn from_protobuf(bytes: &[u8]) -> Result<Self, ProtocolError>;
    pub fn new(protocol: WebUIProtocol) -> Self;
    pub fn protocol(&self) -> &WebUIProtocol;
    pub fn tokens(&self) -> &[String];
    pub fn render_partial(
        &self,
        state_json: &str,
        entry_id: &str,
        request_path: &str,
        inventory_hex: &str,
    ) -> Result<String, HandlerError>;
    pub fn render_component_templates(
        &self,
        component_tags: &[&str],
        inventory_hex: &str,
    ) -> Result<Value, HandlerError>;
}
```

Full renders borrow the immutable protocol without locking. Every authored
route pattern is compiled when `Protocol` is loaded. Absolute routes match from
the request root; relative routes reuse a compiled suffix after the parent
route's consumed request segments, so parameter values never become cache
keys. Parsed template metadata uses a read-write lock limited to individual
cache lookups. `Protocol` is `Send + Sync`.

There are no public raw-`WebUIProtocol` rendering alternatives and no
`ProtocolIndex` lifecycle API. This prevents callers from accidentally
decoding or rebuilding the deterministic index per request. Request-specific
route-pattern caches do not exist: the immutable protocol-owned route index is
shared by full renders, partial navigation, and route-parameter extraction.

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
    if let Err(e) = handler.render(&proto, &state, &opts, &mut writer) {
        log::error!("render failed: {e}");
        let _ = ResponseWriter::write(&mut writer, "<!-- webui: render error -->");
        let _ = ResponseWriter::end(&mut writer);
    }
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
handler.render(&protocol, &state, &options, &mut writer)?;
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
    /// CSS custom property definitions from this component's CSS.
    pub css_definitions: Vec<String>,
    /// CSS `var()` fallback chains from this component's CSS.
    pub css_fallback_chains: Vec<CssFallbackChain>,
    /// Whether authored browser code owns this custom element. This can be true
    /// for external packages whose source is not available to the parser.
    pub is_client_owned: bool,
}
```

#### Registration Methods
```rust
pub fn register_component(&mut self, registration: ComponentRegistration<'_>) -> Result<(), ParserError>
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

/// A discovered component (tag name, HTML template, optional CSS, ownership).
pub struct DiscoveredComponent {
    pub tag_name: String,
    pub html_content: String,
    pub css_content: Option<String>,
    /// Whether authored browser code owns this custom element.
    pub is_client_owned: bool,
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
   - root JS entry (`exports["."]`, `main`, `module`, or `browser`) → authored component ownership
4. Parse the Custom Elements Manifest for `modules[].declarations[].tagName`
5. Return `DiscoveredComponent` structs with `is_client_owned` set from source metadata (callers handle registration)

Conditional exports are resolved with deterministic priority: `default` → `import` → `require`.

Script ownership is metadata-only: discovery never scans package JavaScript to find
`customElements.define()` calls. Packages without a root JS entry are treated as
compiler-owned template libraries. Packages with a root JS entry own their custom
elements and are never replaced by compiler-owned hosts. Package source is not
scanned by Rust. If the package is bundled with the application, the bundler
projection adapter analyzes its source and includes it in the application
manifest; external/separately built packages provide their own fragment.

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
Local paths use the same sibling-file rule as app components: `my-card.html`
paired with `my-card.ts` or `my-card.js` is authored/interactive; otherwise it is
SSR-only. Scriptless templates may contain server-rendered bindings,
conditionals, and repeats, but they emit no client metadata or state.
Local paths perform a recursive WalkDir scan for HTML files with hyphenated names, pairing matching CSS files — the same convention used by the parser's `ComponentRegistry`.

### HTML Parser
```rust
pub struct HtmlParser {
    component_registry: ComponentRegistry,
    css_parser: CssParser,
    condition_parser: ConditionParser,
    handlebars_parser: HandlebarsParser,
    css_strategy: CssStrategy,
    legal_comments: LegalComments,
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

- **Link** (default): Emits `<link>` tags referencing external `.css` files only for components whose discovery/registration data included CSS. Used by the CLI for production builds where CSS files are served separately. Output filenames are configurable with a naming template (`[name]`, `[hash]`, `[ext]`), defaulting to `[name].[ext]`. `[hash]` is SHA-256 truncated to 8 hex chars. An optional public base prefix can be applied so protocol `css_href` values point to CDN URLs. The resolved href is used consistently for handler-emitted head links and parser/plugin-generated component template stylesheet links. Handler-emitted `<head>` links are ordered by **document/traversal order** (the order components are first discovered while walking the fragment graph), not alphabetically by tag name. This keeps the Light-DOM cascade aligned with source order (stable across component renames) and prioritizes Shadow-DOM `<link rel="preload">` hints by appearance. The order is deterministic because the graph walk is deterministic.
- **Style**: Embeds the full CSS content in `<style>` tags inside the shadow DOM template. Used when all files are needed in-memory.
- **Module**: Registers each component's CSS as a CSS Module via an [Import Map](https://html.spec.whatwg.org/multipage/webappapis.html#import-maps) entry whose value is a `data:text/css,...` URI. During SSR, the handler emits a `<script type="importmap">{"imports":{"component-name":"data:text/css,..."}}</script>` in each component's light DOM on first render (e.g., `<my-comp><script type="importmap">...</script><template ...>`) and adds `shadowrootadoptedstylesheets="component-name"` to each shadow root `<template>`. When the developer supplies their own `<template>` wrapper (e.g., to attach `@event` handlers), the parser preserves the wrapper attributes and appends `shadowrootadoptedstylesheets="component-name"` when it is missing. Multi-specifier values already authored by the developer (`shadowrootadoptedstylesheets="component-name other-sheet"`) are honored verbatim. Components inside false `<if>` blocks or empty `<for>` loops that were not rendered during SSR get their importmap definitions emitted at `body_end`, so client-side activation can adopt them. CSS bytes are percent-encoded as needed to survive the `data:` URI parser (`%`, `#`, `"`, whitespace, and non-ASCII / control bytes); the importmap JSON object is built via `serde_json` so the specifier and URI value are correctly JSON-escaped. **Requires browser support for [Multiple Import Maps](https://github.com/WICG/import-maps/blob/main/proposals/multiple-import-maps.md) (Chrome 133+)** so each component's importmap can be emitted independently and merged into the document-level resolution table by the browser. When a CSP nonce is configured (via `RenderOptions::with_nonce` / `webui_handler_set_nonce`), the SSR-emitted `<script type="importmap">` tags include `nonce="VALUE"` (in `type`, `nonce` order) so strict `script-src 'nonce-...'` policies allow them, matching the existing nonce treatment of inline `<script>` tags. The browser registers the CSS module globally and shares a single `CSSStyleSheet` across all shadow roots that adopt it. No external CSS files are produced. During SPA partial navigation, definitions for newly needed components are sent in the `templateStyles` array as `<script type="importmap">{"imports":{...}}</script>` strings (without a `nonce` attribute - the router materializes each tag client-side and applies the per-request nonce when appending to `<head>` before installing component template closure arrays). WebUI Framework compiled metadata carries the adopted stylesheet specifier (`sa`) so client-created components can adopt the registered stylesheet on their shadow root.

Set at construction time with
`HtmlParser::with_options(ParserOptions::try_new(css, dom, css_file_name_template, css_public_base, legal_comments))`.

#### Legal Comments
```rust
/// Strategy for preserving legal comments in generated output.
pub enum LegalComments {
    /// Strip every HTML and CSS comment.
    None,
    /// Preserve legal CSS comments inline and strip all other comments.
    Inline,
}
```

The default is `LegalComments::Inline`, which preserves CSS comments that match
esbuild's legal-comment convention: comments containing `@license` or
`@preserve`, or comments starting with `/*!` or `//!`. WebUI supports only
`none` and `inline` modes. HTML comments are always stripped, and bindings or
directives inside HTML comments never produce fragments or plugin metadata.
CSS comments are stripped from external component CSS, inline `<style>` content,
component template CSS, and plugin-captured component templates unless they are
legal comments and `inline` preservation is active.

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
    fn into_artifacts(self: Box<Self>) -> Result<ParserPluginArtifacts>;
}
```

**Hook invocation points:**
- **Fragment start**: `start_fragment` runs before each `HtmlParser::parse(...)` call so plugins can reset fragment-local counters
- **Attribute loop**: `classify_attribute` decides whether framework-owned attrs are kept, skipped, or skipped-and-counted as bindings
- **Element completion**: `finish_element` runs with the final binding count after all attrs are processed; returned bytes are emitted as a `Plugin` fragment
- **Component registration**: `register_component_template` receives the plugin-facing component template HTML after HTML/CSS comment stripping. Authored root `<template>` attributes are preserved for plugins; the SSR/internal parse view may strip runtime-only attributes so rendered HTML stays clean. The component's client-ownership marker distinguishes authored from scriptless templates; Rust does not inspect JavaScript/TypeScript semantics.
- **Artifact extraction**: `into_artifacts` returns post-parse outputs such as client component templates without `Any` downcasts. It is **fallible**: template-authoring mistakes found while compiling component templates (an invalid `@event` handler or a non-braced `w-ref`) surface as `ParserError::Template` instead of panicking, so every host (CLI, Node, FFI, WASM) can handle them.

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
webui build ./templates --out ./dist --asset-file-name-template="[name]-[hash].[ext]" --css-public-base="https://cdn.example.com/assets"
webui build ./templates --out ./dist --plugin=webui --emit-component-assets mail-thread,compose-page
webui build ./templates --out ./dist --plugin=webui --emit-component-assets mail-thread --asset-file-name-template="[name]-[hash].[ext]"
webui serve ./templates --state ./data/state.json --plugin=<name>
webui serve ./templates --state ./data/state.json --plugin=webui --emit-component-assets mail-thread,compose-page --watch
```

`webui serve` performs a preflight bind check on its configured HTTP port and
fails before the initial build if that port is already in use, returning an
actionable message so stale dev processes can be stopped explicitly.

With `webui serve --api-port`, route state requests and `/api/*` forwarding
preserve the incoming URI's encoded path and query exactly except for the entry
route alias. The development server does not decode or re-encode percent escapes
before sending non-entry request paths to the backend. `/` and `/index.html`
both resolve backend state at `/` (the entry path is normalized), while still
preserving the query string. All other request paths forward their encoded path
and query unchanged. Encoded slashes such as `%2F` therefore remain inside one
route segment, and encoded spaces, percent signs, and UTF-8 bytes reach
development backends with the same representation used by production clients.

After generated assets and `--servedir` files miss, `webui serve` uses request
intent rather than path punctuation to decide whether to run the SPA route
fallback. Fallback runs only when `Accept` explicitly includes `text/html` or
`application/xhtml+xml` for document navigation, or `application/json` for a
WebUI JSON partial render. `q=0` disables that media type, while a malformed or
out-of-range `q` value falls back to `q=1.0`; when HTML and JSON are both
acceptable, the higher `q` wins and exact ties prefer JSON. Missing or
wildcard-only `Accept` headers return 404, as do JS, CSS, image, and other
non-HTML/non-JSON asset requests.
Literal dots in route segments, such as `/docs/v2.1`, are valid and do not block
route matching or fallback.

In `webui serve --watch`, the file watcher is **content-aware**: it hashes each
changed file and drops events whose bytes are unchanged, so a no-op save
(repeated Ctrl+S that rewrites identical content) triggers no rebuild in the
clean state. While a rebuild error is active, unchanged events are forwarded so a
no-op save can retry transient failures without forcing a real content edit.
Deletions and oversized files always count as changed. Each rebuild's terminal
line names the triggering file (`↻ rebuilt app-shell.css …`, or `… (+N more)`).
Incremental rebuild failures are retained in dev-server state. The rebuild
worker reports the error to the terminal and live-reload SSE; subsequent browser
refreshes, route renders, JSON partial requests, and component template requests
return the latest rebuild error instead of stale output. HTML error pages keep
the live-reload client connected, and JSON partial requests return the rebuild
error before resolving file/API state. A successful rebuild clears the stored
error and updates the served protocol/HTML. Non-fatal build advisories (e.g. a
literal-fallback CSS token absent from every theme) are warning-severity
`Diagnostic`s rendered with the same `--> file:line:column` + snippet + `help:`
layout as errors, framed with surrounding blank lines so consecutive
errors/warnings stay readable. They print under the rebuild line but are
**deduplicated**: a warning is printed when it first appears (or reappears after
being resolved), not on every rebuild, so editing an unrelated file does not
re-spam unchanged advisories. Errors are not deduplicated — a broken build is
surfaced on every rebuild attempt.

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
- Strip HTML comment nodes before output; comment contents are never parsed for signals, directives, attributes, or plugin metadata

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
pub struct CssParser;

impl CssParser {
    pub fn extract_tokens(&mut self, css_content: &str) -> Result<HashSet<String>, ParserError>
    pub fn extract_definitions(&mut self, css_content: &str) -> Result<HashSet<String>, ParserError>
    pub fn extract_tokens_and_definitions(&mut self, css_content: &str) -> Result<(HashSet<String>, HashSet<String>), ParserError>
}
```

#### Requirements
- Process CSS variables
- Extract dynamic variables with --webui- prefix
- Convert dynamic variables to signals
- Handle nested variable references
- Process inline and external CSS
- Strip CSS comments during parsing, preserving only legal comments when `LegalComments::Inline` is active
- Reject malformed CSS at build time with `ParserError::Css`, including
  unterminated `var()` calls, block comments, strings, and unmatched braces,
  parentheses, or brackets.
- Exclude any token that is defined by local CSS before validating theme
  coverage. For example, `--foo: var(--token-a, var(--token-b))` reports
  `token-b` only when `--token-a` is defined in the same CSS input.

### HTML Scanner

The parser uses a deterministic, quote-aware HTML scanner (`html_parser`) rather
than a browser-complete DOM parser. It intentionally exposes a zero-copy,
DOM-like event API (`Walker`, `Event`, `Element`, `Attr`) over borrowed source
ranges. The semantic parser may use the same primitives directly in hot paths to
avoid iterator overhead.

Semantic template traversal is stack-driven rather than recursive. Entering a
child range pushes an explicit parse operation, and directive bodies (`<for>`,
`<if>`) swap to isolated fragment contexts until their body parse completes.

#### Validation semantics

- HTML comments are build-time-only and stripped.
- Declarations/DOCTYPE are preserved as raw content.
- Unterminated opening tags, missing closing tags, overlapping/misnested tags,
  unexpected closing tags, and unterminated comments or declarations are rejected
  at build time with `ParserError::Html`.
- Recursive component template references are rejected at build time with an
  actionable directive error instead of recursing through parser calls.
- The scanner is quote-aware for opening tags, so `>` inside `'...'` or `"..."`
  attribute values never terminates a tag.
- HTML tag names are matched ASCII-case-insensitively where the HTML
  specification requires it: void elements (`<BR>`), closing-tag matching, and
  `<style>`/`<STYLE>` are recognized regardless of case. WebUI directives
  (`<for>`, `<if>`, `<route>`, `<outlet>`) and component names remain
  case-sensitive.
- This is not a browser HTML parser. It supports the WebUI template subset used
  at build time and should not be used for arbitrary browser DOM conformance.

#### Guardrails

- A single template is capped at 16 MiB. Larger generated templates must be
  split into components before build.
- Semantic template nesting and nested `<route>` trees are each capped at 512
  levels to prevent stack exhaustion on pathological input.
- Core scanner loops are iterative and do not use regular expressions.

### CSS Token Hoisting

CSS Token Hoisting extracts the set of CSS custom properties (tokens) that are **used** across all components and entry page styles at build time. The sorted, deduplicated list is included in the protocol's `tokens` field, enabling host runtimes to resolve only the design tokens the application can still need after local CSS definitions are considered.

#### Token Extraction (`CssParser::extract_tokens`)

The `extract_tokens` method uses a deterministic CSS scanner to extract custom property **usages** from `var()` calls, while **excluding** locally-defined custom properties.

**Extracted (hoisted):**
- `var(--colorPrimary)` → token `"colorPrimary"`
- `var(--a, var(--b, var(--c)))` → tokens `"a"`, `"b"`, `"c"` (nested fallbacks)
- `var(--size, 16px)` → token `"size"` (literal fallbacks ignored)

**Excluded (not hoisted):**
- `--bar: 12px` — local custom property definitions
- `var(--bar)` when `--bar` is defined in the same CSS file or by an ancestor
  component/root CSS scope

The scanner tracks nested `var()` fallback expressions, so nested fallbacks are naturally handled.

#### Token Collection During Parsing

The `HtmlParser` records CSS fallback-chain requirements and custom-property
definitions from two sources:

1. **Component CSS** — component registration stores each component's
   pre-extracted `css_fallback_chains` and `css_definitions`.
2. **Inline `<style>` tags** — when the parser processes a `<style>` tag, it extracts token usages and definitions while stripping removable CSS comments in the same scanner pass.

After parsing completes, `HtmlParser::token_analysis()` walks the parsed fragment
graph iteratively from each entry/root fragment and returns `CssTokenAnalysis {
protocol_tokens, fallback_chains }`. The walk carries a counted set of CSS
custom-property definitions from the entry/root through component boundaries,
because CSS custom properties inherit through Shadow DOM. Each token candidate
in a fallback chain such as `var(--a, var(--b, var(--c)))` is removed when that
token is defined by the current or ancestor CSS scope; any remaining candidates
contribute to the sorted protocol token list.

#### Comment Handling

HTML comments are stripped from parser output. Comment contents are never parsed
for signals, directives, attributes, or plugin metadata. CSS comments are
stripped from inline `<style>` elements and component CSS, except for two cases:
legal comments when `LegalComments::Inline` is active, and CSS signal fragments
in inline `<style>` elements. A CSS signal fragment is a block comment whose
trimmed body is exactly one handlebars expression:

```css
/*{{tokens}}*/        → Signal { value: "tokens", raw: false }
/*{{{tokens.light}}}*/ → Signal { value: "tokens.light", raw: true }
```

Bare handlebars expressions in CSS are raw text. Dynamic CSS fragments must use
the comment wrapper so the CSS parser can distinguish them from invalid CSS.

### Design Token Resolution (`webui-tokens`)

The `webui-tokens` crate provides build/serve-time validation and resolution of design token values. While the parser extracts token **names** and `var()` fallback chains, the token crate owns the theme-coverage policy: `validate_chain_tokens` decides which chain candidates a theme must provide (literal-fallback chains are exempt) and `unthemed_literal_fallback_tokens` reports likely typos, while `resolve_tokens` generates CSS declarations for injection into state. The parser only adapts the resulting [`webui_tokens::TokenError`] into a structured `Diagnostic`.

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
CssTokenAnalysis::validate_theme_tokens(token_file) → Result<()>
    ↓
resolve_tokens(protocol.tokens, token_file) → ResolvedTokens { css }
    ↓
inject_token_css(state, css) → state["tokens"]["light"] = "..."
```

1. **Validate**: Every *required* token must exist in every theme. A token is required when it appears in at least one unresolved `var()` chain with no literal CSS fallback. Local and ancestor CSS definitions are removed before validation, so `--token-a: red; --foo: var(--token-a, var(--token-b))` requires `token-b` from the theme but not `token-a`. A literal-terminated chain such as `var(--brand, #000)` is exempt — `--brand` stays in `protocol.tokens` for runtime resolution (the theme value still wins when present) but does not fail the build when the theme omits it. The same token referenced once with a bare `var(--brand)` and once as `var(--brand, #000)` is still required (the bare usage has no fallback). Missing required tokens fail with `missing-theme-token`. Theme token values are trusted: unresolved or cyclic `var(--x)` references inside the theme remain browser CSS semantics rather than build failures.
2. **Dependency closure**: Token values referencing other tokens via `var(--x)` trigger transitive inclusion when the referenced token is present in the same theme. Missing transitive references are left in the CSS value as authored.
3. **CSS generation**: Sorted `--name: value;` declarations. Output is deterministic.
4. **State injection**: Per-theme CSS strings are set on `state.tokens.<theme>`, where `/*{{{tokens.<theme>}}}*/` signals resolve them during rendering. These render-only token strings are omitted from the emitted `webui-data` client bootstrap.

A token used **only** with a literal `var()` fallback and defined in no theme (e.g. a misspelled `var(--colr-brand, #000)`) is reported as a non-fatal `unthemed-token` warning in `BuildResult::warnings` (a `Vec<Diagnostic>`) rather than failing the build. These are warning-severity `Diagnostic`s carrying location, snippet, and a `did you mean …?` suggestion, so `webui build` and `webui serve` render them with the same layout as errors; Node receives their plain `Display` text.

#### Package Resolution (`resolve_theme_path`)

The CLI `--theme` flag accepts a file path or an npm package name:

```bash
webui build ./src --out ./dist --theme=@microsoft/webui-examples-theme
webui serve ./src --theme=@microsoft/webui-examples-theme
webui serve ./src --theme=./my-theme.json
```

Package names are resolved by walking up from `search_root` looking for `node_modules/<pkg>/tokens.json`. Scoped packages (`@scope/name`) and explicit subpaths (`@scope/name/custom.json`) are supported.

`BuildOptions::theme` accepts a loaded `TokenFile`. When present, `webui::build`
validates parser-discovered unresolved tokens before protocol serialization and
returns `WebUIError::Parse { source: ParserError::Template(..) }` when required
tokens are missing from the theme. CLI `webui build --theme`, `webui serve
--theme`, and Node `build({ theme })` all use this same build validation path.

When `webui serve --watch` hits one of these theme-token validation failures
during an incremental rebuild, the failure is retained as the current dev-server
state so refreshes keep showing the diagnostic until the next successful rebuild.

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

Each component's compiled template metadata is emitted as JSON-safe data in
`template_json` and registered in `window.__webui.templates[tagName]`. Condition
expressions are the only executable part: the parser emits them into
`template_functions` as a component-local JavaScript closure array, and metadata
condition references link to that array by index. The framework normalizes
`[functionIndex, paths]` into `[fn, paths]` once before hydration, so reactive
update hot paths still call the function directly.

| Field | Type                              | Description                                        |
|-------|-----------------------------------|----------------------------------------------------|
| `h`   | `string`                          | Marker-free static HTML for client-created DOM, including baked-in `<link>` / `<style>` nodes for link/style CSS strategies |
| `tx`  | `[slot, parts][]`                 | Client text runs inserted at precompiled slots     |
| `a`   | `CompiledAttrMeta[]`              | Attribute binding metadata                         |
| `ag`  | `[elementPath, start, count][]`   | Attribute-target groups for `a[]`                  |
| `c`   | `[ConditionRef, blockIndex, slot][]` | Conditional blocks                              |
| `r`   | `[collection, itemVar, blockIndex, slot, keyPath?][]` | Repeat blocks; `keyPath` is relative to the item variable |
| `eg`  | `[event, [[handler, argSpecs, targetPath, usesEvent?]]][]` | Body events grouped by event name |
| `b`   | `TemplateBlockMeta[]`             | Nested compiled block table referenced by `c` / `r` |
| `sa`  | `string`                          | Optional module-mode adopted stylesheet specifier copied from `shadowrootadoptedstylesheets` |
| `re`  | `[event, handler, argSpecs][]`    | Root events, attached to the host element          |
| `tr`  | `string[]`                        | Component-level state roots referenced by the template, excluding repeat item variables |
| `ta`  | `string[]`                        | Observed host attributes index-aligned with `tr` |
| `sd`  | `1`                               | Shadow DOM flag for client-created components      |
| `th`  | `1`                               | Compiler-owned host flag for a scriptless template |

All arrays are optional and omitted from the output when empty to minimize payload.

`ConditionRef` in JSON metadata is `[functionIndex, paths]`:

- `functionIndex` indexes the component-local `window.__webui.templateFns[tagName]` array.
- `paths` lists every state path referenced by the condition so the framework can build its targeted reactive path index without inspecting function source.

The closure itself has the shape `(resolve, scope) => boolean`; generated source calls
`resolve(path, scope)` for identifier lookups and preserves the existing WebUI condition
semantics for truthiness, comparison, negation, and `&&` / `||` compounds.
- `5` = `GREATER_THAN_OR_EQUAL`
- `6` = `LESS_THAN_OR_EQUAL`

Logical operators also match the protocol enum values:

- `1` = `AND`
- `2` = `OR`

`tr` and `ta` are emitted by the compiler so the browser runtime does not walk
every binding at startup to rediscover roots or observed attributes. `ta` is
index-aligned with `tr`: `ta[i]` is the host attribute observed for template
root `tr[i]`. Metadata generated by `--plugin=webui` must include these fields
when applicable; runtimes treat missing fields as empty. Authored components
emit normal metadata. Scriptless components retain the same compiled template
metadata with `th: 1`, allowing the framework to register a compiler-owned
dormant host without requiring an empty authored module.

#### Hydration keys and state projection

Deriving a component's initial hydration surface is a build-time decision, and
Rust never performs JavaScript/TypeScript analysis to make it. Default and
FAST builds always set `InitialStateStrategy::Full`. The WebUI plugin sets
`InitialStateStrategy::Components` only when one or more
`--projection-manifest` fragments were supplied, loaded, and merged
successfully (see "Bundler-Neutral State Projection Compiler" below); absent
any manifest, WebUI builds also use `InitialStateStrategy::Full`.

The WebUI parser plugin represents both initial hydration and partial
navigation with an explicit `StateSurface`:

- `None`: the complete surface is known to be empty.
- `Keys(Vec<String>)`: the complete surface is known and the sorted,
  deduplicated keys are authoritative.
- `All`: the complete surface cannot be proven, so correctness requires full
  state.

Client ownership and source availability are distinct. Scriptless components
use initial `None` and navigation template roots — this is Rust-derived and
does not depend on manifests. Authored (scripted) modules default to `All`
for both surfaces until a merged projection manifest proves an exact key set
for that component's tag; opaque external modules always use `All` for both
surfaces and cannot be replaced by compiler-owned hosts.

When a manifest covers a scripted component, its initial surface is exactly
`ComponentEntry.hydrationKeys`: proven `@observable + @attr` property names.
For `@attr`, an existing SSR host attribute wins; bootstrap state fills the
property only when that attribute was not materialized on the host.
The navigation surface is the union of
`ComponentEntry.navigationKeys` (`@observable + @attr`) and compiled template
roots (`tr`). A proven empty array is a valid `Keys([])`, distinct from
`None`. If a
scripted component has no manifest coverage, its initial and navigation
surfaces are both `All` — but this is a build error under strict coverage
whenever any manifest is supplied at all (`PROJ-B001`); `All` for an uncovered
scripted component is only reachable when no manifest was supplied for the
build at all.
`ComponentData::{hydration_mode,navigation_mode}` encode the surface and the
corresponding key vectors are populated only for `Keys`.
For protocol binaries created before the mode fields existed, a default
`None` mode paired with a non-empty legacy key vector is interpreted as
`Keys`; current builders never emit that combination.

At initial render, `InitialStateStrategy::Full` bypasses component-key
collection. `Components` finds components reachable from the active entry and
request route, then combines their hydration surfaces: any `All` selects full
state, `Keys` contributes keys, and `None` contributes nothing. Inactive sibling
routes are excluded. Components behind active-route conditionals, loops, and
attribute-template edges remain conservatively reachable because reachability
is state-agnostic. Unknown protocol enum values are treated as `All`.

Projection iterates whichever side is smaller:

- schema keys plus direct object lookup for sparse schemas over wide state maps
- state entries plus binary-search membership for compact state maps

The projected object is serialized into the existing buffered script-safe JSON
path. The handler does not stream serde tokens directly to `ResponseWriter`;
that alternative regressed the equal-byte 1 MB benchmark.

**Security boundary.** Hydration projection reduces payload and CPU cost. It is
not a secrecy boundary. Any state selected by exact metadata or preserved by a
full fallback is client-facing. Hosts must not place secrets in browser render
state.

**Partial state.** `Protocol::render_partial()` accepts raw JSON and applies the
active route's navigation surfaces with the same `None` / `Keys` / `All`
rules. On `Keys`, a streaming JSON visitor validates the complete object,
skips unselected values without materializing them, and borrows selected raw
values into the response. On `All`, raw APIs validate and preserve the borrowed
JSON object without materializing it. Scriptless routes therefore receive only
template roots needed for the destination; uncertain routes receive complete
state.

`a[]` uses compact tuple forms to avoid runtime parsing:

- `[name, 0, path]` — simple attribute binding, e.g. `href="{{url}}"`
- `[name, 1, path]` — pass-through/property binding, e.g. `:config="{{settings}}"` or `:value="{{searchQuery}}"`
- `[name, 2, ConditionExpr]` — boolean attribute binding, e.g. `?disabled="{{expr}}"`
- `[name, 3, parts]` — mixed/template attribute binding, e.g. `class="item {{state}}"`

`argSpecs` for event handlers are resolved at dispatch time against the captured scope chain for the rendered template block:

- `["e"]` passes the DOM event object
- `["p", path]` resolves a component or active `<for>` scope path, e.g. `item.id`
- `["s", value]`, `["n", value]`, `["b", 0|1]`, and `["z"]` pass string, number, boolean, and `null` literals

For example, `@click="{selectItem(item.id)}"` calls `selectItem` with the current repeat item id, while `@click="{selectItem(item.id, e)}"` passes the item id followed by the event object. `@click="{selectItem(e)}"` keeps the existing event-passing behavior, and `@click="{selectItem()}"` calls the handler with no arguments.

### Compilation rules

The Rust compiler (`generate_compiled_template` in `webui-parser/src/plugin/webui.rs`) transforms the HTML template in a single forward pass, then finalizes it into marker-free client HTML plus locator metadata:

| Source syntax                        | Metadata field(s)      | Client `h` result                 |
|--------------------------------------|------------------------|-----------------------------------|
| `{{expr}}`, `{{{expr}}}`, mixed text | `tx[]`                 | dynamic text run removed          |
| `href="{{url}}"`                     | `a[]` + `ag[]`         | element kept marker-free          |
| `class="item {{state}}"`             | `a[]` + `ag[]`         | element kept marker-free          |
| `?disabled="{{expr}}"`               | `a[]` + `ag[]`         | element kept marker-free          |
| `:config="{{settings}}"`, `:value="{{searchQuery}}"` | `a[]` + `ag[]` | element kept marker-free |
| `<if condition="expr">body</if>`     | `c[]` + `b[]`          | block removed; anchor slot stored |
| `<for each="v in coll">body</for>`   | `r[]` + `b[]`          | block removed; anchor slot stored |
| `<for each="v in coll"><x key="{{v.id}}">body</x></for>` | `r[]` + `b[]` | block removed; first-child key path stored |
| `@event="{handler(item.id, e)}"`     | `eg[]`                 | element kept marker-free          |
| `@event` on `<template>` wrapper     | `re[N]`                | *(stripped)*                      |
| `w-ref="{name}"`                     | *(stays)*              | *(unchanged)*                     |
| `<outlet />`                         | *(stays)*              | `<outlet></outlet>`               |

Repeat identity is positional by default. At runtime, the existing block at
index `i` receives the current collection item at index `i`; only tail growth
or shrinkage creates or removes blocks. The runtime never infers identity from
repeated-root attributes, so duplicate values and attributes are safe and
attribute order has no reconciliation semantics.

Authors may opt into logical identity by adding `key="{{item.id}}"` to the first
child inside `<for>`. Primitive arrays use `key="{{item}}"`. Under the WebUI
plugin, `key` is compiler-only structural metadata: the compiler validates this
restricted item-rooted path grammar, removes the attribute from SSR and client
HTML, and emits only the relative path as the optional fifth `r[]` tuple field
(`""` for the item itself). It is not included in `a[]` or `ag[]`. Unkeyed
repeats retain the four-field tuple. `data-key` remains an ordinary application
attribute and has no identity semantics. Only `key` on the first repeated child
is consumed as identity metadata. A `key` on another regular element produces
an `invalid-for-key` build diagnostic; directive attributes remain governed by
their own contracts. Keyed repeats accept unique
strings and finite numbers, preserve number/string type identity, and use a
stable-order positional fast path. A changed order uses a reusable key map to
move existing block instances, preserving browser-owned and local component
state with the logical item. Invalid or duplicate runtime key values clear
established identity, warn once, and reconcile positionally for that update; a
later valid update establishes identity again. Validation completes before DOM,
scope, or instance mutation.

SSR repeat markers do not serialize separate key values. When the bootstrap
collection is present and its length matches the hydrated SSR instance count,
the runtime derives typed keys by index from that collection and establishes
identity immediately, so the first later reorder can move existing SSR blocks.
Missing state, a count mismatch, or invalid keys leave identity unestablished
and the next valid update reconciles positionally once. This relies on the
existing hydration invariant that SSR HTML and bootstrap state represent the
same render. FAST v2/v3 do not reserve `key` and do not emit WebUI key metadata
into `<f-repeat>` markup.

**Authoring validation.** Build-time authoring mistakes are returned as a structured `ParserError::Template(Box<Diagnostic>)`, never panicked. This covers invalid `@event` handlers (e.g. `@click="e.preventDefault()"`, or a bare `@click="{closeMenu}"`), scriptless components that contain `@event` bindings, non-braced `w-ref` (`w-ref="name"` instead of `w-ref="{name}"`), core-parser mistakes — an invalid `<for each>` expression, a malformed or misplaced first-child repeat key (`invalid-for-key`), a missing/invalid `<if condition>`, an unknown component tag, a recursive template reference — malformed CSS in a `<style>` block, and structural HTML well-formedness errors (unclosed/malformed tags, unterminated comments/declarations, unexpected closing tags, excessive nesting), so every build error renders identically. The `Diagnostic` is plain, actionable data — a **stable machine-readable `code`** (e.g. `invalid-for-each` or `scriptless-event-handler`; see `diagnostic::codes`), title, source location (rendered rustc-style as `--> owner:line:column` when the offending byte offset is known, otherwise `in component <c> · element <e>`), offending snippet, and a `help:` fix — and carries **no color**: `webui-cli` styles it with `console`, while Node/FFI/WASM forward the plain `Display` text through their native error channel. Where a fix is likely a typo, the `help:` offers a **"did you mean …?" suggestion** via an iterative Levenshtein match (`suggest::closest_match`): a misspelled directive attribute (`eahc` → `each`), or an unregistered custom-element tag that closely matches a registered component **in the same namespace** (`<mp-buton>` → `<mp-button>`; cross-namespace tags like `<md-button>` still pass through as genuine custom elements).

---

## Bundler-Neutral State Projection Compiler

This section is the authoritative specification for the projection compiler,
manifest schema, adapter SPI, Rust consumer contract, diagnostic codes, and
conformance fixtures. It is precise enough for the TypeScript compiler/esbuild
adapter and the Rust manifest consumer to be implemented concurrently by
independent agents without semantic drift.

### Canonical build order

State projection requires exactly one bundler run followed by one WebUI build
invocation. There is no second bundler pass and no in-process analysis.

```text
Step 1 — Bundler (esbuild or compatible adapter)
  Input:  application entry points, TypeScript/JavaScript sources
  Output: dist/index.js (+ split chunks)
          dist/webui-projection.json   ← projection manifest

Step 2 — WebUI build
  webui build ./src \
    --plugin webui \
    --projection-manifest ./dist/webui-projection.json \
    --out ./dist
  Input:  HTML/CSS templates, projection manifest(s)
  Output: dist/protocol.bin

Step 3 — Runtime handler
  Input:  protocol.bin, request state
  Output: HTML response
  Never reads: manifest, JavaScript, TypeScript, or bundler output
```

The manifest is a build-time handoff artifact. It is not deployed as a handler
runtime dependency.

An optional `buildWebUI()` convenience helper may run steps 1 and 2
sequentially, but it must be orchestration sugar over the same manifest contract
and must not create a second projection architecture.

### Package architecture

No new npm package is created. The existing `@microsoft/webui` package gains
one build-only subpath:

```typescript
import { compileProjection, esbuildProjection } from '@microsoft/webui/projection.js';
```

The root `@microsoft/webui` entry does **not** import or re-export the
projection subpath so that render/build consumers do not load compiler or
adapter code.

Internal source organization:

```text
packages/webui/src/projection/
  index.ts          — public subpath barrel
  compiler.ts       — TypeScript AST analysis and symbol graph
  graph.ts          — normalized module graph types and adapter SPI
  manifest.ts       — manifest schema types and serialization
  diagnostics.ts    — stable diagnostic codes and error types
  adapters/
    esbuild.ts      — esbuild adapter (first supported adapter)
    vite.ts         — future
    rollup.ts       — future
    rolldown.ts     — future
    webpack.ts      — future
    rspack.ts       — future
  fixtures/
    conformance.ts  — adapter conformance test helpers and reference cases
```

The canonical TypeScript type definitions live in `packages/webui/src/projection/`
and are the machine-readable specification that both the TypeScript compiler
implementation and the Rust manifest consumer must satisfy.

### Optional peer dependency policy

`typescript` and each officially supported bundler are optional peer
dependencies of `@microsoft/webui`. The first supported bundler is esbuild:

```json
{
  "peerDependencies": {
    "esbuild": "^0.28.1",
    "typescript": "^6.0.3"
  },
  "peerDependenciesMeta": {
    "esbuild": { "optional": true },
    "typescript": { "optional": true }
  }
}
```

**`esbuild` must not be a direct dependency and must not be bundled into
`@microsoft/webui`.** The application owns the esbuild installation and version.
Importing or invoking `@microsoft/webui/projection.js` without the required
peer produces an actionable diagnostic (`PROJ-P001`/`PROJ-P002`; see
[Diagnostic codes](#projection-diagnostic-codes)).

Both peers are optional so users importing only the root build/render API do
not receive dependency warnings for compiler tooling they do not use.

Bundler adapters use local structural interfaces and do **not** statically
import their bundler packages at module load time. Future supported adapters
add their bundlers as optional peers under the same policy.

### Normalized module graph and adapter SPI

The adapter SPI is defined in `packages/webui/src/projection/graph.ts`. It
isolates bundler-specific semantics behind a stable interface so the projection
compiler never depends on a particular bundler.

```typescript
export type ModuleKind = "file" | "virtual";

export interface ResolvedImport {
  /** Specifier exactly as authored. */
  readonly specifier: string;
  /** Exact bundler-resolved module ID; absent only for external edges. */
  readonly resolvedId: string | undefined;
  readonly external: boolean;
  readonly kind: "static" | "dynamic";
  /** Owning package identity, when the adapter can prove it. */
  readonly packageName?: string;
}

/** A single resolved module in the build graph. */
export interface ModuleNode {
  /** Canonical absolute path on disk, or a virtual ID beginning with NUL. */
  readonly id: string;
  readonly kind: ModuleKind;
  /** Owning package identity, when proven by the adapter. */
  readonly packageName?: string;
  /** Raw UTF-8 source text. Required for file modules. */
  readonly source: string | undefined;
  /** Authored specifiers paired with exact bundler-resolved targets. */
  readonly imports: ReadonlyArray<ResolvedImport>;
}

/** The resolved input module graph as seen by the bundler. */
export interface ModuleGraph {
  /** All modules reachable from the entry set, keyed by canonical ID. */
  readonly modules: ReadonlyMap<string, ModuleNode>;
  /** Canonical entry module IDs. */
  readonly entries: ReadonlyArray<string>;
}

/** Maps each emitted output path to the input module IDs that contribute to it. */
export interface OutputMembership {
  /**
   * Key: canonical absolute output ID, or a virtual ID beginning with NUL.
   * Value: set of canonical input module IDs whose code appears in this output.
   */
  readonly outputs: ReadonlyMap<string, ReadonlySet<string>>;
}

/** Context passed from the bundler adapter to the projection compiler. */
export interface AdapterContext {
  readonly graph: ModuleGraph;
  readonly membership: OutputMembership;
  /** Absolute build root containing every physical input/output/manifest. */
  readonly rootDir: string;
  /**
   * Manifest output path as an absolute disk path.
   * The serialized `root` field is relative from this path's directory to
   * `rootDir`; all physical manifest keys are root-relative.
   */
  readonly manifestPath: string;
  /** Bundler name, e.g. "esbuild". */
  readonly bundlerName: string;
  /** Bundler version string, e.g. "0.28.1". */
  readonly bundlerVersion: string;
  /** Exact bytes for every physical emitted output, keyed by output ID. */
  readonly outputContents: ReadonlyMap<string, string | Uint8Array>;
}
```

**Canonicalization rules:**

- `ModuleNode.id`, physical output IDs, `rootDir`, and `manifestPath` are
  canonical absolute paths. Virtual IDs begin with `\0`.
- Every authored import/re-export retains its authored `specifier` **and** the
  adapter-resolved `resolvedId`. The compiler never joins paths, substitutes
  extensions, reads package exports, or performs filesystem resolution.
- `packageName` carries semantic package identity independently of path layout.
  A specifier is treated as WebUI framework semantics only when the adapter
  proves `packageName: "@microsoft/webui-framework"`. Literal source text does
  not override adapter resolution.
- Every physical module has raw source and every physical output has exact
  bytes. Disk outputs can never be represented as `"virtual"` to skip stale
  validation.
- `rootDir` contains the manifest and every physical input/output. The compiler
  rejects graph members outside it.

The compiler parses source lazily. It seeds modules containing a supported
literal `.define(...)`/`customElements.define(...)` candidate (or a framework
edge needed to diagnose a dynamic tag), then loads imported/re-exported/base
modules only when symbol resolution follows them. Behavior-only dependency
graphs such as CodeMirror are hashed for staleness and membership but are not
parsed as projection semantics.

**Adapter responsibilities:**

1. Enable metafile/stats collection in the underlying bundler.
2. Populate `ModuleGraph`, retaining specifier-to-resolved-target edges.
3. Populate `OutputMembership` from the bundler's emitted output→input map.
4. Provide raw source text and exact emitted output bytes.
5. Invoke `compileProjection(ctx)` (see below).
6. Write the returned `ProjectionManifest` atomically to `manifestPath`.

**Compiler responsibilities (not adapter responsibilities):**

- TypeScript AST parsing.
- Symbol resolution.
- Decorator key extraction.
- `define()` association.
- Manifest serialization.
- Diagnostic reporting.

### TypeScript AST semantic rules

The compiler in `compiler.ts` processes all source files in the `ModuleGraph`
whose extension matches `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`,
or `.cjs`.

#### Symbol graph construction

The compiler builds a per-file symbol map from adapter-resolved edges rather
than re-running module resolution. For each `ModuleNode`:

1. Parse with the TypeScript compiler API (`ts.createSourceFile`) with
   `ScriptTarget.Latest` and `ScriptKind` inferred from the extension.
2. For each top-level import declaration, use the adapter's resolved graph to
   map the module specifier to a `ModuleNode.id` instead of resolving the path.
3. Record bound names: default import, named imports, namespace imports,
   re-exports. Build a `SymbolRef` for each: `{ moduleId, localName }`.
4. For each export: record the exported name and the `SymbolRef` it resolves to.

A `SymbolRef` tracks the ultimate origin of a binding through any number of
re-export hops. Resolution is iterative (no recursion); cycles in the import
graph are detected and produce diagnostic `PROJ-C012`.

#### Class analysis

For each class declaration or expression that is exported or associated with a
`define()` call:

1. Collect all own property declarations that carry a decorator.
2. For each decorator:
   - Resolve the decorator identifier through the symbol graph back to its
     export in the defining module.
   - If the resolved export is `observable` from `@microsoft/webui-framework`,
     record the property name in both `hydrationKeys` and `navigationKeys`.
   - If the resolved export is `attr` from `@microsoft/webui-framework`,
     record the **JavaScript property name** in both surfaces.
     `@attr({ attribute: "display-value" }) displayValue` therefore emits
     `displayValue`: framework state addresses the property registry, while an
     existing `display-value` host attribute takes precedence during SSR
     hydration.
3. Walk the class `extends` clause. Resolve the base class through the symbol
   graph. Collect the base class's keys recursively (iterative: push unresolved
   bases onto a stack). Stop at `WebUIElement` or any class from
   `@microsoft/webui-framework` whose own keys are already fully resolved.
4. Each final surface is own keys ∪ inherited keys (sorted, deduplicated,
   case-sensitive), and navigation is validated as a hydration superset.

**Supported import forms.** The compiler resolves keys through all of the
following:

| Source form | Resolution |
|---|---|
| `import { observable, attr } from '...'` | Direct named import |
| `import { observable as obs } from '...'` | Aliased named import |
| `import * as webui from '...'` | Namespace; `webui.observable` resolved |
| `export { observable } from '...'` | Re-export chain |
| `export { observable as obs } from '...'` | Aliased re-export |
| `export * from '...'` | Star re-export (all public names forwarded) |
| `export * as ns from '...'` | Namespaced star re-export |

Decorators proven to be non-framework local/imported symbols are irrelevant to
projection and are ignored. A decorator on a proven WebUI class whose identity
cannot be resolved through the adapter graph could be an aliased WebUI
decorator; that uncertainty produces `PROJ-C004` (hard diagnostic).

#### `define()` association

The compiler recognizes exactly two `define()` forms for associating a class
with a custom-element tag name:

```typescript
// Form 1: static class method
ContactCard.define("contact-card");

// Form 2: customElements.define
customElements.define("contact-card", ContactCard);
```

Rules:

- A call is considered a WebUI definition only after its class is proven to
  inherit from the adapter-identified `WebUIElement`, or it uses a proven WebUI
  decorator and inheritance proof fails. This prevents unrelated libraries
  with a `.define()` API from producing false diagnostics.
- A locally shadowed `customElements` binding is not the browser registry and
  is ignored.
- The tag-name argument must be a **string literal** at analysis time. Dynamic
  tags on a proven WebUI class produce `PROJ-C008`.
- An unrelated or unresolvable `.define()` receiver is ignored by the compiler.
  If it was actually a scripted WebUI component, Rust strict coverage later
  fails with `PROJ-B001`; the compiler never guesses based on capitalization or
  method names.
- The same tag defined more than once (within one adapter context) produces
  `PROJ-C010` (duplicate tag, hard diagnostic).
- A class associated with no `define()` call is not included in the manifest.
  It may be a base class or utility class; the compiler does not warn.

#### Exact-only semantics

The compiler operates in exact-only mode. There are no best-effort or partial
outputs:

- `hydrationKeys: []` and `navigationKeys: []` are valid proven-empty results.
  Both contain inherited/local `@observable + @attr`; `navigationKeys` remains
  a validated hydration superset for future channel-specific extensions.
- Any condition that prevents proving the exact key set is a **hard diagnostic**
  that fails the build. There is no fallback `All` entry in the manifest.

Conditions that produce hard diagnostics (cannot prove exact keys):

| Condition | Code |
|---|---|
| Unresolvable decorator identity | `PROJ-C004` |
| Unresolvable base class | `PROJ-C005` |
| Circular import in resolution | `PROJ-C012` |
| Module source unavailable (external/virtual) for a class that uses decorators | `PROJ-C006` |
| Unsupported decorator form (computed, call-chain, etc.) | `PROJ-C007` |

#### Output membership filter

After deriving exact candidates, the shared compiler applies the
adapter-provided output membership filter:

1. For each candidate class, find the `ModuleNode.id` that defines it.
2. Check whether that module ID appears in at least one `OutputMembership`
   output's contributing set.
3. If yes: the component is shipped and enters the manifest.
4. If no: the component was tree-shaken and is silently excluded from the
   manifest. No diagnostic is emitted for excluded components.

This ensures that a tree-shaken component does not appear in the manifest and
does not trigger a coverage requirement in `webui build`.

### Manifest schema

The manifest is a versioned deterministic UTF-8 JSON file. One bundler
invocation produces one manifest fragment. Multiple fragments may be merged
by `webui build` (see [Fragment merge](#projection-fragment-merge)).

#### Top-level structure

```typescript
export interface ProjectionManifest {
  /** Schema identifier — always "webui.state-projection/v1". */
  readonly schema: "webui.state-projection/v1";

  /** Producer identity. */
  readonly producer: {
    readonly name: "@microsoft/webui/projection.js";
    readonly version: string;
  };

  /** Bundler adapter identity. */
  readonly adapter: {
    /** Adapter name, e.g. "esbuild". */
    readonly name: string;
    /** Bundler name@version string, e.g. "esbuild@0.28.1". */
    readonly bundler: string;
  };

  /**
   * Build root relative to the manifest directory: ".", "..", "../..", etc.
   * Every physical path below is relative to this root.
   */
  readonly root: string;

  /**
   * SHA-256 of normalized entry IDs, module/source identities, exact resolved
   * import edges, and output membership.
   */
  readonly analysisHash: string;

  /**
   * Deterministic build identifier covering inputs, graph, config, outputs, and
   * version. Format: "sha256:<64-hex-chars>".
   */
  readonly buildId: string;

  /**
   * Emitted output files.
   * Key: canonical build-root-relative path, or `virtual:<hex-id>`.
   * Value: exact file-byte SHA-256, or "virtual" only for a virtual key.
   */
  readonly outputs: Record<string, string>;

  /**
   * Every module in the adapter graph, including tree-shaken modules.
   * Key: canonical build-root-relative path, or `virtual:<hex-id>`.
   * Value: exact UTF-8 source SHA-256, or "virtual" only for a virtual key.
   */
  readonly inputs: Record<string, string>;

  /**
   * Component entries keyed by custom-element tag name.
   */
  readonly components: Record<string, ComponentEntry>;
}

export interface ComponentEntry {
  /** Canonical build-root-relative physical defining module. */
  readonly module: string;
  /** Canonical output keys that include this component. */
  readonly outputs: readonly string[];
  /** Sorted exact @observable + @attr property keys used by initial bootstrap. */
  readonly hydrationKeys: readonly string[];
  /** Sorted exact @observable + @attr property keys used by navigation. */
  readonly navigationKeys: readonly string[];
}
```

#### Ordering and determinism rules

The manifest must be reproducible byte-for-byte given the same inputs,
graph, configuration, and tool versions:

1. **No timestamps.** No `builtAt`, `date`, `time`, or any time-derived field.
2. **Sorted object keys.** `outputs`, `inputs`, and `components` are sorted
   lexicographically by raw UTF-8 bytes, ascending.
3. **Sorted arrays.** `ComponentEntry.outputs`, `hydrationKeys`, and
   `navigationKeys` are sorted and deduplicated by raw UTF-8 bytes.
   `navigationKeys` must contain every `hydrationKeys` entry.
4. **Normalized paths.** All paths use forward slashes. No leading `./`.
   Physical keys are relative to `root`, never the manifest directory.
5. **Compact JSON serialization.** No trailing newlines, no pretty-printing
   (for the canonical form that participates in hashing). The written file
   may be pretty-printed for readability, but hashing uses compact form.
6. **Stable enum values.** No booleans substituted for integers in future
   versions; new optional fields are added with `undefined` (absent, not `null`).

#### Path normalization algorithm

The adapter supplies absolute `rootDir`, `manifestPath`, module IDs, and output
IDs:

1. `manifestPath` and every physical input/output must be within `rootDir`.
2. Serialize `root = relative(dirname(manifestPath), rootDir)` using forward
   slashes. It must be `"."` or only parent segments (`".."`, `"../.."`, up to
   32 segments). A common layout is manifest `project/dist/...` and
   `root: ".."`.
3. Serialize each physical file as `relative(rootDir, absoluteFile)`, replacing
   separators with `/`. File keys must be non-empty and contain no `.`, `..`,
   absolute prefix, backslash, or control-character segment.
4. Serialize virtual IDs as `virtual:` plus lowercase hexadecimal UTF-8 bytes
   of the ID after its leading NUL.
5. A `"virtual"` hash is valid only for a `virtual:` key. A physical output
   must always carry exact bytes and a SHA-256 hash.
6. Consumers canonicalize `root` and every physical file and verify the
   resulting path remains below the canonical root (including symlink checks).

### Hash and build-ID algorithm

All hashes use **SHA-256** over exact bytes. Source text supplied as a string is
encoded as UTF-8 exactly as supplied (including a BOM code point when present);
output adapters should pass emitted bytes directly. Hash values are lowercase
hexadecimal prefixed with `"sha256:"`.

#### File content hash

For a file-backed module or output file:

```
hash = "sha256:" + hex(sha256(utf8_bytes_of_file))
```

For virtual modules: `"virtual"` (literal string, not a hash).

#### Analysis hash

`analysisHash` is SHA-256 over length-prefixed canonical records for:

- sorted entry IDs;
- every sorted module ID, kind, source hash, and sorted import edge
  (`specifier`, resolved ID, external/internal, static/dynamic, package name);
- every sorted output ID and its sorted contributing module IDs.

It proves the semantic graph and final output membership without requiring the
Rust consumer to reproduce bundler resolution.

#### Build ID

The build ID covers producer/adapter versions, `root`, `analysisHash`, every
input/output hash, and every component's defining module, exact output
membership, and exact keys.

Canonical records use this unambiguous encoding:

```text
<record-label><utf8-byte-length>:<field><utf8-byte-length>:<field>...\n
```

The record sequence is:

```text
schema(schema-id)
producer(name, version)
adapter(adapter-name, bundler-name@version)
root(root)
analysis(analysisHash)
inputs(count)
input(path, hash) ... sorted by UTF-8 path bytes
outputs(count)
output(path, hash) ... sorted by UTF-8 path bytes
components(count)
component(
  tag,
  module,
  output-count,
  outputs...,
  hydration-key-count,
  hydration-keys...,
  navigation-key-count,
  navigation-keys...
)
  ... sorted by UTF-8 tag bytes
```

Each record ends in exactly one LF. Decimal lengths count UTF-8 bytes, not
UTF-16 code units. The final identifier is:

```text
"sha256:" + hex(sha256(canonical_record_bytes))
```

Cross-language golden vector:

```text
producer = @microsoft/webui/projection.js@0.0.18
adapter = esbuild / esbuild@0.28.1
root = ..
analysisHash = sha256:1111111111111111111111111111111111111111111111111111111111111111
input = src/a.ts / sha256:2222222222222222222222222222222222222222222222222222222222222222
output = dist/a.js / sha256:3333333333333333333333333333333333333333333333333333333333333333
component = a-card / src/a.ts / [dist/a.js]
            / hydration [displayValue]
            / navigation [displayValue, é]
buildId = sha256:8319202a060626c39cce76df50197c92dee27aab29d601161183c188204d7c18
```

#### Stale validation

`webui build` re-hashes declared input and output files to detect staleness:

1. Resolve and canonicalize `root`.
2. For each physical `inputs` entry: read the root-relative file and compare
   the computed SHA-256 against the manifest value. Mismatch → `PROJ-M003`.
3. For each physical `outputs` entry: read the root-relative file and compare
   the computed SHA-256 against the manifest value. Mismatch → `PROJ-M004`.
4. Validate virtual/hash pairing, component references, sorted/deduplicated
   arrays, and `analysisHash` format.
5. Re-derive the build ID from current hashes and compare against
   `manifest.buildId`. Mismatch → `PROJ-M005`.

All three checks must pass before any manifest data is consumed. A partially
written manifest (missing declared files) → `PROJ-M001`.

### Fragment merge semantics

`webui build` accepts a repeatable `--projection-manifest` option:

```bash
webui build ./src \
  --plugin webui \
  --components @microsoft/shared-controls \
  --projection-manifest ./dist/app-projection.json \
  --projection-manifest ./shared-controls/dist/control-projection.json \
  --out ./dist
```

Each argument path must point to a valid, non-stale manifest file. Manifests
are loaded and merged by component tag:

1. **Schema compatibility.** Every manifest must have `schema:
   "webui.state-projection/v1"`. An unsupported schema version produces
   `PROJ-M002`.
2. **Duplicate ownership.** The same component tag appearing in two or more
   manifests produces `PROJ-M006` (duplicate tag ownership, hard error).
3. **Conflicting keys.** Not applicable after deduplication by rule 2; each
   tag is owned by exactly one manifest.
4. **Merged result.** The merged component map contains every entry from all
   manifests. File identity is the canonical absolute path obtained from each
   fragment's own `root`; identical relative names under different roots do
   not conflict. If two individually validated fragments refer to the same
   canonical file with different hashes (for example due to a concurrent
   rewrite race), merging produces `PROJ-M007`.

After merging, the merged map is used for strict coverage validation.

### Strict WebUI build validation

After loading and merging all manifests, `webui build` validates strict coverage:

1. Determine the **compiled scripted components**: every component tag that
   appears in the protocol's compiled fragment graph **and** has a sibling
   client module (non-scriptless). This includes:
   - Route component roots.
   - Components reachable via component-asset closure roots.
   - Any component compiled into the protocol whose tag is not marked scriptless.
2. For each compiled scripted component, exactly one manifest entry must be
   present. Missing entry → `PROJ-B001`.
3. Manifest entries for components that exist in the template tree but are
   unused (not compiled into the protocol, not in any route or asset closure)
   do **not** trigger an error. They are silently ignored.
4. The merged manifest may contain components that WebUI has no template for.
   This is permitted (external controls, future components). No warning.

After coverage validation, `webui build`:

1. For each matched scripted component, reads
   `ComponentEntry.hydrationKeys` and `ComponentEntry.navigationKeys`.
2. Sets `ComponentData::hydration_mode = StateProjectionMode::Keys` and
   `ComponentData::hydration_keys = entry.hydrationKeys`.
3. Sets `ComponentData::navigation_mode = StateProjectionMode::Keys` and
   `ComponentData::navigation_keys =
   union(entry.navigationKeys, template_roots(tag))`.
4. Sets `WebUIProtocol::initial_state_strategy = InitialStateStrategy::Components`.
5. Scriptless components continue to use `StateProjectionMode::None` for
   hydration and their template roots for navigation.

### Rust responsibilities and integration

Rust (`webui build`, `webui serve`, `webui-parser`, `webui` crate) owns:

- Template root extraction (`tr` field from compiled template metadata).
- Scriptless component detection and compiler-owned host emission.
- Navigation union: `navigation_keys = union(manifest_keys, template_roots)`.
- `InitialStateStrategy` and `StateProjectionMode` protocol encoding.
- Route closure and component-asset closure reachability.
- Strict coverage validation (every compiled scripted component must be in merged manifest).
- Manifest loading, path normalization, stale validation, and fragment merge.
- Projection-enabled vs. disabled mode selection.

Default and FAST plugin builds continue to use `InitialStateStrategy::Full`.
Passing `--projection-manifest` with a non-WebUI plugin is a hard error
(`PROJ-B002`): only the WebUI plugin produces protocol fields compatible with
per-component key encoding.

The Rust `BuildOptions` struct accepts composable path and inline sources:

```rust
pub enum ProjectionManifestSource {
    Path(PathBuf),
    Inline {
        /// Logical location anchoring root and stale file validation.
        manifest_path: PathBuf,
        json: String,
    },
}

/// Empty means projection is disabled.
pub projection_manifests: Vec<ProjectionManifestSource>,
```

Schema parsing, canonical ordering/reference validation, and build-ID
recomputation live in `webui-protocol::projection_manifest` so native and WASM
hosts share one contract. `webui` adds filesystem root, symlink, and stale
input/output validation for path and inline native sources.

The handler runtime never reads manifest files. Protocol fields are the sole
runtime source of projection metadata.

### CLI, Node, and WASM input shape

#### CLI

```text
webui build ./src --plugin webui --projection-manifest <PATH> [--projection-manifest <PATH>] ...
webui serve ./src --plugin webui --projection-manifest <PATH> [--projection-manifest <PATH>] ...
```

`--projection-manifest` is repeatable and corresponds to
`BuildOptions::projection_manifests`. In watch mode, the server watches manifest
files for changes; a new valid manifest triggers a protocol rebuild. A stale or
invalid manifest produces a structured build error that is displayed and held
until the next valid manifest update.

Manifest files are registered as explicit watcher files through their parent
directories. They bypass the normal `dist/` ignore rule, while sibling bundle
chunks and adapter temporary files remain ignored. This makes the adapter's
atomic rename of the final manifest the sole client-build synchronization
event. The existing rebuild worker stays alive after an error and clears the
stored error on the next successful manifest-triggered rebuild.

#### Node

`@microsoft/webui` accepts manifest paths and already-transported objects:

```typescript
import { build } from '@microsoft/webui';
await build({
  appDir: './src',
  projectionManifests: ['./dist/webui-projection.json'],
  projectionManifestObjects: [{
    path: './dist/other-projection.json',
    manifest: otherManifest,
  }],
});
```

The package serializes inline objects once at the NAPI boundary. NAPI receives
paths plus `{path, json}` records and performs all validation on the Rust side;
it never depends on compiler or esbuild packages. The CLI fallback supports
paths and rejects inline objects with an actionable message rather than
silently writing files.

#### WASM

The WASM parser export accepts an optional third argument containing an array
of manifest objects:

```typescript
buildProtocol(files, entry, projectionManifests?)
```

Without manifests, initial state is full and scripted navigation surfaces stay
`All`. With manifests, WASM uses the shared structural/build-ID validator,
merges tags, enforces strict compiled-scripted coverage, and encodes the same
protocol fields. It cannot perform filesystem stale checks and never analyzes
JavaScript.

### Projection diagnostic codes

All codes are stable and machine-readable. They appear in the `code` field of
a `Diagnostic` object alongside `title`, `location`, `snippet`, and `help`.
No color in diagnostic data; color is added only by `webui-cli` output layer.

#### Compiler diagnostics (PROJ-C*)

| Code | Severity | Condition |
|------|----------|-----------|
| `PROJ-C001` | error | TypeScript parse error in source file |
| `PROJ-C002` | error | Import specifier does not resolve to any module in the graph |
| `PROJ-C003` | error | Named import not found in the resolved module's exports |
| `PROJ-C004` | error | Decorator cannot be resolved to `observable` or `attr` from `@microsoft/webui-framework`; cannot prove exact keys |
| `PROJ-C005` | error | Base class cannot be resolved to a class declaration in the graph; cannot prove exact inherited keys |
| `PROJ-C006` | error | Class uses `@observable`/`@attr` decorators but its module source is unavailable (external/virtual) |
| `PROJ-C007` | error | Unsupported decorator form (computed property key, call-chain decorator, reflection-based) |
| `PROJ-C008` | error | `define()` tag argument is not a string literal |
| `PROJ-C009` | error | Reserved for an adapter that explicitly claims a WebUI class target but cannot supply its declaration |
| `PROJ-C010` | error | Duplicate `define()` for the same tag within one adapter context |
| `PROJ-C011` | error | `Class.define(...)` called with wrong argument count |
| `PROJ-C012` | error | Circular import detected during symbol resolution |
| `PROJ-C013` | error | Adapter graph is incomplete/inconsistent (unknown entry/member, missing resolved edge/source, path outside root) |
| `PROJ-C014` | error | Adapter omitted exact bytes for a physical emitted output |

#### Peer dependency diagnostics (PROJ-P*)

| Code | Severity | Condition |
|------|----------|-----------|
| `PROJ-P001` | error | Required peer `typescript` is absent or below the supported range |
| `PROJ-P002` | error | Required peer `esbuild` is absent or below the supported range (esbuild adapter only) |
| `PROJ-P003` | warning | Peer is present but above the tested range; results may differ |

#### Manifest diagnostics (PROJ-M*)

| Code | Severity | Condition |
|------|----------|-----------|
| `PROJ-M001` | error | Manifest file is missing or unreadable |
| `PROJ-M002` | error | Manifest schema version is not `webui.state-projection/v1` |
| `PROJ-M003` | error | Declared input file hash does not match current file content (stale manifest) |
| `PROJ-M004` | error | Declared output file hash does not match current file content (stale manifest) |
| `PROJ-M005` | error | Manifest `buildId` does not match recomputed build ID |
| `PROJ-M006` | error | Same component tag owned by two or more manifests (duplicate ownership) |
| `PROJ-M007` | error | Same path key in merged `inputs` or `outputs` has conflicting hashes |
| `PROJ-M008` | error | Manifest JSON is syntactically invalid |
| `PROJ-M009` | error | Required manifest field is missing or has wrong type |

#### Build validation diagnostics (PROJ-B*)

| Code | Severity | Condition |
|------|----------|-----------|
| `PROJ-B001` | error | Compiled scripted component has no manifest entry (missing coverage) |
| `PROJ-B002` | error | `--projection-manifest` supplied with a non-WebUI plugin |

#### Security and resource diagnostics (PROJ-S*)

| Code | Severity | Condition |
|------|----------|-----------|
| `PROJ-S001` | error | Manifest file exceeds the 16 MiB size limit |
| `PROJ-S002` | error | Manifest `components` count exceeds 65,535 |
| `PROJ-S003` | error | Normalized path traverses outside the project root (path traversal attempt) |
| `PROJ-S004` | error | Hash format is invalid (not `"sha256:<64-hex>"` or `"virtual"`) |

### Security and resource constraints

The Rust manifest consumer enforces the following before parsing manifest JSON:

1. **Size limit.** Reject any manifest file exceeding **16 MiB** (`PROJ-S001`).
2. **Component count.** Reject manifests with more than **65,535** component
   entries (`PROJ-S002`).
3. **Root/path validation.** `root` is `"."` or at most 32 parent segments.
   Every physical key is root-relative and contains no absolute prefix,
   backslash, control character, `.`, or `..` segment. Canonicalized files
   (including symlink resolution) must remain within the canonical root
   (`PROJ-S003`).
4. **Hash validation.** A hash is exactly `sha256:` plus 64 lowercase
   hexadecimal bytes, checked by an explicit byte scanner. `"virtual"` is
   accepted only for a `virtual:` key; physical disk outputs can never use it
   (`PROJ-S004`).
5. **No code execution.** The Rust consumer never evaluates JavaScript or
   invokes TypeScript APIs. Manifests are pure data files.
6. **Atomic writes.** The TypeScript adapter must write the manifest
   atomically (write to a temporary path in the same directory, then rename)
   to ensure `webui build` never reads a partially written manifest.

### Adapter conformance fixtures and test contract

The canonical conformance test helpers and reference cases live in
`packages/webui/src/projection/fixtures/conformance.ts`.

#### Required fixture scenarios

An adapter implementation is conformant if it produces manifests that match the
expected output for all of the following scenarios. Reference input TypeScript
sources and expected manifests are co-located with the conformance helpers.

| Fixture ID | Scenario |
|---|---|
| `basic-single-entry` | Single entry; `@observable` and `@attr` property names enter both exact client surfaces |
| `empty-keys` | Component with no reactive properties; both key arrays are empty |
| `aliased-decorator` | `@observable` imported under a local alias |
| `namespace-decorator` | `@observable` accessed through a namespace import (`import * as webui`) |
| `re-export-chain` | `observable` re-exported through multiple barrel files |
| `inheritance-single` | Class inherits keys from a single base class |
| `inheritance-multi` | Class inherits keys through a two-level chain |
| `code-splitting` | Two entries, split chunks; component appears only in the chunk, not the main bundle |
| `tree-shaking` | Component class present in source but tree-shaken from all outputs; must not appear in manifest |
| `shared-component` | Component imported by multiple entries and deduplicated in a shared chunk |
| `external-bundle` | Two manifests merged for a shared control built separately |
| `virtual-source` | Module with no physical file (`source: undefined`); valid if no decorators |
| `duplicate-tag-error` | Two `define()` calls for the same tag; must produce `PROJ-C010` |
| `unresolvable-base-error` | Base class cannot be found in graph; must produce `PROJ-C005` |
| `dynamic-tag-error` | `define()` with a non-literal tag; must produce `PROJ-C008` |
| `stale-input-error` | Manifest input hash mismatches file on disk; must produce `PROJ-M003` |
| `missing-coverage-error` | WebUI build with compiled scripted component absent from manifest; must produce `PROJ-B001` |

#### Conformance test helper interface

```typescript
export interface ConformanceCase {
  /** Unique fixture identifier, e.g. "basic-single-entry". */
  readonly id: string;
  /** Human-readable description of the scenario. */
  readonly description: string;
  /** Compiler fixtures run in JS; Rust fixtures are skipped by this runner. */
  readonly scope: "compiler" | "rust";
  /** Input module graph (adapter-resolved). */
  readonly graph: ModuleGraph;
  /** Input output membership. */
  readonly membership: OutputMembership;
  /** Exact bytes for physical fixture outputs. */
  readonly outputContents: ReadonlyMap<string, string | Uint8Array>;
  /** Expected component entries, or null for a diagnostic fixture. */
  readonly expectedComponents: Readonly<Record<string, ComponentEntry>> | null;
  /** Expected diagnostic codes when expectedComponents is null. */
  readonly expectedDiagnosticCodes: readonly string[];
}

/**
 * Runs all conformance cases against the provided adapter factory.
 * Returns a report of passed, failed, and skipped cases.
 */
export function runConformanceSuite(
  adapterFactory: (ctx: AdapterContext) => Promise<ProjectionManifest>,
  options?: { filter?: (c: ConformanceCase) => boolean }
): Promise<ConformanceReport>;

export interface ConformanceReport {
  readonly passed: readonly string[];
  readonly failed: readonly ConformanceFailure[];
  readonly skipped: readonly string[];
}

export interface ConformanceFailure {
  readonly id: string;
  readonly reason: string;
  readonly expected: unknown;
  readonly actual: unknown;
}
```

#### Determinism test

For every non-error fixture, running the adapter twice with identical inputs
must produce byte-for-byte identical manifest JSON. This verifies:

- No timestamps in output.
- Deterministic key ordering.
- Deterministic path normalization.
- Deterministic build ID.

#### Merge contract test

The `external-bundle` fixture provides two manifests for merging. The
test verifies:

1. No `PROJ-M006` when tags are disjoint.
2. Merged `components` contains entries from both manifests.
3. Merged `outputs` and `inputs` contain the union.
4. `PROJ-M006` is produced when the same tag appears in both manifests.

#### Coverage contract test

The `missing-coverage-error` fixture provides a partial manifest (missing one
compiled scripted component). The test verifies that `webui build`'s validation
step produces exactly `PROJ-B001` for the missing tag and fails the build.

### esbuild adapter specification

The esbuild adapter is implemented in `packages/webui/src/projection/adapters/esbuild.ts`.

```typescript
import type { Plugin } from 'esbuild';

export interface EsbuildProjectionOptions {
  /**
   * Absolute or CWD-relative path where the manifest will be written.
   * Defaults to `<outdir>/webui-projection.json`.
   */
  manifest?: string;
}

/**
 * Returns an esbuild plugin that compiles the projection manifest.
 * The plugin enables metafile, reads the resolved graph, applies the
 * projection compiler, and writes the manifest atomically on build success.
 *
 * The adapter imports esbuild types only. The application-owned esbuild
 * instance invokes the plugin and exposes its runtime/version through
 * PluginBuild.esbuild.
 */
export function esbuildProjection(options?: EsbuildProjectionOptions): Plugin;
```

The esbuild adapter:

1. Enables `metafile` on the existing build; it never invokes esbuild again.
2. Validates `PluginBuild.esbuild.version` against the supported `^0.28.1`
   range (`PROJ-P002`).
3. Reads `metafile.inputs[*].imports[*].original` for the authored specifier
   and `.path` for esbuild's resolved target. This directly populates
   `ResolvedImport`; no path/extension reconstruction occurs.
4. Determines semantic package identity from the resolved physical target's
   nearest `package.json`, so esbuild aliases are honored. Literal package text
   never overrides the resolved target.
5. Reads `metafile.outputs[*].inputs` for final output membership and
   `output.entryPoint` for normalized graph entries.
6. Reads exact file source bytes/text for physical inputs. Non-file namespace
   inputs are represented as virtual graph nodes; a WebUI component whose
   defining source is unavailable fails strict coverage instead of being
   guessed.
7. Hashes exact `result.outputFiles` bytes for `write: false`, or reads emitted
   files during `onEnd` for `write: true` (esbuild has completed writes before
   `onEnd`).
8. Chooses the common ancestor of the manifest, physical inputs, and outputs
   as `rootDir`, constructs `AdapterContext`, and calls the shared compiler.
9. Writes canonical compact JSON to a same-directory temporary file, flushes
   it, and atomically renames it over the manifest.
10. If the build or projection compiler has errors, the manifest is **not**
   written. An existing stale
   manifest from a prior run is left in place (not deleted).

The adapter does **not** search output text for decorator syntax. The final
emitted outputs determine membership; the source AST determines semantics.

A single `esbuild.build()` call with `metafile: true` and code splitting is
fully supported. External components are absent from the application fragment
and produce their own fragment when built separately with the same adapter.
The adapter handles all outputs in one `onEnd` pass.

### webui-press integration

`webui-press` invokes esbuild's JavaScript API once through
`@microsoft/webui/projection.js`, then validates the generated manifest once.
The resulting `PreparedProjectionManifests` is reused by every page and the 404
build; page builds never re-open or re-hash bundle files. The prepared handle
is an `Arc`-backed immutable snapshot containing both component surfaces and
canonical artifact identities. A page using one prepared source clones only
the `Arc`; mixed prepared/fresh sources retain artifact identities so
conflicting hashes still fail with `PROJ-M007`.

To preserve build throughput without exposing a public compile/finalize split,
press uses a hidden orchestration barrier:

1. Start the one esbuild/projection build on a worker thread.
2. Start parallel page `webui::build()` calls immediately.
3. Each build performs template discovery/parsing while esbuild runs.
4. At the internal projection-finalization point, builds wait for the prepared
   manifest proof, apply exact surfaces, then render.

External bundle fragments can be listed in
`bundler.projectionManifests` (paths relative to `config.json`) and are merged
with the generated application fragment before the barrier completes.

On the 33-page documentation site, an initial strictly sequential
implementation regressed warm build wall time by 49.0%. Precise component
seeding, lazy AST parsing, compiler preloading, one-time manifest preparation,
and the projection barrier reduced the controlled interleaved regression to
11.8% (1.225 s → 1.370 s). The eight emitted browser JavaScript files changed
from 529,140 to 529,136 bytes (−4 bytes, effectively unchanged).

### Official esbuild examples

Official WebUI examples use small JavaScript-API build scripts instead of the
esbuild CLI so the projection adapter participates in the application's one
client build:

```text
build workspace dependencies
esbuild + esbuildProjection() → JS chunks + webui-projection.json
webui build --projection-manifest ... → protocol.bin
```

`examples/build-webui-client.mjs` centralizes watch/color/plugin wiring while
each app retains its own esbuild entry/output/splitting options. `cargo xtask
dev` starts the client watcher first, removes stale manifests, waits for every
repeated manifest path to appear atomically, and only then starts the server.

The component-assets example also builds an `external-panel` bundle separately,
passes a second manifest alongside the application manifest, and emits the
external control as a component asset. Its projection-contract check
deliberately omits the external fragment and requires `PROJ-B001`, covering the
strict missing-fragment failure.

**Machine-readable diagnostics.** `webui-cli` accepts a global `--format <human|json>` flag. In `json` mode the colorized terminal output is suppressed and each error is emitted as a single JSON object on **stdout** (`{severity, code, message, file, line, column, snippet, help, chain}`), so editors, CI, and AI assistants consume diagnostics without scraping ANSI text. The process exit code follows BSD `sysexits.h` so callers can branch on the cause: `65` (`EX_DATAERR`) for a template/authoring error, `66` (`EX_NOINPUT`) for a missing app folder / state file / serve dir / entry, `69` (`EX_UNAVAILABLE`) for an occupied port, `74` (`EX_IOERR`) for other I/O failures, `2` for argument/usage errors (clap), and `1` otherwise.

`tx[]` stores text runs as `[slot, parts]`, where `parts` reuse the compact attribute-part encoding (`string` for static text, `[path]` for dynamic text). Client-created DOM inserts one runtime `Text` node per run instead of scanning compiled marker comments.

Attribute bindings are recorded in `a[]`, while `ag[]` points at the owning element and the contiguous `[start, count)` range inside `a[]`. The compiled client HTML never embeds `data-w-*` markers; those remain SSR-only handler markers.

Nested `<if>` / `<for>` blocks are recursively compiled into the shared `b[]` block table. The client runtime instantiates compiled child blocks directly and evaluates precompiled condition AST tuples — it does not parse raw template syntax or condition strings from repeat or conditional body content.

The private workspace package `packages/webui-test-support` (`@microsoft/webui-test-support`) exists to build this metadata shape in JS-side tests without duplicating tuple encodings or fixture infrastructure across `webui-framework` and `webui-router`. It centralizes fixture builders such as `buildTemplate`, `registerCompiledTemplate`, and the condition AST helpers, and it also provides shared Node-side fixture bundling/server helpers so browser fixture apps and Playwright servers stay aligned with the runtime/compiler contract as that contract evolves.

### Plugin data and SSR hydration markers

The current WebUI parser emits a 12-byte `Plugin` fragment (`WebUIElementData`) for each element that has attribute bindings or `@event` handlers:

```
Bytes 0–3:  binding_count   (u32 LE)  — number of dynamic attribute bindings
Bytes 4–7:  event_start_idx (u32 LE)  — global index into the parser event list
Bytes 8–11: event_count     (u32 LE)  — number of @event attrs on this element
```

The handler decodes this in `on_element_data` and emits SSR-only markers:

- `data-w-b-N` for one bound attribute, or `data-w-c-START-COUNT` for multiple `a[]` entries on the same element
- `data-ev="COUNT"` once per element, where `COUNT` is the number of consecutive parser event entries that belong to that element

WebUI SSR marker formats are:

| Marker | Format | Notes |
|--------|--------|-------|
| Repeat block start | `<!--wr-->` | Opens a `<for>` loop region |
| Repeat block end | `<!--/wr-->` | Closes the `<for>` loop region |
| Repeat item | `<!--wi-->` | Marks each iteration boundary inside a repeat |
| Conditional start | `<!--wc-->` | Opens an `<if>` block |
| Conditional end | `<!--/wc-->` | Closes the `<if>` block |

The WebUI handler plugin emits only these five comment markers. Text bindings, attribute bindings, and event handlers are resolved from compiled metadata path indices at hydration time - no DOM attribute markers are needed. The handler only emits markers in active child scopes; the root page scope remains marker-free. During hydration the framework keeps `<!--wr-->` and `<!--wc-->` as runtime anchors and removes `<!--/wr-->`, `<!--/wc-->`, and `<!--wi-->` markers.

WebUI Framework hydration assumes the SSR DOM, hydration markers, and compiled metadata were generated by the same trusted WebUI compiler/handler version. Hand-authored or partially modified marker streams are unsupported; missing structural closing markers are invalid input, not a recoverable runtime condition.

### Runtime contract

`@microsoft/webui-framework` consumes the metadata object above plus the SSR markers emitted by `WebUIHydrationPlugin`. This follows an Islands Architecture approach: the server delivers fully-rendered HTML, authored Web Components hydrate on startup, and compiler-owned scriptless hosts remain dormant until browser code actually writes state.

- SSR hydration uses one DOM walk to discover `<!--wr-->`, `<!--wi-->`, and `<!--wc-->` comment markers, wire the relevant bindings using compiled metadata path indices, then remove SSR-only markers.
- Authored browser entries execute only after every SSR instance they may
  upgrade has complete markup. Parser-inserted, non-async ES module scripts and
  classic `defer` scripts satisfy this automatically; blocking classic scripts
  must appear after all such instances. Under this loading contract,
  `TemplateElement.connectedCallback()` hydrates synchronously, so
  `super.connectedCallback()` returns only after that component's bindings,
  events, and references are wired.
- Before a containing WebUI component hydrates, descendants must not
  structurally mutate its SSR subtree. Hydration resolves compiled node paths
  against the trusted server DOM and does not recover from pre-hydration node
  insertion, removal, or reordering.
- Client-created DOM never reparses template syntax; it clones marker-free `h`,
  upgrades the detached custom-element subtree, resolves `tx`, `ag`, the slots
  embedded in `c` / `r`, and event target paths directly, then applies the first binding pass before
  appending nodes to the connected DOM. Child components therefore observe
  initial parent `:` property bindings in `connectedCallback`, while later parent
  updates remain live.
- **Hydration-mismatch diagnostic (#379).** SSR hydration is single-pass: the
  runtime wires bindings to the server-rendered DOM and trusts it (it never runs
  a first binding pass over SSR roots). A reactive write that lands while the
  element is connected but before hydration finishes — an `@observable` field
  initializer, a constructor assignment, or an assignment before
  `super.connectedCallback()` — updates the backing field but cannot touch the
  DOM yet, so the value is dropped and the element's own observable ends up
  disagreeing with its DOM (silently, and inconsistently with client-side
  rendering). The runtime records those pre-ready write paths and, once hydrated,
  performs a **read-only** comparison of each against the server-rendered DOM. If
  any disagree it emits a single `console.warn` naming the properties and the
  host tag — the same hydration-mismatch signal React, Vue, Svelte, and Solid
  produce. It does **not** reconcile: patching would repair only the
  post-hydration state while leaving the first server paint wrong, and would
  erode the SSR-trust invariant that lets a child hydrate parent-delayed `:`
  bindings before the parent sets them (#286), so complex `:` bindings are exempt
  from the comparison. The lifecycle rule for authors: a value that must appear
  in the initial render belongs in the SSR state; assign anything else after
  `super.connectedCallback()`. Components that follow this rule allocate nothing
  on the hot path — the tracking `Set` stays `null` and the check early-returns.
  The diagnostic is **development-only**. Its comparators and message string live
  in `hydration-mismatch.ts` behind the `reportHydrationMismatch` entry point,
  reached solely through a dynamic `import()` gated by the module-local `DEV`
  constant — derived from the compile-time flag `__WEBUI_DEV__` as
  `typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__`, so an **undefined** flag
  defaults the diagnostic **on** (raw ESM, the framework's own `tsc` output, and
  unit tests keep the warning without any bundler cooperation). When a bundler
  folds `__WEBUI_DEV__` to `false`, `DEV` folds with it: `$checkHydrationMismatch`
  empties and its lone `import()` is dead-code-eliminated, dropping the whole
  diagnostic module — comparison code *and* strings — from the output. The dynamic
  import is load-bearing: esbuild fixes static-import reachability before
  constant-folding and never re-runs tree-shaking, so a static import would ship
  even when its only caller folds away. `webui-press build` injects
  `--define:__WEBUI_DEV__=false` automatically (and `serve` leaves it undefined);
  apps that bundle their own client define the flag as `false` for production.
- Scriptless components receive compiled `template_json` with `th: 1` but no
  `hydration_keys` or initial bootstrap state. The framework registers a
  compiler-owned `TemplateElement` host for each such tag. Existing SSR DOM is
  not walked and bindings are not installed on startup. The host activates only
  after `setState`, a compiled parent property write, or a later observed
  attribute change. Activation wires the existing SSR markers against the new
  state and replays only the roots supplied by the triggering write. Omitted
  text, attribute, condition, and repeat roots keep their trusted SSR DOM until
  explicitly supplied; an explicit empty collection removes repeat items.
  Client-created instances mount immediately from the cached template.
  `WebUIElement` remains the authored layer for events, `w-ref`, lifecycle code,
  decorators, and `$emit`.
- Developer-authored `WebUIElement` classes also treat compiled template roots
  as navigation state. `setState()` stores undecorated template-bound roots in
  hidden framework state, so `@observable` is only required when TypeScript
  reads or mutates the property directly. Initial SSR projection includes
  explicit `@observable + @attr`; existing host attributes override matching
  bootstrap keys, while absent attributes use state. Template-only values stay
  out because they already exist in the trusted SSR DOM. Navigation carries
  `@observable + @attr + template roots`.
- The router publishes initial and partial template registrations through
  `webui:templates-registered`. This lets the framework claim scriptless route
  tags before the router commits a partial, preserving soft navigation without
  empty modules. Tags owned by configured lazy loaders are reserved in
  `window.__webui.templateHostExclusions`; the framework defers its initial
  registry claim by one task and never defines compiler-owned hosts for those
  tags. This keeps `customElements.define()` available to the authored module.
  After configured lazy loaders run, document navigation is used only when
  neither authored code nor the compiler-owned host runtime registers the
  destination tag. Route chain JSON has no `client` capability flag.
- Events are resolved from compiler-grouped `eg[]` metadata entries using path
  indices. The compiler groups element events by event name and marks handlers
  that receive `e`, so the runtime installs one delegated listener per event
  name on the component render root without regrouping or scanning event
  arguments during hydration. It resolves handler
  arguments against the scope captured when that block was rendered. Nested
  conditional/repeat instances unregister their delegated listeners when removed
  so detached DOM is not retained. Root events from `re[]` attach directly to the
  host element or shadow root.
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
├── dotnet/
│   ├── src/Microsoft.WebUI/  # Managed .NET bindings for webui-ffi
│   ├── runtime/              # RID-specific native runtime NuGet packages
│   └── tool/Microsoft.WebUI.Tool/ # .NET global tool package
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

webui-ffi ──────► webui-handler ◄────── webui-wasm (handler feature)
     └──────────► webui-protocol   ┌──── webui-wasm (parser feature)
                                   └──── webui-wasm (all/default feature)
```

The `webui` library crate is the primary API surface for programmatic use.
It re-exports `WebUIHandler`, `Protocol`, `ResponseWriter`, and
`WebUIProtocol` from their respective crates and provides `build()`,
`build_to_disk()`, and `inspect()` functions with `BuildStats` (duration,
fragment/component/CSS counts, protocol size).

### WASM Distribution

The `microsoft-webui-wasm` crate exposes feature-gated browser bindings so
consumers only ship the parser and/or handler code they need:

- `handler` builds `webui_wasm_handler.js` and exports `Protocol`. It accepts
  protobuf protocol bytes and depends on `webui-handler` and `webui-protocol`,
  not `webui-parser`. `Protocol` decodes and indexes once, binds the selected
  plugin at construction, and provides `render`, `renderStream`,
  `renderPartial`, `renderComponentTemplates`, and `tokens`. Callback rendering
  coalesces handler fragments with a
  16 KiB target before crossing the WASM-to-JavaScript boundary.
- `parser` builds `webui_wasm_parser.js` and exports `build_protocol`. It
  returns protobuf protocol bytes and depends on `webui-parser` and
  `webui-protocol`, not `webui-handler`.
- `all` is the default feature, builds `webui_wasm_all.js`, and exports the
  parser plus handler surfaces for playground-style live preview. Callers
  compose `build_protocol()` with a loaded `Protocol`.

`cargo xtask build-wasm` builds all three variants into
`docs/.webui-press/public/wasm/{all,handler,parser}/` with stable `wasm-pack`
output names.

### npm Distribution

The `@microsoft/webui` npm package follows the esbuild single-package model:
- `bin: { "webui": "bin/webui" }` — CLI binary via platform-specific `optionalDependencies`
- `exports["."]` points to the compiled `dist/index.js` programmatic API, which
  loads the platform native addon directly
- `Protocol` is the only runtime rendering API; construction decodes and
  indexes a protocol `Buffer` once and binds the selected plugin
- callers own the lifecycle explicitly, so the package has no hidden
  `WeakMap`, no protocol-sized mutation snapshot, and no byte-per-call render
  functions
- `Protocol.render()` returns the buffered-string result;
  `Protocol.renderStream()` batches callbacks with a 16 KiB target instead of
  crossing into JavaScript for every internal handler fragment
- render currently requires the native addon; no WASM render fallback is wired

### .NET / NuGet Distribution

The `Microsoft.WebUI` package is the managed .NET binding for `webui-ffi`. It targets `net8.0` and `net9.0`, packs `dotnet/src/Microsoft.WebUI/README.md`, and publishes XML documentation generated from public API comments.

`Protocol` is a public `IDisposable` type backed by a native
`SafeHandle`. Applications create one from `protocol.bin` at startup and pass it
to `WebUIHandler.Render`. Partial navigation, component-template loading, and
token queries are protocol-owned operations exposed as `RenderPartial`,
`RenderComponentTemplates`, and `Tokens`. The type is thread-safe and releases
both decoded protocol data and reusable indices on dispose.

Native assets are split into `Microsoft.WebUI.Runtime.<rid>` packages for each supported RID. The runtime packages share `dotnet/runtime/README.md`, include NuGet release notes pointing to the GitHub release notes, and carry the matching `runtimes/<rid>/native` asset. The managed package references every runtime package so NuGet restores them transitively; .NET then resolves `webui_ffi` from the matching native asset. `WEBUI_LIB_PATH` remains the override for custom local native builds.

`dotnet/Directory.Build.props` applies NuGet metadata to packable .NET projects: `Authors=Microsoft`, `PackageOwners=Microsoft`, a package license URL with `PackageRequireLicenseAcceptance=true`, project and repository URLs, Source Link, release notes links, discoverability tags, the required `© Microsoft Corporation. All rights reserved.` copyright notice, and `.snupkg` symbol package generation. `cargo xtask publish` runs `dotnet pack` on `dotnet/Microsoft.WebUI.sln` and stages both `.nupkg` and `.snupkg` files under `publish/nuget`.

NuGet publishing is not automated by ESRP today. Release workflows attach staged NuGet artifacts to GitHub Releases for manual/externally tracked nuget.org publishing. Before nuget.org publishing, ownership must be limited to the approved Microsoft package owner/co-owner accounts, every Authenticode-signable file in the package must be signed, and each `.nupkg` must be signed with the Microsoft certificate through the approved signing process.

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
| `webui_handler_create()` | Create a reusable handler (no plugin). |
| `webui_handler_create_with_plugin(plugin_id)` | Create a handler with a named plugin. Returns `NULL` on error. Refer to the CLI/crate docs for the current list of plugin identifiers. |
| `webui_protocol_create(data, len)` | Decode and index a protocol once. Returns a thread-safe opaque handle. |
| `webui_protocol_destroy(protocol)` | Destroy a loaded protocol handle. `NULL` is a safe no-op. |
| `webui_handler_render(handler, protocol, json, entry_id, request_path)` | Render a loaded protocol with route matching. `request_path` controls which route is active. Returns a heap-allocated string. |
| `webui_protocol_render_partial(protocol, state_json, entry_id, request_path, inventory_hex)` | Produce a complete JSON partial response with active-route projected state. |
| `webui_protocol_render_component_templates(protocol, tags_json, inventory_hex)` | Return requested component template payloads and updated inventory. |
| `webui_protocol_tokens(protocol)` | Return newline-delimited CSS token names. |
| `webui_handler_destroy(handler)` | Destroy a handler. `NULL` is a safe no-op. |
| `webui_free(ptr)` | Free a string returned by any render function. `NULL` is a safe no-op. |
| `webui_last_error()` | Return per-thread error message. Caller must **not** free. |

The C ABI uses a typed opaque `webui_protocol_t *` with explicit
`webui_protocol_create` / `webui_protocol_destroy` ownership because C has no
portable object constructor or RAII lifetime. Automatically caching raw
`(pointer, length)` inputs would be unsound: the caller may mutate, move, or
free the bytes, pointer identity is not content identity, hashing on every
request is O(protocol size), and a global copied cache would require arbitrary
memory and eviction policy. Higher-level bindings wrap this native lifetime in
their normal `Protocol` object (`IDisposable` / garbage-collected class).

### Error Model
Thread-local error storage following the POSIX `dlerror()` pattern. After any
function returns `NULL`, call `webui_last_error()` for a human-readable diagnostic.

## CLI Tool (webui-cli)

The CLI specification and usage details are maintained in [crates/webui-cli/README.md](crates/webui-cli/README.md).

## Example Workflow

Examples and end-to-end walkthroughs are maintained in [examples/README.md](examples/README.md)
