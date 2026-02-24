# WebUI Rust Port — Feature Tracker

Tracking parity between the prototype implementations and the Rust port.

## Prototype Sources

| Area | Source | Path |
|------|--------|------|
| Parser (build step) | Node.js | `~/edge/src/edge_webui/packages/build/src/btr/generator.test.js` |
| Parser (fast-html) | Node.js | `~/edge/src/edge_webui/packages/build/src/btr/generator-fast-html.test.js` |
| Handler (JS) | Node.js | `~/edge/src/edge_webui/packages/build/src/btr/handler.test.js` |
| Handler (C++) | C++ | `~/edge/src/components/edge_build_time_rendering/webui_handler_unittest.cc` |

## Rust Crates

| Crate | Role | Tests |
|-------|------|-------|
| `webui-parser` | Build-step parser (equivalent to generator.js) | 35 |
| `webui-handler` | Runtime renderer (equivalent to handler.js / C++ handler) | 7 |
| `webui-protocol` | Proto schema + serialization | 13 |

---

## ✅ Completed — Parser (`webui-parser`)

| # | Prototype Test | Rust Test | Branch |
|---|----------------|-----------|--------|
| 1 | `should process raw text correctly` | `test_parse_signal` (partial) | pre-existing |
| 2 | `should process signal correctly` | `test_parse_signal` | pre-existing |
| 3 | `should process handlebars from text as signals` | `test_parse_raw_signal` | pre-existing |
| 4 | `should process for stream correctly` | `test_parse_for_directive` | pre-existing |
| 5 | `should process nested for streams correctly` | `test_nested_directives` | pre-existing |
| 6 | `should process transient node if` | `test_parse_if_directive` | pre-existing |
| 7 | `should process missing web components correctly` | `test_component_directive` | pre-existing |
| 8 | `should process handlebars from attributes as signals` | `test_attribute_handlebars_in_href` | `attribute-fragments` |
| 9 | `should process boolean attribute with handlebars expression` | `test_attribute_boolean_with_handlebars` | `attribute-fragments` |
| 10 | `should process multiple boolean attributes` | `test_attribute_multiple_boolean` | `attribute-fragments` |
| 11 | `should process a boolean attribute and a regular attribute together` | `test_attribute_boolean_and_regular_together` | `attribute-fragments` |
| 12 | `should process a boolean attribute sandwiched between regular attributes` | `test_attribute_boolean_sandwiched` | `attribute-fragments` |
| 13 | `should process html ending with boolean attribute correctly` | `test_attribute_boolean_ending` | `attribute-fragments` |
| 14 | `should process boolean attribute with dotted path` | `test_attribute_boolean_dotted_path` | `attribute-fragments` |
| 15 | `should process colon-prefixed attribute with handlebars` | `test_attribute_colon_prefixed_complex` | `attribute-fragments` |
| 16 | `should process multiple colon-prefixed complex attributes` | `test_attribute_multiple_colon_prefixed` | `attribute-fragments` |
| 17 | `should process mixed normal, boolean, and colon-prefixed attributes` | `test_attribute_mixed_normal_boolean_colon` | `attribute-fragments` |
| 18 | `should reject boolean attribute without handlebars` | `test_attribute_reject_boolean_without_handlebars` | `attribute-fragments` |
| 19 | `should reject boolean attribute with partial handlebars` | `test_attribute_reject_boolean_with_partial_handlebars` | `attribute-fragments` |
| 20 | `should reject boolean attribute with plain value` | `test_attribute_reject_boolean_with_plain_value` | `attribute-fragments` |
| 21 | `should process mixed attributes correctly` | `test_attribute_mixed_static_dynamic` | `attribute-fragments` |
| 22 | `should inject body_start and body_end signal` | `test_body_signals` | `parser-p1-features` |
| 23 | `should process nothing with empty for stream` | `test_empty_for_produces_nothing` | `parser-p1-features` |
| 24 | `should process elements with children having self-closing tags` | `test_self_closing_svg_path` | `parser-p1-features` |
| 25 | `should handle HTML5 void elements correctly` | `test_html5_void_elements` | `parser-p1-features` |
| 26 | `should handle self-closing tags with attributes and signals` | `test_self_closing_with_dynamic_attributes` | `parser-p1-features` |
| 27 | `should handle self-closing tags with boolean attributes` | `test_self_closing_with_boolean_attributes` | `parser-p1-features` |
| 28 | `should handle multiple self-closing tags in sequence` | `test_multiple_self_closing_in_sequence` | `parser-p1-features` |
| 29 | `should handle self-closing tags with mixed content types` | `test_self_closing_with_mixed_content` | `parser-p1-features` |
| 30 | `should handle self-closing SVG elements correctly` | `test_self_closing_svg_elements` | `parser-p1-features` |
| 31 | `should handle self-closing tags inside for loops` | `test_self_closing_inside_for_loop` | `parser-p1-features` |
| 32 | `should handle self-closing tags with whitespace variations` | `test_self_closing_whitespace_variations` | `parser-p1-features` |
| 33 | `should handle deeply nested self-closing tags` | `test_deeply_nested_self_closing` | `parser-p1-features` |
| 34 | `should differentiate between self-closing and empty regular tags` | `test_self_closing_vs_empty_regular_tags` | `parser-p1-features` |

