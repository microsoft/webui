
# Feature Gap Analysis: Node `generator.test.js` vs Rust `webui-parser`

## Problem Statement

The Node.js implementation in `~/edge/src/edge_webui/packages/build/src/btr/generator.test.js` (1117 lines, ~55 test cases) is the reference/complete implementation. The Rust port in `crates/webui-parser` is incomplete. This document catalogs every feature present in the Node tests that is missing or incomplete in the Rust parser.

## Summary of Findings

The Rust `webui-parser` currently implements:
- ✅ Basic raw text/HTML passthrough
- ✅ `{{signal}}` double-brace signals in text nodes
- ✅ `{{{raw}}}` triple-brace raw signals in text nodes
- ✅ `<for each="item in items">` directive
- ✅ `<if condition="...">` directive
- ✅ Nested `<for>` and `<if>` directives
- ✅ Basic component registration and rendering
- ✅ Component slot content (basic)
- ✅ CSS parsing (placeholder/stub only)
- ✅ Condition expression parsing (identifiers, predicates, compound, negation)

---

## Missing Features (Ordered by Priority)

### 1. ~~🔴 Attribute Fragment Type (Protocol + Parser)~~ ✅ DONE

**Status:** Implemented in branch `mmansour/attribute-fragments`

Added `WebUIFragmentAttribute` message to `webui.proto` with fields: `name`, `value`, `template`, `complex`, `attr_start`, `attr_skip`, `raw_value`, `condition_tree`. Added constructors `attribute()`, `attribute_template()`, `attribute_complex()`, `attribute_boolean()` to protocol. Implemented attribute-aware element parsing in `HtmlParser::process_regular_element()` that processes each attribute individually instead of dumping the entire opening tag as raw content.

---

### 2. ~~🔴 Boolean Attributes (`?` prefix)~~ ✅ DONE

**Status:** Implemented in branch `mmansour/attribute-fragments`

Parser detects `?`-prefixed attributes, extracts the handlebars signal name, creates `ConditionExpr::identifier()`, and emits `attribute_boolean` fragments. Silently drops boolean attributes without pure handlebars values (plain text, mixed content). Supports dotted paths (e.g., `layout.isPinned`). 14 ported tests pass.

---

### 3. ~~🔴 Colon-Prefixed Complex Attributes (`:` prefix)~~ ✅ DONE

**Status:** Implemented in branch `mmansour/attribute-fragments`

Parser detects `:`-prefixed attributes, extracts the handlebars signal name, and emits `attribute_complex` fragments with `complex: true` and the `:` preserved in the name.

---

### 4. ~~🔴 Mixed/Template Attributes (static + dynamic content)~~ ✅ DONE

**Status:** Implemented in branch `mmansour/attribute-fragments`

Parser detects mixed static+dynamic attribute values (e.g., `value="hello {{world}}"`), creates a sub-stream with unique `attr-N` ID containing the parsed handlebars fragments, and emits `attribute_template` fragments referencing the sub-stream.

---

### 5. 🔴 `<f-signal>` Transient Element

**Node behavior:** `<f-signal value="name">default</f-signal>` produces a signal fragment with an optional `defaultValue`.

**Test cases affected:**
- `should process signal attribute correctly with default value` — `<f-signal value="testSignal">Hi</f-signal>` → `{ type: 'signal', value: 'testSignal', defaultValue: 'Hi' }`
- `should process transient node f-signal` — `<f-signal value="name"></f-signal><f-signal value="age"/>` → two signal fragments

**What's needed:**
1. Add `default_value` field to `WebUIFragmentSignal` in proto
2. Detect `<f-signal>` elements in the parser
3. Extract `value` attribute and text content as default value
4. Handle self-closing `<f-signal value="age"/>` form

---

### 6. 🔴 Body Signals (`body_start` / `body_end`)

**Node behavior:** The parser injects `{ type: 'signal', value: 'body_start', raw: true }` and `{ type: 'signal', value: 'body_end', raw: true }` signals around `<body>` content.

**Test cases affected:**
- `should inject body_start and body_end signal immediately before </body> tag`
- `should process a complex raw text` (includes body signals)

**What's needed:**
1. Detect `<body>` element during parsing
2. Emit `body_start` signal immediately after `<body>` opening tag
3. Emit `body_end` signal immediately before `</body>` closing tag

---

### 7. 🔴 Skipped Attribute Handling (`attrSkip`, `attrStart`)

