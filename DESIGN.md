# WebUI Framework Technical Specification

## Overview
WebUI Framework is a high-performance server-side rendering framework that operates without JavaScript runtimes. It separates static and dynamic content at build time, creating an efficient protocol that enables fast server-side rendering in any language (Rust, Go, C#, PHP, Ruby, etc.).

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

#### Plugin Fragment
Plugin fragments carry opaque data from parser plugins to handler plugins. WebUI does
not interpret this data — each parser/handler plugin pair defines its own binary contract.
```rust
pub struct WebUIFragmentPlugin {
    /// Opaque plugin-specific binary data.
    pub data: Vec<u8>,
}
```

**Attribute types:**
- **Simple dynamic:** `href="{{url}}"` → `{ name: "href", value: "url" }`
- **Boolean (`?` prefix):** `?disabled={{isDisabled}}` → `{ name: "disabled", condition_tree: identifier("isDisabled") }` — rendered only if condition is truthy; silently dropped if value is not a pure handlebars expression.
- **Complex (`:` prefix):** `:config="{{settings}}"` → `{ name: ":config", value: "settings", complex: true }`
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
The `find_value_by_dotted_path` function provides a high-performance way to query JSON state:
```rust
pub fn find_value_by_dotted_path(path: &str, state: &Value) -> Result<Value, StateError>
```
### Requirements
- Dot notation support (e.g., user.profile.name)
- Array indexing support (e.g., users.0.name)
- Special length property support for arrays (e.g., users.length)
- Nullable path handling
- Missing path error reporting

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

impl WebUIHandler {
    pub fn new() -> Self;
    pub fn with_plugin(plugin: Box<dyn HandlerPlugin>) -> Self;

    pub fn handle(
        &mut self,
        protocol: &WebUIProtocol,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```
### Writer Interface
```rust
pub trait ResponseWriter {
    /// Write content to the output
    fn write(&mut self, content: &str) -> Result<()>;
    /// Finalize the output
    fn end(&mut self) -> Result<()>;
}
```

### Handler Plugin System
The handler supports a framework-agnostic plugin system. Plugins receive lifecycle
callbacks during rendering and can inject arbitrary content. WebUI does not interpret
what plugins write — each framework defines its own marker format.

```rust
pub trait HandlerPlugin {
    fn push_scope(&mut self);
    fn pop_scope(&mut self);
    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_start(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;
    fn on_plugin_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;
}
```

**Hook invocation points:**
- **Signal**: `on_binding_start` before, `on_binding_end` after (same scope)
- **For loop**: `on_binding_start/end` around entire loop; `on_repeat_item_start/end` + `push_scope/pop_scope` per item
- **If condition**: `on_binding_start/end` around condition; `push_scope/pop_scope` if condition is true
- **Component**: `push_scope/pop_scope` around component body
- **Plugin fragment**: `on_plugin_data` with opaque bytes from protocol

**Built-in plugin: `FastHydrationPlugin`**
Injects FAST-HTML hydration comment markers for client-side re-hydration:
- Binding: `<!--fe-b$$start$$INDEX$$NAME$$fe-b-->` / `<!--fe-b$$end$$INDEX$$NAME$$fe-b-->`
- Repeat: `<!--fe-repeat$$start$$INDEX$$fe-repeat-->` / `<!--fe-repeat$$end$$INDEX$$fe-repeat-->`
- Attribute (single): ` data-fe-b-INDEX`
- Attribute (multi): ` data-fe-c-INDEX-COUNT`

**Usage:**
```rust
let mut handler = WebUIHandler::with_plugin(Box::new(FastHydrationPlugin::new()));
handler.handle(&protocol, &state, &mut writer)?;
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
  - **Complex (with `complex: true`):** Same as simple dynamic but for `:` prefixed pass-through attributes.
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
- **Plugin fragments:** Pass opaque `data` bytes to the handler plugin's `on_plugin_data` hook. Skipped silently when no plugin is configured.

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
- **Symlink escape**: Resolved package paths must remain within `node_modules/` after `fs::canonicalize()`
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
    /// Emit `<link rel="stylesheet" href="./component.css">` tags (default).
    External,
    /// Embed CSS content inline in `<style>` tags within the shadow DOM template.
    Inline,
}
```

- **External** (default): Emits `<link>` tags referencing external `.css` files. Used by the CLI for production builds where CSS files are served separately.
- **Inline**: Embeds the full CSS content in `<style>` tags inside the shadow DOM template. Used when all files are needed in-memory.

Set via `parser.set_css_strategy(CssStrategy::Inline)`.

#### Primary Method
```rust
pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<(), ParserError>
pub fn into_fragment_records(self) -> WebUIFragmentRecords
```

### Parser Plugin System
The parser supports a framework-agnostic plugin system. Plugins customize parsing
behavior for framework-specific needs (component discovery, attribute filtering,
hydration data emission) without WebUI knowing framework internals.

```rust
pub trait ParserPlugin {
    fn on_parse_component(&mut self, tag_name: &str, component: &Component) -> Result<()>;
    fn should_skip_attribute(&self, attr_name: &str) -> bool;
    fn on_body_end(&mut self) -> Option<String>;
    fn on_element_parsed(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;
}
```

**Hook invocation points:**
- **Attribute loop**: `should_skip_attribute` called per attribute; skipped attrs are not parsed
- **Element completion**: `on_element_parsed` called with binding count after all attrs processed; returned bytes emitted as `Plugin` fragment
- **Component encounter**: `on_parse_component` called when a custom element is found
- **Body end**: `on_body_end` called before `body_end` signal; returned HTML injected as raw fragment

**Built-in plugin: `FastParserPlugin`**
- Skips FAST-specific runtime attributes (`@click`, `f-ref`, `f-slotted`, `f-children`)
- Emits `Plugin` fragments with u32 LE attribute binding counts
- Tracks components and injects `<f-template>` wrappers at body end
- Converts BTR syntax to FAST syntax: `<if>`→`<f-when>`, `<for>`→`<f-repeat>`, `{{expr}}`→`{expr}` in `:attr` values

**Usage:**
```rust
let mut parser = HtmlParser::with_plugin(Box::new(FastParserPlugin::new()));
parser.parse("index.html", &html)?;
```

**CLI integration:**
```bash
webui build ./templates --out ./dist --plugin=fast
webui start ./templates --state ./data/state.json --plugin=fast
```
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
│   ├── webui-cli/            # CLI build tool (binary: "webui")
│   ├── webui-discovery/      # External component discovery (npm, paths)
│   ├── webui-expressions/    # Expression evaluation engine
│   ├── webui-ffi/            # C-compatible FFI bindings
│   ├── webui-handler/        # Protocol handler implementation
│   ├── webui-node/           # Node.js native addon (napi-rs)
│   ├── webui-parser/         # HTML/CSS/template parser
│   ├── webui-protocol/       # Protocol definition
│   ├── webui-state/          # State management
│   ├── webui-test-utils/     # Testing utilities
│   └── webui-wasm/           # WebAssembly bindings
├── examples/                 # Example applications
├── docs/                     # Documentation
├── tests/                    # Integration tests
└── benchmarks/               # Performance benchmarks
```
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
| `webui_render(html, data_json)` | Parse + render in one call. Returns heap-allocated string (caller frees with `webui_free`). |
| `webui_handler_create()` | Create a reusable handler (no plugin). |
| `webui_handler_create_with_plugin(plugin_id)` | Create a handler with a named plugin (e.g. `"fast"`). Returns `NULL` on error. |
| `webui_handler_render(handler, data, len, json)` | Render a pre-compiled protocol. Returns heap-allocated string. |
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