## ✅ Completed — Handler (`webui-handler`)

| # | Prototype Test | Rust Test |
|---|----------------|-----------|
| 1 | `RawStreamProducesCorrectOutput` / `should process raw stream` | `test_handle_raw` |
| 2 | `SignalStreamProducesCorrectOutput` / `should process signal stream` | `test_handle_signal` |
| 3 | `ForStreamProcessesCorrectly` / `should process for stream` | `test_handle_for_loop` |
| 4 | `IfTrueStreamProducesCorrectOutput` / `should process if stream` | `test_handle_if_condition` |
| 5 | `ComponentStreamProducesCorrectOutput` / `should process component stream` | `test_handle_component` |
| 6 | (error case) | `test_missing_fragment` |
| 7 | (error case) | `test_missing_signal_renders_empty` |

---

## 🔲 Missing — Parser (`webui-parser`)

### From `generator.test.js` (67 total tests, 34 ported)

| # | Prototype Test | Category | Complexity |
|---|----------------|----------|------------|
| 1 | `should fail with invalid markup` | Error handling | Low |
| 2 | `should process a complex raw text` | DOCTYPE + full page | Medium |
| 3 | `should process signal attribute correctly with default value` | `<f-signal>` | Low |
| 4 | `should process available web components` (with styles + tokens) | Component styles | Medium |
| 5 | `should process available web components with legacy themes` | Component styles | Medium |
| 6 | `should process available web components with slots` (attrStart, rawValue) | Component attrs | Medium |
| 7 | `should process available web components with multiple slots and attributes` | Component attrs | Medium |
| 8 | `handle multiple nested web components` | Nested components | Medium |
| 9 | `should not wrap component with template if it already has a template element` | Template wrapping | Low |
| 10 | `should not wrap styled component with template if it already has a template element` | Template wrapping | Low |
| 11 | `should strip attributes starting "@" or ":" or "?" from the template` | Attr stripping | Low |
| 12 | `should process handlebars from text at beginning` | Handlebars | Low |
| 13 | `should process handlebars from text at beginning and raw` | Handlebars | Low |
| 14 | `should process handlebars from text at raw and end` | Handlebars | Low |
| 15 | `should not process handlebars when invalid` | Handlebars edge | Low |
| 16 | `should not process handlebars when invalid since triple exists` | Handlebars edge | Low |
| 17 | `should not process handlebars when invalid but with valid triple` | Handlebars edge | Low |
| 18 | `should process entities correctly` | Entities | Low |
| 19 | `should extract tokens from style tags` | CSS tokens | Medium |
| 20 | `should process available web components correctly` (tokens merge) | CSS tokens | Medium |
| 21 | `should process transient node f-signal` | `<f-signal>` | Low |
| 22 | `should handle <if> with multiple children` | If directive | Low |
| 23 | `should handle <for> with multiple children` | For directive | Low |
| 24 | `should process transient node for with template` | Custom template attr | Low |
| 25 | `should process recursive transient nodes` | Recursive template | Medium |
| 26 | `should handle nested self-closing tags in components` | Component + self-close | Medium |
| 27 | `should handle self-closing custom elements` | Component self-close | Medium |
| 28 | `should handle meta tags and link tags correctly` | Void elements + attrs | Low |
| 29 | `should set attrSkip for skipped component attributes` | attrSkip | Medium |
| 30 | `should mark leading boolean component attribute with attrStart` | attrStart | Medium |
| 31 | `should have estimatedBufferSize field set correctly` | Buffer size | Low |
| 32 | `should render hydration attributes correctly for option template fast-html` | Hydration | Medium |
| 33 | `should process transient node for` (standalone) | For directive | Low |

### From `generator-fast-html.test.js` (11 tests, 0 ported)

