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
### Core Function
```rust
pub fn handler(
    protocol: &WebUIProtocol,
    state: &Value,
    writer: impl Writer
) -> Result<(), HandlerError>
```
### Writer Interface
```rust
pub trait Writer {
    fn write(&mut self, content: &str) -> Result<(), io::Error>;
    fn end(&mut self) -> Result<(), io::Error>;
}
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

### HTML Parser
```rust
pub struct HtmlParser {
    component_registry: ComponentRegistry,
    css_parser: CssParser,
    condition_parser: ConditionParser,
    handlebars_parser: HandlebarsParser,
    // Other fields...
}
```
#### Primary Method
```rust
pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<(), ParserError>
pub fn into_fragment_records(self) -> WebUIFragmentRecords
```
#### Content Processing

##### Raw Content
- Buffer content until directive or signal encountered
- Consolidate adjacent raw content
- Flush buffer when transitioning to non-raw content

##### Directive Processing
- **<for>:** Extract item/collection pair and process children into separate fragment
- **<if>:** Extract and parse condition, process children into separate fragment
- **Components:** Check component registry, process as component if found

##### Element Processing
- Maintain proper tag structure
- Process children recursively (iterative implementation)
- Handle attributes and special elements

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
}
```

#### Requirements
- Process CSS variables
- Extract dynamic variables with --webui- prefix
- Convert dynamic variables to signals
- Handle nested variable references
- Process inline and external CSS

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
│   ├── webui-expressions/    # Expression evaluation engine
│   ├── webui-ffi/            # C-compatible FFI bindings
│   ├── webui-handler/        # Protocol handler implementation
│   ├── webui-parser/         # HTML/CSS/template parser
│   ├── webui-protocol/       # Protocol definition
│   ├── webui-state/          # State management
│   └── webui-test-utils/     # Testing utilities
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

## CLI Tool (webui-cli)

The `webui` CLI provides the developer-facing build toolchain for WebUI applications.

### Binary Name
`webui` (configured via `[[bin]]` in `crates/webui-cli/Cargo.toml`)

### Subcommands

#### `webui build`
Builds a WebUI application from an app folder into the protocol format.

```bash
webui build [APP] --out <OUT> [--entry <FILE>]
```

**Arguments:**
- `APP` — Path to the app folder (defaults to current directory `.`)
- `--out <OUT>` — Output folder for protocol and assets (required)
- `--entry <FILE>` — Entry HTML file name (defaults to `index.html`)

#### `webui inspect`
Inspects a `protocol.bin` file by converting it to JSON and printing to stdout. Useful for debugging and piping to `jq`.

```bash
webui inspect <FILE>
```

**Arguments:**
- `FILE` — Path to a `protocol.bin` file

### Build Pipeline
1. Resolve and validate the app folder path
2. Create the output directory
3. Initialize `HtmlParser` and register components from the app folder via `ComponentRegistry::register_from_paths()`
4. Read the entry HTML file (default `index.html`) from the app folder
5. Parse HTML into `WebUIFragmentRecords` via `HtmlParser::parse()`
6. Wrap fragments in `WebUIProtocol` and serialize to protobuf binary
7. Write `protocol.bin` to the output folder
8. Copy each discovered component's CSS file (e.g., `my-card.css`) to the output folder

### Dependencies
- `webui-parser` — For `HtmlParser` and `ComponentRegistry`
- `webui-protocol` — For `WebUIProtocol` serialization
- `clap` — CLI argument parsing (derive mode)
- `console` — Colored terminal output
- `anyhow` — Error handling with context

### Component Discovery
The CLI uses `ComponentRegistry::register_from_paths()` which:
- Recursively walks the app directory using `walkdir`
- Identifies component files as `.html` files whose name contains a hyphen (e.g., `my-card.html`)
- Automatically pairs with optional `.css` files of the same name (e.g., `my-card.css`)
- Registers components by tag name (the file stem, e.g., `my-card`)

### App Folder Convention
```
my-app/
├── index.html          # Entry template (configurable via --entry)
├── my-card.html        # Component: <my-card>
├── my-card.css         # Component styles (auto-discovered)
├── nav-bar.html        # Component: <nav-bar>
├── styles.css          # Global styles (not auto-processed)
└── data.json           # Sample data (not used by CLI)
```

### Output Structure
```
out/
├── protocol.bin        # Serialized WebUIProtocol (protobuf binary)
├── my-card.css         # Copied component CSS
└── nav-bar.css         # Copied component CSS (if exists)
```

### Error Handling
- Missing app folder → actionable error with path shown
- Missing entry file → error with hint to use `--entry` flag
- Parse failures → propagated with context from `webui-parser`
- Write failures → propagated with file path context

## Example Workflow
```
HTML Template → HTML Parser → Protocol → Handler + State → Rendered HTML
```

### Input Template
```html
Hello, WebUI!
<for each="person in people">
    <person-card ?disabled={{person.isInactive}}>{{person.name}}</person-card>
</for>
{{{raw_description}}}
<if condition="contact">
    Hello, {{name}}
</if>
```
### Generated Protocol
> Note: Shown as JSON for readability; the actual output is stored as protobuf binary.
```json
{
    "fragments": {
        "index.html": [
            { "type": "raw", "value": "Hello, WebUI!\n" },
            {
                "type": "for",
                "item": "person",
                "collection": "people",
                "fragmentId": "for-1"
            },
            {
                "type": "signal",
                "value": "raw_description",
                "raw": true
            },
            {
                "type": "if",
                "condition": {
                    "kind": "identifier",
                    "value": "contact"
                },
                "fragmentId": "if-1"
            }
        ],
        "for-1": [
            {
                "type": "raw",
                "value": "<person-card"
            },
            {
                "type": "attribute",
                "name": "disabled",
                "conditionTree": {
                    "type": "identifier",
                    "value": "person.isInactive"
                }
            },
            {
                "type": "raw",
                "value": ">"
            },
            {
                "type": "component",
                "fragmentId": "person_card",
                "raw": false
            },
            {
                "type": "signal",
                "value": "person.name",
                "raw": false
            },
            {
                "type": "raw",
                "value": "</person-card>"
            }
        ],
        "if-1": [
            {
                "type": "raw",
                "value": "Hello, "
            },
            {
                "type": "signal",
                "value": "name"
            }
        ],
        "person-card": [
            {
                "type": "raw",
                "value": "<p><slot></slot></p>"
            }
        ]
    }
}
```
### Handler State
```json
{
    people: [
        {
            name: "Ali",
            isInactive: true
        },
        {
            name: "Amanda",
            isInactive: false
        }
    ],
    raw_description: "<b>YES!</b>",
    name: "Mohamed Mansour"
}
```

### Handler Output
```html
Hello, WebUI!
<person-card disabled>
    <template shadowrootmode="open">
        <p><slot></slot></p>
    </template>
    Ali
</person-card>
<person-card>
    <template shadowrootmode="open">
        <p><slot></slot></p>
    </template>
    Amanda
</person-card>
<b>YES!</b>
<template id="for-1"><person-card>{{person.name}}</person-card></template>
<template id="if-1">Hello, {{name}}</template>
```