**Node behavior:** Component attributes matching specific names (`class`, `style`, `role`) or prefixes (`data-`, `aria-`) are marked with `attrSkip: true`. The first dynamic attribute on a component gets `attrStart: true`.

**Test cases affected:**
- `should set attrSkip for skipped component attributes`
- `should mark leading boolean component attribute with attrStart`
- Component slot tests with `attrStart` and `rawValue` markers

**What's needed:**
1. Implement skipped attribute constants: `SkippedAttributes = ['class', 'style', 'role']`, `SkippedAttributePrefixes = ['data-', 'aria-']`
2. When processing component attributes, mark matching ones with `attrSkip: true`
3. Mark the first dynamic attribute with `attrStart: true`

---

### 8. 🔴 Component Attribute Passthrough (`rawValue`)

**Node behavior:** When a component has regular (non-dynamic) attributes, they produce attribute fragments with `rawValue: true` (static value passed through as-is).

**Test cases affected:**
- `should process available web components with slots` — `appearance="subtle"` → `{ type: 'attribute', name: 'appearance', value: 'subtle', attrStart: true, rawValue: true }`
- `should mark leading boolean component attribute with attrStart` — `title="Hello"` → `{ type: 'attribute', name: 'title', value: 'Hello', rawValue: true }`

**What's needed:**
1. During component processing, emit attribute fragments for all non-skipped attributes
2. Static attributes get `rawValue: true`
3. Dynamic attributes get their signal value

---

### 9. 🔴 Custom `template` Attribute on `<for>`

**Node behavior:** `<for each="item in items" template="static">` uses the provided template name instead of auto-generating `for-N`.

**Test cases affected:**
- `should process transient node for with template` — `template="static"` → uses `'static'` as stream ID
- `should process recursive transient nodes` — self-referencing template

**What's needed:**
1. Check for `template` attribute on `<for>` elements
2. If present, use its value as the fragment ID instead of generating one

---

### 10. 🔴 Empty `<for>` Handling

**Node behavior:** `<for each="item in items"></for>` with no body produces no fragment at all — the surrounding content merges.

**Test cases affected:**
- `should process nothing with empty for stream` — `<div><for each="item in items"></for></div>` → just `[raw('<div></div>')]`

**What's needed:**
1. After processing `<for>` body, if fragment list is empty, skip adding the for fragment entirely
2. Continue accumulating raw content as if the `<for>` wasn't there

---

### 11. 🔴 Stripping Special Attributes from Templates (`@`, `:`, `?`)

**Node behavior:** In component templates, attributes starting with `@`, `:`, or `?` are stripped from the rendered output.

**Test cases affected:**
- `should strip attributes that have this character starting "@" or ":" or "?" from the template` — `<template @click={foo} :bar="baz" ?bool="true">` → `<template>`

**What's needed:**
1. When parsing component templates, detect and remove attributes prefixed with `@`, `:`, or `?`
2. These are runtime-only bindings that shouldn't appear in the static output

---

### 12. 🔴 Component Template Wrapping Logic

**Node behavior:** Components that already have a `<template>` element in their content don't get double-wrapped. Components without one get wrapped in `<template shadowrootmode="open">...</template>`.

**Test cases affected:**
- `should not wrap component with template if it already has a template element`
- `should not wrap styled component with template if it already has a template element`

**Current Rust behavior:** Always wraps with `<template shadowrootmode="open">`.

**What's needed:**
1. Before wrapping, check if component HTML already starts with `<template`
2. If it does, skip the automatic wrapping
3. For styled components with existing template, inject the `<link>` inside the existing template

---

### 13. 🔴 CSS Hoisting / Component Styles

**Node behavior:** Components with `styles` field get a `<link rel="stylesheet" href="./custom.css">` injected inside their template.

**Test cases affected:**
- `should process available web components` — component with `styles: 'custom.css'` → link tag in template
- `should process available web components correctly` (with tokens)
- `should not wrap styled component with template if it already has a template element`

**Current Rust behavior:** Component `css_content` is stored but never used in template rendering.

**What's needed:**
1. When rendering a component template, if it has styles, inject `<link rel="stylesheet" href="./{styles}">` at the beginning of the template content
2. Support `cssHoisting: 'legacy'` option

---

### 14. 🔴 Token Extraction from Styles

**Node behavior:** CSS `var(--tokenName)` references are extracted into a `tokens` array on the protocol output.

**Test cases affected:**
- `should extract tokens from style tags` — extracts `['borderRadiusSmall', 'colorBrandBackground', 'lineHeightBase400']`
- `should process available web components correctly` — merges component tokens

