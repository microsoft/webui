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
- Buffer consolidation for reduced allocations
- Strict context isolation during processing
- Proactive error handling with actionable messages

## Protocol Specification (webui-protocol)
The protocol defines the serializable structure representing UI templates.

### Data Types
```rust
/// The root protocol structure representing a complete webpage configuration.
pub struct WebUIProtocol {
    /// Map of stream identifiers to their associated streams.
    pub streams: WebUIStreamRecords,
}

/// A mapping of unique stream identifiers to their corresponding stream vectors.
pub type WebUIStreamRecords = HashMap<String, Vec<WebUIStream>>;

/// Defines the various types of streams in the WebUI protocol.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WebUIStream {
    /// Outputs static content.
    Raw(WebUIStreamRaw),
    /// A reusable component with styling.
    Component(WebUIStreamComponent),
    /// Iterates over a collection to generate repeated content.
    For(WebUIStreamFor),
    /// Connects dynamic content via signals.
    Signal(WebUIStreamSignal),
    /// Renders content conditionally.
    If(WebUIStreamIf),
    /// Represents a boolean attribute (e.g., disabled, checked).
    BooleanAttribute(WebUIStreamBooleanAttribute),
}
```
### Stream Types
#### Raw Stream
```rust
pub struct WebUIStreamRaw {
    /// The content to render.
    pub value: String,
}
```
#### Component Stream
```rust
pub struct WebUIStreamComponent {
    /// The identifier for the associated stream record.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}
```
#### For Loop Stream
```rust
pub struct WebUIStreamFor {
    /// The name representing a singular item (e.g., "person").
    pub item: String,
    /// The collection name (e.g., "people").
    pub collection: String,
    /// The identifier for the stream to render for each item.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}
```
#### Signal Stream
```rust
pub struct WebUIStreamSignal {
    /// The value or identifier of the signal.
    pub value: String,
    /// Determines if the value should be rendered as raw content.
    #[serde(default)]
    pub raw: bool,
}
```
#### Conditional Stream
```rust
pub struct WebUIStreamIf {
    /// The condition expression to evaluate.
    pub condition: ConditionExpr,
    /// The identifier for the stream record to render if true.
    #[serde(rename = "streamId")]
    pub stream_id: String,
}
```
#### Boolean Attribute Stream
```rust
/// Represents a boolean attribute on an element.
/// Boolean attributes must start with '?' (e.g., ?disabled).
pub struct WebUIStreamBooleanAttribute {
    /// The boolean attribute name (e.g., "disabled").
    pub name: String,
    /// The attribute value, if false, attribute is ignored.
    #[serde(default)]
    pub value: bool,
}
```
#### Condition Expressions
```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConditionExpr {
    /// A simple predicate condition.
    Predicate(Predicate),
    /// A negation of a condition expression.
    Not(Box<ConditionExpr>),
    /// A compound condition combining two expressions with a logical operator.
    Compound {
        /// The left-hand side condition.
        left: Box<ConditionExpr>,
        /// The logical operator (And or Or).
        op: LogicalOperator,
        /// The right-hand side condition.
        right: Box<ConditionExpr>,
    },
    /// An identifier condition, single variable.
    Identifier {
        /// The identifier to evaluate.
        value: String,
    },
}
```
#### Operators
```rust
/// Logical operators for compound conditions.
#[derive(Clone, Serialize, Deserialize)]
pub enum LogicalOperator {
    /// Represents a logical AND.
    And,
    /// Represents a logical OR.
    Or,
}

/// Comparison operators for predicates.
#[derive(Clone, Serialize, Deserialize)]
pub enum ComparisonOperator {
    GreaterThan,         // >
    LessThan,            // <
    Equal,               // ==
    NotEqual,            // !=
    GreaterThanOrEqual,  // >=
    LessThanOrEqual,     // <=
}
```
#### Predicates
```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct Predicate {
    /// The left-hand side value.
    pub left: String,
    /// The operator used in comparison.
    pub operator: ComparisonOperator,
    /// The right-hand side value.
    pub right: String,
}
```
#### Serialization Requirements
- JSON serialization/deserialization with proper error handling
- Support for custom error types and propagation
- Validation of protocol structure during deserialization
- Performance optimizations for large protocol structures
- Support for stream reference validation
- Attribute names starting with '?' are treated as boolean attributes using the `BooleanAttribute` stream type. The attribute is rendered only if the value/expression evaluates to true.

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
### Stream Processing
- **Raw streams:** Write value directly to output
- **Signal streams:**
  - Resolve value from state using `find_value_by_dotted_path`
  - Escape value if `raw` is false, otherwise write as-is