| # | Prototype Test | Category | Complexity |
|---|----------------|----------|------------|
| 1 | `should return empty string if no components are parsed` | Fast-html gen | Low |
| 2 | `should create the f-template for single component` | Fast-html gen | Medium |
| 3 | `should skip adding template for components that define it` | Fast-html gen | Medium |
| 4 | `should create the f-template for multiple components` | Fast-html gen | Medium |
| 5 | `should not create f-template for non js components` | Fast-html gen | Low |
| 6 | `should create inner template for non js components when used` | Fast-html gen | Medium |
| 7 | `should transform if tags to f-when` | Fast-html gen | Medium |
| 8 | `should transform for tags to f-repeat` | Fast-html gen | Medium |
| 9 | `should transform nested if and for tags properly` | Fast-html gen | Medium |
| 10 | `should include complex attributes in generated templates` | Fast-html gen | Low |
| 11 | `should normalize complex attributes across spacing and quoting variants` | Fast-html gen | Low |

## 🔲 Missing — Handler (`webui-handler`)

### From `handler.test.js` (62 tests, 7 ported)

| # | Prototype Test | Category | Complexity |
|---|----------------|----------|------------|
| 1 | `should process nested for stream` | For loop | Low |
| 2 | `should process nested for stream with signals` | For + signals | Medium |
| 3 | `should process nested for stream with signals and top-level state` | For + global state | Medium |
| 4 | `should process available web components with slots` | Component slots | Medium |
| 5 | `handle multiple nested component stream` | Nested components | Medium |
| 6 | `should process raw signals and not escape them` | Raw signals | Low |
| 7 | `should process for and if streams with overlapping global and local states` | State scoping | Medium |
| 8 | `should process for and if streams where global flag true does not take effect` | State scoping | Medium |
| 9 | `should process recursive node refs correctly` | Recursive refs | Medium |
| 10 | `should process component stream nested in for stream cannot access local state` | State scoping | Medium |
| 11 | `should process nested for streams with hierarchical state access` | State scoping | Medium |
| 12 | `should process component stream nested in for stream can ONLY access global state` | State scoping | Medium |
| 13 | `should not resolve local item state with item moniker in component stream` | State scoping | Medium |
| 14 | `should not use local item state for non-qualified access in ForStream` | State scoping | Low |
| 15 | `should process nested for streams with interleaving if stream` | For + if nesting | High |
| 16 | `should process nested for streams with interleaved if using outer state` | For + if nesting | High |
| 17 | `should process nested for streams with interleaved if using inner state` | For + if nesting | High |
| 18 | `should correctly merge local for stream item and global state when monikers match` | State merging | Medium |
| 19 | `should process component stream nested in for stream with same-name global state` | State scoping | Medium |
| 20 | `should use local item flag in if stream nested in for stream` | State scoping | Medium |
| 21 | `should fallback to global when local flag is missing in for stream` | State fallback | Medium |
| 22 | `should evaluate IfStream condition mixing operands from different scopes` | Mixed scopes | High |
| 23 | `should render boolean attribute when value is true` | Boolean attrs | Low |
| 24 | `should not render boolean attribute when value is false` | Boolean attrs | Low |
| 25 | `should not render boolean attribute when value is missing` | Boolean attrs | Low |
| 26 | `should render multiple boolean attributes correctly` | Boolean attrs | Low |
| 27 | `should render boolean attribute for all truthy values` | Boolean attrs | Low |
| 28 | `should not render boolean attribute for all falsy values except true` | Boolean attrs | Low |
| 29 | `should render boolean attribute when evaluating to true` | Boolean attrs | Low |
| 30 | `should not render boolean attribute when evaluating to false` | Boolean attrs | Low |
| 31 | `should render attribute with value` | Simple attrs | Low |
| 32 | `should render attribute with falsy numeric value` | Simple attrs | Low |
| 33 | `should render mixed attributes correctly` | Template attrs | Medium |
| 34 | `should add mixed attribute values to component attribute state` | Component attr state | Medium |
| 35 | `should camelCase hyphenated mixed attributes in component attribute state` | Component attr state | Medium |
| 36 | `should capture mixed attributes for nested component streams` | Component attr state | Medium |
| 37 | `should capture mixed attributes across parent/child/grandchild` | Component attr state | High |
| 38 | `should capture mixed attributes in nested components from for-stream` | Component attr state | High |
| 39 | `should add multiple mixed attributes to component attribute state` | Component attr state | Medium |
| 40 | `should prioritize component attribute state over global state` | State priority | Medium |
| 41 | `should prioritize component attribute state over local and global state` | State priority | Medium |
| 42 | `should handle boolean attribute as first component attribute` | Component bool attrs | Medium |
| 43 | `should not resolve skipped component attributes defined in constants` | attrSkip | Medium |
| 44 | `should handle hyphenated attributes in component attribute state via camelCase` | camelCase | Medium |
| 45 | `should allow nested component to inherit attribute from parent` | Attr inheritance | Medium |
| 46 | `should allow deeply nested component to inherit attribute from immediate parent` | Attr inheritance | Medium |
| 47 | `should allow component to access complex attribute values` | Complex attrs | Medium |
| 48 | `should pass for-stream moniker state into complex component attributes` | Complex + for | High |
| 49 | `should pass nested for-stream monikers into complex component attributes` | Complex + nested for | High |
| 50 | `should expose boolean component attribute state when true` | Bool component state | Medium |
| 51 | `should expose boolean component attribute state when false` | Bool component state | Medium |
| 52 | `should forward boolean component attribute state to nested components` | Bool component state | Medium |
| 53 | `should handle boolean attribute as first component attribute` (2nd) | Bool component state | Medium |
| 54 | `should not pollute parent scope with component attributes` | Scope isolation | Medium |
| 55 | `should add hydration comments for signals, attributes, nested for/if streams` | Hydration | High |