**What's needed:**
1. Parse CSS `var()` function calls in style content
2. Extract token names (strip `--` prefix and convert)
3. Collect into a `tokens` array on the output

---

### 15. 🔴 Token Signal in Style Comments

**Node behavior:** CSS comments like `/*{{tokens.light}}*/` are parsed as signals with default values derived from component tokens.

**Test cases affected:**
- `should process available web components` — `/*{{tokens.light}}*/` → signal with defaultValue based on component tokens
- `should process available web components correctly` — empty default when no matching tokens

**What's needed:**
1. Detect `/*{{...}}*/` patterns in style content
2. Generate signal fragments with token-derived default values
3. Resolve token values from component registry

---

### 16. 🔴 Self-Closing Tag Handling

**Node behavior:** Self-closing tags (`<img/>`, `<br/>`, `<input/>`) are handled correctly with proper attribute processing.

**Test cases affected:**
- Multiple self-closing tag tests (sequence, whitespace variations, deeply nested, etc.)
- `should handle self-closing custom elements` — component with self-closing syntax
- `should differentiate between self-closing and empty regular tags`

**Current Rust behavior:** The parser uses tree-sitter which handles self-closing tags via `self_closing_tag` node type, but the current code only handles `start_tag`/`end_tag` patterns.

**What's needed:**
1. Handle `self_closing_tag` tree-sitter node type
2. Properly process attributes on self-closing tags
3. Render with `/>` suffix instead of separate closing tag

---

### 17. 🔴 HTML5 Void Elements

**Node behavior:** HTML5 void elements (`<img>`, `<br>`, `<hr>`, `<input>`, `<meta>`, `<link>`) are handled without requiring closing tags.

**Test cases affected:**
- `should handle HTML5 void elements correctly`
- `should process elements with children having self-closing tags correctly` (SVG `<path/>`)

**What's needed:**
1. Maintain a list of HTML5 void elements
2. Don't expect/emit closing tags for void elements

---

### 18. 🔴 Hydration Attributes

**Node behavior:** With `templateType: 'fast-html'` option, elements with specific attributes (`f-ref`, `@click`) produce hydration fragments.

**Test cases affected:**
- `should render hydration attributes correctly for option template fast-html`

**What's needed:**
1. Add `WebUIFragmentHydration` message to proto with `attributeCount` field
2. Implement hydration detection for `f-ref` and `@`-prefixed event attributes
3. Support `templateType` option in parser

---

### 19. 🔴 `estimatedBufferSize` Field

**Node behavior:** The protocol output includes `estimatedBufferSize` — a pre-calculated estimate of the final rendered output size.

**Test cases affected:**
- `should have estimatedBufferSize field set correctly`

**What's needed:**
1. Track cumulative raw content size during parsing
2. Add `estimated_buffer_size` field to protocol output

---

### 20. 🟡 Handlebars Edge Cases

**Node behavior:** Various edge cases for invalid/malformed handlebars expressions.

**Test cases affected:**
- `should not process handlebars when invalid` — `{{{invalid}}` → raw text
- `should not process handlebars when invalid since triple exists` — `{{{{invalid}}` → raw text
- `should not process handlebars when invalid but with valid triple` — `{{{{{invalid}}` → raw `{{{` + signal
- `should process entities correctly` — `Hello&#125;World` preserved as-is

**Current Rust behavior:** The handlebars parser may not handle all these edge cases correctly.

**What's needed:**
1. Verify and fix edge case handling for malformed brace patterns
2. Ensure HTML entities are preserved in raw output

---

### 21. 🟡 Whitespace Handling / Content Normalization

**Node behavior:** The Node implementation normalizes whitespace in HTML output (collapses whitespace between elements, trims insignificant whitespace).

**Test cases affected:**
- Most tests show normalized output (no extra whitespace between tags)

**Current Rust behavior:** The parser preserves all whitespace from the source, including leading/trailing whitespace in text nodes (which are trimmed away).

**What's needed:**
1. Review whitespace normalization rules
2. Ensure raw content output matches the Node implementation's normalization

---

### 22. 🟡 DOCTYPE Handling

**Node behavior:** `<!DOCTYPE HTML>` is preserved as raw content.

**Test cases affected:**
- `should process a complex raw text` — includes `<!DOCTYPE HTML>`

**What's needed:**
1. Handle `doctype` tree-sitter node type
2. Emit as raw content

---