- **Boolean attribute streams:**
  - Evaluate the value; if true, render the attribute name.
  - If false, omit the attribute.
- **If streams:**
  - Evaluate condition using `evaluate`
  - If true, process referenced stream
  - Track false conditions for template generation
  - When the `If` streams are enclosed in one or more `For` streams it can access the states of those `For` streams'
    current item thorugh their corresponding item monikers. It can also access global state.
  - `If` stream conditions can have tokens from different state objects i.e. local states from enclosing `For` stream
    items and/or the global state mixed in the condition expression.
- **For streams:**
  - Iterate over collection from state
  - Process referenced stream for each item with current item's state accessible thorugh a moniker and the global state
    as a fallback.
- **Component streams:** Process referenced stream directly. `Component` streams enclosed in a For stream has access to
    the fields of the current item being looped over and the global state. The `Component` stream doesn't need to use
    the `For` stream item moniker and can access the fields without the qualification. If the `Component` stream is
    nested in multiple `For` streams only the closest enclosing `For` stream item's state is accessible to it.

### State Management
- Global state refers to the global application state that is available to all streams at all times.
- Local state refers to the state corresponding to the current item being looped over in a `For` stream.
- When nested `For` streams are present local state of the current item being looped over for any of the `For` stream in the
  hierarchy can be accessed through the corresponding item moniker with an exception for `Component` streams.
- For `Component` streams only the closest enclosing `For` stream's current item state is available and can be accessed
  directly without the item moniker qualification. `Component` streams also have access to the global application state.

### Error Handling
- Report missing stream references
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
pub fn parse(&mut self, stream_id: &str, html_content: &str) -> Result<(), ParserError>
pub fn into_stream_records(self) -> WebUIStreamRecords
```
#### Content Processing

##### Raw Content
- Buffer content until directive or signal encountered
- Consolidate adjacent raw content
- Flush buffer when transitioning to non-raw content

##### Directive Processing
- **<for>:** Extract item/collection pair and process children into separate stream
- **<if>:** Extract and parse condition, process children into separate stream
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
    pub fn parse(&self, text: &str) -> Result<Vec<WebUIStream>, ParserError>
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
    pub fn parse(&mut self, css_content: &str) -> Result<WebUIStreamRecords, ParserError>
    pub fn process_css(&mut self, css_content: &str, streams: &mut WebUIStreamRecords) -> Result<(), ParserError>
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
│   ├── webui-expressions/    # Expression evaluation engine
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

## Example Workflow
```
HTML Template → HTML Parser → Protocol → Handler + State → Rendered HTML
```

### Input Template
```html
Hello, WebUI!
<for each="person in people">
    <person-card ?disabled="{{person.isInactive}}">{{person.name}}</person-card>
</for>
{{{raw_description}}}
<if condition="contact">
    Hello, {{name}}
</if>
```
### Generated Protocol
```json
{
    "streams": {
        "index.html": [
            { "type": "raw", "value": "Hello, WebUI!\n" },
            {
                "type": "for",
                "item": "person",
                "collection": "people",
                "streamId": "for-1"
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
                "streamId": "if-1"
            }
        ],
        "for-1": [
            {
                "type": "raw",
                "value": "<person-card"
            },
            {
                "type": "booleanAttribute",
                "name": "disabled",
                "value": "{{person.isInactive}}"
            },
            {
                "type": "raw",
                "value": ">"
            },
            {
                "type": "component",
                "streamId": "person_card",
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