### From `webui_handler_unittest.cc` (40+ tests, 7 ported via JS equivalents)

The C++ tests largely mirror the JS handler tests. Key additional patterns:
- `SignalStreamWithDefaultValue` — default value rendering
- `EmptyProtocolProducesNoOutput` — empty protocol edge case
- `UnknownStreamTypeDoesNotAffectOutput` — robustness
- `MalformedStateInputHandledGracefully` — error resilience
- `BooleanAttributeStream*` — extensive boolean attribute rendering tests
- `AttributeStreamWithValue` / `AttributeStreamWithTemplate` — attribute rendering
- `MixedAttributeAddedToComponentAttributeState` — component attribute state management
- `HydrationEnabledWithNestedForAndIfStreams` — hydration comment generation
- `ComponentAttributeOverridesGlobalState` — state priority
- `ComponentSpecialAttributeProvidesComplexState` — complex attribute state
- `ComponentAttributesDoNotPolluteParentScope` — scope isolation

---

## Summary

| Area | Total Prototype Tests | Ported | Remaining |
|------|----------------------|--------|-----------|
| Parser (`generator.test.js`) | 67 | 34 | 33 |
| Parser (`generator-fast-html.test.js`) | 11 | 0 | 11 |
| Handler (`handler.test.js`) | 62 | 7 | 55 |
| Handler (`webui_handler_unittest.cc`) | ~40 | 7 (overlap) | ~33 unique |
| **Total** | **~180** | **~48** | **~132** |

## Suggested Implementation Order

### Phase 1 — Parser remaining low-hanging fruit
- [ ] `<f-signal>` element (proto `default_value` + parser)
- [ ] Custom `template` attribute on `<for>`
- [ ] `<if>` / `<for>` with multiple children (may already work — verify)
- [ ] Handlebars edge cases (invalid braces, entities)
- [ ] DOCTYPE handling
- [ ] Handlebars at beginning/end of text

### Phase 2 — Component enhancements
- [ ] Template wrapping logic (skip if already has `<template>`)
- [ ] Strip `@`/`:`/`?` attrs from component templates
- [ ] CSS style link injection into component templates
- [ ] Component attribute passthrough (attrSkip, attrStart, rawValue)
- [ ] Self-closing custom elements
- [ ] Nested self-closing tags in components

### Phase 3 — Handler core
- [ ] Nested for loops with signal resolution
- [ ] For + if state scoping (local vs global)
- [ ] Raw signal rendering (no HTML escaping)
- [ ] Boolean attribute rendering (true/false/missing/truthy/falsy)
- [ ] Simple attribute rendering with value resolution
- [ ] Template/mixed attribute rendering
- [ ] Recursive node ref handling

### Phase 4 — Handler component attributes
- [ ] Component attribute state management
- [ ] camelCase hyphenated attributes
- [ ] Attribute inheritance across component nesting
- [ ] Complex attribute state resolution
- [ ] State priority (component > local > global)
- [ ] Scope isolation (no parent pollution)
- [ ] Skipped attributes (class, style, role, data-*, aria-*)

### Phase 5 — Advanced
- [ ] CSS token extraction from `var()` calls
- [ ] Token signals in style comments
- [ ] Hydration fragments and comment generation
- [ ] `estimatedBufferSize` calculation
- [ ] Fast-html generator (f-template, f-when, f-repeat)