## Protocol Schema Changes Required

The following changes to `webui.proto` are needed:

```protobuf
// New: Attribute fragment for dynamic attributes
message WebUIFragmentAttribute {
  string name = 1;
  string value = 2;              // signal name for simple dynamic attrs
  string template = 3;           // stream ID for mixed (template) attrs
  bool complex = 4;              // true for :-prefixed attrs
  bool attr_start = 5;           // true for first dynamic attr on component
  bool attr_skip = 6;            // true for skipped attrs (class, style, etc.)
  bool raw_value = 7;            // true for static attrs on components
  ConditionExpr condition_tree = 8; // for ?-prefixed boolean attrs
}

// New: Hydration fragment
message WebUIFragmentHydration {
  int32 attribute_count = 1;
}

// Updated: Signal with default value
message WebUIFragmentSignal {
  string value = 1;
  bool raw = 2;
  string default_value = 3;      // NEW: for <f-signal> default content
}

// Updated: WebUIFragment oneof
message WebUIFragment {
  oneof fragment {
    WebUIFragmentRaw raw = 1;
    WebUIFragmentComponent component = 2;
    WebUIFragmentFor for_loop = 3;
    WebUIFragmentSignal signal = 4;
    WebUIFragmentIf if_cond = 5;
    WebUIFragmentAttribute attribute = 6;    // NEW
    WebUIFragmentHydration hydration = 7;    // NEW
  }
}
```

---

## Implementation Priority

| Priority | Feature | Complexity | Dependencies |
|----------|---------|-----------|--------------|
| ~~P0~~ | ~~Attribute fragment type (proto + parser)~~ | ~~High~~ | ✅ Done |
| ~~P0~~ | ~~Boolean attributes (`?` prefix)~~ | ~~Medium~~ | ✅ Done |
| ~~P0~~ | ~~Colon-prefixed complex attributes~~ | ~~Medium~~ | ✅ Done |
| ~~P0~~ | ~~Mixed/template attributes~~ | ~~Medium~~ | ✅ Done |
| P0 | `<f-signal>` element | Low | Proto signal change |
| P1 | Body signals | Low | None |
| P1 | Empty `<for>` handling | Low | None |
| P1 | Custom template attr on `<for>` | Low | None |
| P1 | Self-closing tag handling | Medium | None |
| P1 | Component template wrapping logic | Medium | None |
| P1 | Strip `@`/`:`/`?` attrs from templates | Low | None |
| P2 | Skipped attributes (attrSkip/attrStart) | Medium | Attribute fragment |
| P2 | Component attribute passthrough (rawValue) | Medium | Attribute fragment |
| P2 | CSS component styles (link injection) | Medium | None |
| P2 | Token extraction from styles | Medium | CSS parser |
| P2 | Token signals in style comments | Medium | Token extraction |
| P2 | HTML5 void elements | Low | None |
| P2 | DOCTYPE handling | Low | None |
| P2 | Handlebars edge cases | Low | None |
| P2 | Whitespace normalization | Medium | None |
| P3 | Hydration attributes | Medium | Proto changes |
| P3 | estimatedBufferSize | Low | None |

---

## Todos

1. **proto-attribute-fragment** — Add `WebUIFragmentAttribute` and `WebUIFragmentHydration` to proto schema, add `default_value` to signal
2. **parser-attribute-handling** — Implement dynamic attribute detection and fragment generation for `{{...}}` in attribute values
3. **parser-boolean-attributes** — Implement `?`-prefixed boolean attribute parsing with conditionTree
4. **parser-complex-attributes** — Implement `:`-prefixed complex attribute parsing
5. **parser-mixed-attributes** — Implement template attributes (mixed static+dynamic) with sub-streams
6. **parser-f-signal** — Implement `<f-signal>` transient element parsing
7. **parser-body-signals** — Inject body_start/body_end signals around body content
8. **parser-for-enhancements** — Custom template attr, empty for handling
9. **parser-self-closing-tags** — Handle self-closing tags and HTML5 void elements
10. **parser-component-enhancements** — Template wrapping logic, style injection, attribute stripping
11. **parser-component-attributes** — attrSkip, attrStart, rawValue for component attributes
12. **parser-css-tokens** — Token extraction from styles, token signals in comments
13. **parser-edge-cases** — Handlebars edge cases, whitespace normalization, DOCTYPE
14. **parser-hydration** — Hydration fragment support for fast-html template type
15. **parser-buffer-size** — estimatedBufferSize calculation
