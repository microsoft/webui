// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI Framework parser plugin.
//!
//! # Overview
//!
//! Compiles HTML templates into structured metadata objects stored as JSON
//! data plus component-local condition closure arrays in the protocol.
//! During SSR the handler emits metadata in an `application/json` data block
//! and only emits the closures as JavaScript. During SPA navigation the
//! router registers the JSON data directly and evaluates only the closures.
//! Each metadata object contains
//! **marker-free static HTML** plus locator arrays for client-created DOM
//! (`tx`, `ag`, `c`/`r` slots) and semantic arrays (`a`, `c`, `r`, `e`,
//! `re`, `b`). The client runtime resolves those locators once and then
//! patches direct node references — **no template string parsing, no regex,
//! no DOM scanning** on the client-created path.
//!
//! # Metadata object format
//!
//! ```js
//! {
//!   "h": "<button class=\"item\"><span></span></button>",
//!   "tx": [[[[0, 0], 0], [["title"]]]],
//!   "a": [["title", 0, "title"]],
//!   "ag": [[[0], 0, 1]],
//!   "c": [[[0, ["state"]], 0, [[0], 1]]],
//!   "e": [["click", "onClick", [], [0]]],
//!   "b": [{ "h": "<span class=\"check\">✓</span>" }],
//!   "sa": "my-component",
//!   "re": [["submit", "onSubmit", [["e"]]]]
//! }
//! ```
//!
//! # Plugin data protocol
//!
//! Each element with attribute bindings or events produces a 12-byte
//! `Plugin` fragment via [`finish_element`](ParserPlugin::finish_element):
//!
//! | Bytes  | Field              | Description                               |
//! |--------|--------------------|-------------------------------------------|
//! | 0–3    | `binding_count`    | Number of dynamic attribute bindings      |
//! | 4–7    | `event_start_idx`  | Starting index in the global event list   |
//! | 8–11   | `event_count`      | Number of `@event` attrs on this element  |
//!
//! The handler plugin decodes this to emit `data-w-*` binding markers
//! and single `data-ev="COUNT"` event markers on **SSR HTML only**. Compiled templates
//! keep the same binding/event indices but encode client targets via locator
//! arrays instead of embedding client-only marker attrs/comments in `h`.
//!
//! # Lifecycle
//!
//! 1. **`classify_attribute`** — intercepts `@event` attributes,
//!    counts them per-element, and keeps them out of the parsed protocol.
//! 2. **`finish_element`** — encodes the accumulated event count
//!    plus attribute binding count into 12 bytes.
//! 3. **`register_component_template`** — caches the component's plugin-facing
//!    template HTML for later compilation (deduplicates by tag name).
//! 4. **`take_component_templates`** — called after parsing is complete;
//!    compiles each tracked component into JSON metadata plus condition closures.

use super::{AttributeAction, ComponentTemplateArtifact, ParserPlugin, ParserPluginArtifacts};
use crate::comment_policy;
use crate::component_registry::Component;
use crate::diagnostic::Diagnostic;
use crate::html_parser::{find_tag_close, style_element_bounds};
use crate::{ConditionParser, DomStrategy, ParserOptions, Result};
use std::cell::Cell;
use std::fmt::Write;
use webui_protocol::{condition_expr, ConditionExpr, WebUIElementData};

/// A component whose plugin-facing template HTML has been captured for compilation.
/// Repeated tracking for the same tag updates the stored template so the plugin
/// can prefer processed HTML over raw component source.
struct TrackedComponent {
    tag_name: String,
    template_html: String,
    root_event_source: String,
    /// Source fact from the component registry. Auto-element metadata is derived
    /// as `!has_script` only when the final template payload is emitted.
    has_script: bool,
}

/// WebUI Framework parser plugin.
///
/// Intercepts `@event` attributes and component registrations during parsing,
/// then compiles each component's HTML template into JSON metadata and JS closures.
///
/// # Event tracking
///
/// Event indices are assigned **globally** across all elements in a component.
/// `element_events` counts events on the current element (reset per element),
/// while `next_event_idx` is a monotonically increasing global counter.
/// This preserves event order in the emitted metadata while letting SSR HTML
/// mark one element with a single `data-ev="COUNT"` attribute.
pub struct WebUIParserPlugin {
    components: Vec<TrackedComponent>,
    /// Per-element event count, reset on each `finish_element` call.
    element_events: Cell<u32>,
    /// Global event index, incremented across all elements in a component.
    next_event_idx: Cell<u32>,
    /// DOM strategy — shadow or light.
    dom_strategy: DomStrategy,
}

impl WebUIParserPlugin {
    #[must_use]
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
            element_events: Cell::new(0),
            next_event_idx: Cell::new(0),
            dom_strategy: DomStrategy::default(),
        }
    }

    /// Compile all tracked components and return split template payloads.
    ///
    /// Each payload contains JSON-safe metadata plus a component-local closure
    /// array for condition evaluation. Call this after all HTML parsing is complete.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ParserError::Template`] if any tracked component
    /// contains an invalid `@event` handler or a non-braced `w-ref` binding.
    pub fn take_component_templates(&self) -> Result<Vec<ComponentTemplateArtifact>> {
        let use_shadow = matches!(self.dom_strategy, DomStrategy::Shadow);
        let mut out = Vec::with_capacity(self.components.len());
        for c in &self.components {
            let payload = generate_compiled_template_with_root_source(
                &c.tag_name,
                &c.template_html,
                &c.root_event_source,
                use_shadow,
                !c.has_script,
            )?;
            out.push(ComponentTemplateArtifact::webui(
                c.tag_name.clone(),
                payload.template_json,
                payload.template_functions,
            ));
        }
        Ok(out)
    }

    fn store_component_template(
        &mut self,
        tag_name: &str,
        template_html: &str,
        root_event_source: &str,
        has_script: bool,
    ) {
        if let Some(component) = self.components.iter_mut().find(|c| c.tag_name == tag_name) {
            component.template_html.clear();
            component.template_html.push_str(template_html);
            component.root_event_source.clear();
            component.root_event_source.push_str(root_event_source);
            component.has_script = has_script;
            return;
        }
        self.components.push(TrackedComponent {
            tag_name: tag_name.to_string(),
            template_html: template_html.to_string(),
            root_event_source: root_event_source.to_string(),
            has_script,
        });
    }
}

impl Default for WebUIParserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserPlugin for WebUIParserPlugin {
    fn start_fragment(&mut self, _fragment_id: &str) {
        self.element_events.set(0);
        self.next_event_idx.set(0);
    }

    fn configure(&mut self, options: &ParserOptions) {
        self.dom_strategy = options.dom_strategy;
    }

    fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction {
        if attr_name.starts_with('@') {
            self.element_events.set(self.element_events.get() + 1);
            return AttributeAction::Skip;
        }
        AttributeAction::Keep
    }

    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        processed_template: &str,
    ) -> Result<()> {
        self.store_component_template(
            tag_name,
            processed_template,
            &component.html_content,
            component.has_script,
        );
        Ok(())
    }

    fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
        let ev_count = self.element_events.get();
        let ev_start = self.next_event_idx.get();

        // Advance the global event index
        self.next_event_idx.set(ev_start + ev_count);
        // Reset per-element event counter
        self.element_events.set(0);

        if binding_attribute_count == 0 && ev_count == 0 {
            return None;
        }

        Some(
            WebUIElementData {
                binding_count: binding_attribute_count,
                event_start: ev_start,
                event_count: ev_count,
            }
            .encode()
            .to_vec(),
        )
    }

    fn into_artifacts(self: Box<Self>) -> Result<ParserPluginArtifacts> {
        Ok(ParserPluginArtifacts::ComponentTemplates(
            self.take_component_templates()?,
        ))
    }
}

// ── Compiled template generation ───────────────────────────────────

const TEXT_MARKER_PREFIX: &str = "<!--t:";
const TEXT_MARKER_SUFFIX: &str = "-->";
const MIN_TEXT_MARKER_DIGITS: usize = 1;
const MIN_TEXT_MARKER_LEN: usize =
    TEXT_MARKER_PREFIX.len() + MIN_TEXT_MARKER_DIGITS + TEXT_MARKER_SUFFIX.len();
const TEXT_MARKER_INDEX_RADIX: usize = 10;

/// A compiled template section.
///
/// Used for the root template and for nested block-table entries.
/// `conditionals` and `repeats` point at block-table indices instead of
/// inlining raw body HTML so the client never reparses nested template syntax.
/// The final `html` payload is marker-free; client-created DOM uses the
/// locator tables to connect bindings without scanning comments or attrs.
#[derive(Default)]
struct TemplateSectionMeta {
    /// Marker-free static HTML for client-created DOM.
    html: String,
    /// Intermediate text binding paths collected during compilation.
    /// These are resolved into `text_runs` during finalization.
    /// Each entry is `(path, raw)` where raw indicates triple-brace `{{{...}}}`.
    text_bindings: Vec<(String, bool)>,
    /// Byte offsets in `html` where compiler-generated text markers start.
    /// Authored CSS may contain marker-like text; only these offsets are metadata.
    text_marker_offsets: Vec<usize>,
    /// Client text-run metadata: `(slot, parts, raw)`.
    /// `raw` is true when the binding uses triple-brace `{{{...}}}` syntax.
    text_runs: Vec<(SlotLocator, Vec<CompiledAttrPart>, bool)>,
    /// Attribute bindings in source order, shared by SSR markers and client `ag[]` locators.
    attr_bindings: Vec<CompiledAttrBinding>,
    /// Client attribute groups: `(element_path, start, count)`.
    attr_groups: Vec<(Vec<usize>, usize, usize)>,
    /// Conditional blocks: `(condition_expression_ast, block_index)`.
    conditionals: Vec<(ConditionExpr, usize)>,
    /// Client conditional anchor slots aligned to `conditionals`.
    condition_slots: Vec<SlotLocator>,
    /// For-loop blocks: `(collection_path, item_variable, block_index)`.
    repeats: Vec<(String, String, usize)>,
    /// Client repeat anchor slots aligned to `repeats`.
    repeat_slots: Vec<SlotLocator>,
    /// Body-level events: `(event_name, handler_method, argument_specs)`.
    events: Vec<EventBinding>,
    /// Client event target element paths aligned to `events`.
    event_targets: Vec<Vec<usize>>,
}

#[derive(Clone)]
struct SlotLocator {
    parent_path: Vec<usize>,
    before_index: usize,
    order: usize,
}

/// Collected metadata produced by [`compile_to_metadata`].
///
/// The root template emits directly into the top-level metadata object.
/// Nested blocks live in `blocks` and are referenced by index from `c` / `r`.
struct TemplateMeta {
    root: TemplateSectionMeta,
    blocks: Vec<TemplateSectionMeta>,
    /// Root-level events from the `<template>` wrapper tag.
    /// Attached to the host element (shadow root host) by the client.
    root_events: Vec<EventBinding>,
}

type EventBinding = (String, String, Vec<EventArg>);

/// Result of parsing one `@event` attribute: the event binding
/// (`event_name`, `handler_name`, `args`) plus the number of bytes consumed.
type ParsedEventAttr = (String, String, Vec<EventArg>, usize);

#[derive(Clone, Debug, PartialEq)]
enum EventArg {
    Event,
    Path(String),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

enum CompiledAttrBinding {
    Simple {
        name: String,
        value: String,
    },
    Complex {
        name: String,
        value: String,
    },
    Boolean {
        name: String,
        condition: ConditionExpr,
    },
    Template {
        name: String,
        parts: Vec<CompiledAttrPart>,
    },
}

enum CompiledAttrPart {
    Static(String),
    Dynamic(String),
}

struct CompiledTemplatePayload {
    template_json: String,
    template_functions: String,
}

struct TemplateBuildMetadata {
    roots: Vec<String>,
    has_events: bool,
}

struct RootScope<'a> {
    name: &'a str,
    parent: Option<usize>,
}

struct RootVisit<'a> {
    block: &'a TemplateSectionMeta,
    scope: Option<usize>,
}

struct ConditionFunctionEmitter {
    functions: String,
    count: usize,
}

impl ConditionFunctionEmitter {
    fn new(capacity: usize) -> Self {
        Self {
            functions: String::with_capacity(capacity),
            count: 0,
        }
    }

    fn push(&mut self, condition: &ConditionExpr) -> usize {
        if self.count == 0 {
            self.functions.push('[');
        } else {
            self.functions.push(',');
        }
        emit_js_condition_function(condition, &mut self.functions);
        let index = self.count;
        self.count += 1;
        index
    }

    fn finish(mut self) -> String {
        if self.count == 0 {
            String::new()
        } else {
            self.functions.push(']');
            self.functions
        }
    }
}

/// Generate a compiled template as JSON-safe metadata.
///
/// The returned string is the metadata data payload. Condition closures are
/// emitted separately by [`generate_compiled_template_with_root_source`].
///
/// # Flow
///
/// 1. Extract `@event` bindings from the `<template>` wrapper (→ `root_events`).
/// 2. Strip the `<template shadowrootmode="…">` wrapper if present.
/// 3. Compile the inner body via [`compile_to_metadata`].
/// 4. Serialize into JSON metadata and component-local condition closures.
///
/// # Compilation rules
///
/// | Source syntax                          | Metadata field(s)          | Client `h` result                 |
/// |---------------------------------------|----------------------------|-----------------------------------|
/// | `{{expr}}`, `{{{expr}}}`, mixed text  | `tx[]`                     | dynamic text run removed          |
/// | `href="{{url}}"`, `class="x {{y}}"`   | `a[]` + `ag[]`             | element kept marker-free          |
/// | `?disabled="{{expr}}"`                | `a[]` + `ag[]`             | element kept marker-free          |
/// | `:config="{{settings}}"`              | `a[]` + `ag[]`             | element kept marker-free          |
/// | `<if condition="…">body</if>`         | `c[]` + `cl[]` + `b[]`     | block removed; anchor slot stored |
/// | `<for each="v in coll">body</for>`    | `r[]` + `rl[]` + `b[]`     | block removed; anchor slot stored |
/// | `<link>` / `<style>` child nodes      | `h`                        | preserved in static HTML          |
/// | module adopted stylesheet specifier   | `sa`                       | stored from `<template>` wrapper  |
/// | `@event="{handler(e)}"`               | `e[]`                      | element kept marker-free          |
/// | `w-ref="name"` / `w-ref={name}`       | *(stays in HTML)*          | *(unchanged)*                     |
/// | `<outlet />` / `<outlet>`             | *(stays in HTML)*          | `<outlet></outlet>`               |
///
/// # Errors
///
/// Returns [`crate::ParserError::Template`] if the template contains an invalid
/// `@event` handler or a non-braced `w-ref` binding.
pub fn generate_compiled_template(tag_name: &str, html_content: &str) -> Result<String> {
    Ok(generate_compiled_template_with_root_source(
        tag_name,
        html_content,
        html_content,
        false,
        false,
    )?
    .template_json)
}

fn generate_compiled_template_with_root_source(
    tag_name: &str,
    html_content: &str,
    root_event_source: &str,
    shadow_dom: bool,
    emit_auto_element: bool,
) -> Result<CompiledTemplatePayload> {
    let trimmed = html_content.trim();
    let root_events = extract_root_events(tag_name, root_event_source.trim())?;
    let adopted_stylesheet = extract_adopted_stylesheet_specifier(trimmed);
    let body = strip_template_wrapper(trimmed);
    let meta = compile_to_metadata(tag_name, body, root_events)?;
    Ok(emit_compiled_template_payload(
        html_content,
        &meta,
        adopted_stylesheet.as_deref(),
        shadow_dom,
        emit_auto_element,
    ))
}

fn emit_compiled_template_payload(
    html_content: &str,
    meta: &TemplateMeta,
    adopted_stylesheet: Option<&str>,
    shadow_dom: bool,
    emit_auto_element: bool,
) -> CompiledTemplatePayload {
    let mut conditions = ConditionFunctionEmitter::new(128);
    let mut out = String::with_capacity(512 + html_content.len());
    let build_meta = collect_template_build_metadata(meta);
    out.push('{');

    emit_json_template_section(&meta.root, &mut out, &mut conditions);

    if let Some(adopted_stylesheet) = adopted_stylesheet {
        out.push_str(",\"sa\":");
        emit_js_string(adopted_stylesheet, &mut out);
    }

    // sd: shadow DOM flag — tells the client runtime to use shadow root
    if shadow_dom {
        out.push_str(",\"sd\":1");
    }

    if emit_auto_element {
        out.push_str(",\"ae\":1");
    }

    // re: root events
    if !meta.root_events.is_empty() {
        out.push_str(",\"re\":[");
        for (i, (event, handler, args)) in meta.root_events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_js_string(event, &mut out);
            out.push(',');
            emit_js_string(handler, &mut out);
            out.push(',');
            emit_js_event_args(args, &mut out);
            out.push(']');
        }
        out.push(']');
    }

    if !build_meta.roots.is_empty() {
        out.push_str(",\"tr\":[");
        for (i, root) in build_meta.roots.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            emit_js_string(root, &mut out);
        }
        out.push_str("],\"ta\":[");
        for (i, root) in build_meta.roots.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let attr = webui_protocol::attrs::camel_to_kebab(root);
            emit_js_string(&attr, &mut out);
            out.push(',');
            emit_js_string(root, &mut out);
        }
        out.push(']');
    }

    if build_meta.has_events {
        out.push_str(",\"tf\":1");
    }

    // b: nested block table
    if !meta.blocks.is_empty() {
        out.push_str(",\"b\":[");
        for (i, block) in meta.blocks.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('{');
            emit_json_template_section(block, &mut out, &mut conditions);
            out.push('}');
        }
        out.push(']');
    }

    out.push('}');
    CompiledTemplatePayload {
        template_json: out,
        template_functions: conditions.finish(),
    }
}

fn emit_js_string(s: &str, out: &mut String) {
    out.push('"');
    let mut index = 0usize;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    while index < s.len() {
        let remaining = &s[index..];
        if remaining.as_bytes().starts_with(b"</")
            && remaining
                .as_bytes()
                .get(2..8)
                .map(|bytes| bytes.eq_ignore_ascii_case(b"script"))
                .unwrap_or(false)
        {
            out.push_str("\\u003C");
            index += 1;
            continue;
        }
        let Some(ch) = remaining.chars().next() else {
            break;
        };
        match ch {
            '\0' => out.push_str("\\u0000"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            '\u{0001}'..='\u{001f}' => {
                let code = ch as usize;
                out.push_str("\\u00");
                out.push(char::from(HEX[(code >> 4) & 0x0f]));
                out.push(char::from(HEX[code & 0x0f]));
            }
            _ => out.push(ch),
        }
        index += ch.len_utf8();
    }
    out.push('"');
}

fn emit_js_event_args(args: &[EventArg], out: &mut String) {
    out.push('[');
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match arg {
            EventArg::Event => out.push_str(r#"["e"]"#),
            EventArg::Path(path) => {
                out.push_str(r#"["p","#);
                emit_js_string(path, out);
                out.push(']');
            }
            EventArg::String(value) => {
                out.push_str(r#"["s","#);
                emit_js_string(value, out);
                out.push(']');
            }
            EventArg::Number(value) => {
                out.push_str(r#"["n","#);
                out.push_str(value);
                out.push(']');
            }
            EventArg::Bool(value) => out.push_str(if *value { r#"["b",1]"# } else { r#"["b",0]"# }),
            EventArg::Null => out.push_str(r#"["z"]"#),
        }
    }
    out.push(']');
}

fn emit_json_attr_binding(
    binding: &CompiledAttrBinding,
    out: &mut String,
    functions: &mut ConditionFunctionEmitter,
) {
    out.push('[');
    match binding {
        CompiledAttrBinding::Simple { name, value } => {
            emit_js_string(name, out);
            out.push_str(",0,");
            emit_js_string(value, out);
        }
        CompiledAttrBinding::Complex { name, value } => {
            emit_js_string(name, out);
            out.push_str(",1,");
            emit_js_string(value, out);
        }
        CompiledAttrBinding::Boolean { name, condition } => {
            emit_js_string(name, out);
            out.push_str(",2,");
            emit_json_condition_ref(condition, out, functions);
        }
        CompiledAttrBinding::Template { name, parts } => {
            emit_js_string(name, out);
            out.push_str(",3,[");
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match part {
                    CompiledAttrPart::Static(value) => emit_js_string(value, out),
                    CompiledAttrPart::Dynamic(path) => {
                        out.push('[');
                        emit_js_string(path, out);
                        out.push(']');
                    }
                }
            }
            out.push(']');
        }
    }
    out.push(']');
}

fn collect_template_build_metadata(meta: &TemplateMeta) -> TemplateBuildMetadata {
    let mut roots = Vec::new();
    let mut scopes = Vec::<RootScope<'_>>::new();
    let mut stack = Vec::with_capacity(1 + meta.blocks.len());
    let mut has_events = !meta.root_events.is_empty();
    stack.push(RootVisit {
        block: &meta.root,
        scope: None,
    });

    while let Some(visit) = stack.pop() {
        let block = visit.block;
        if !block.events.is_empty() {
            has_events = true;
        }

        for (_, parts, _) in &block.text_runs {
            add_part_roots(&mut roots, parts, &scopes, visit.scope);
        }

        for binding in &block.attr_bindings {
            match binding {
                CompiledAttrBinding::Simple { value, .. }
                | CompiledAttrBinding::Complex { value, .. } => {
                    add_root(&mut roots, value, &scopes, visit.scope);
                }
                CompiledAttrBinding::Boolean { condition, .. } => {
                    add_condition_roots(&mut roots, condition, &scopes, visit.scope);
                }
                CompiledAttrBinding::Template { parts, .. } => {
                    add_part_roots(&mut roots, parts, &scopes, visit.scope);
                }
            }
        }

        for (condition, block_index) in &block.conditionals {
            add_condition_roots(&mut roots, condition, &scopes, visit.scope);
            if let Some(child) = meta.blocks.get(*block_index) {
                stack.push(RootVisit {
                    block: child,
                    scope: visit.scope,
                });
            }
        }

        for (collection, item_var, block_index) in &block.repeats {
            add_root(&mut roots, collection, &scopes, visit.scope);
            if let Some(child) = meta.blocks.get(*block_index) {
                let scope = scopes.len();
                scopes.push(RootScope {
                    name: item_var,
                    parent: visit.scope,
                });
                stack.push(RootVisit {
                    block: child,
                    scope: Some(scope),
                });
            }
        }
    }

    TemplateBuildMetadata { roots, has_events }
}

fn path_root(path: &str) -> &str {
    match path.find('.') {
        Some(dot) => &path[..dot],
        None => path,
    }
}

fn is_scoped_path(path: &str, scopes: &[RootScope<'_>], scope: Option<usize>) -> bool {
    let root = path_root(path);
    let mut current = scope;
    while let Some(index) = current {
        let Some(frame) = scopes.get(index) else {
            return false;
        };
        if frame.name == root {
            return true;
        }
        current = frame.parent;
    }
    false
}

fn add_root(roots: &mut Vec<String>, path: &str, scopes: &[RootScope<'_>], scope: Option<usize>) {
    if path.is_empty() || is_scoped_path(path, scopes, scope) {
        return;
    }
    let root = path_root(path);
    if roots.iter().any(|existing| existing == root) {
        return;
    }
    roots.push(root.to_string());
}

fn add_part_roots(
    roots: &mut Vec<String>,
    parts: &[CompiledAttrPart],
    scopes: &[RootScope<'_>],
    scope: Option<usize>,
) {
    for part in parts {
        if let CompiledAttrPart::Dynamic(path) = part {
            add_root(roots, path, scopes, scope);
        }
    }
}

fn is_condition_literal(value: &str) -> bool {
    (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
        || value == "true"
        || value == "false"
        || (!value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-'))
}

fn add_condition_roots(
    roots: &mut Vec<String>,
    condition: &ConditionExpr,
    scopes: &[RootScope<'_>],
    scope: Option<usize>,
) {
    let mut stack = Vec::with_capacity(1);
    stack.push(condition);
    while let Some(current) = stack.pop() {
        match &current.expr {
            Some(condition_expr::Expr::Identifier(id)) => {
                add_root(roots, &id.value, scopes, scope);
            }
            Some(condition_expr::Expr::Predicate(pred)) => {
                add_root(roots, &pred.left, scopes, scope);
                if !is_condition_literal(&pred.right) {
                    add_root(roots, &pred.right, scopes, scope);
                }
            }
            Some(condition_expr::Expr::Not(not_cond)) => {
                if let Some(inner) = not_cond.condition.as_ref() {
                    stack.push(inner);
                }
            }
            Some(condition_expr::Expr::Compound(compound)) => {
                if let Some(left) = compound.left.as_ref() {
                    stack.push(left);
                }
                if let Some(right) = compound.right.as_ref() {
                    stack.push(right);
                }
            }
            None => {}
        }
    }
}

/// Emit a compiled condition function as `function(v,s){return EXPR}`.
/// The function evaluates the condition using `v(path,s)` for value resolution.
fn emit_js_condition_function(condition: &ConditionExpr, out: &mut String) {
    out.push_str("function(v,s){return ");
    emit_js_condition_expr(condition, out);
    out.push('}');
}

/// Emit a JSON condition reference as `[functionIndex,["path1",...]]`.
/// The paths array lists all referenced identifiers for the reactive path index.
fn emit_json_condition_ref(
    condition: &ConditionExpr,
    out: &mut String,
    functions: &mut ConditionFunctionEmitter,
) {
    let function_index = functions.push(condition);
    out.push('[');
    let _ = write!(out, "{}", function_index);
    out.push_str(",[");
    let mut paths = Vec::new();
    collect_condition_paths(condition, &mut paths);
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        emit_js_string(path, out);
    }
    out.push_str("]]");
}

/// Emit the JS expression body for a condition (no wrapping).
fn emit_js_condition_expr(condition: &ConditionExpr, out: &mut String) {
    match &condition.expr {
        Some(condition_expr::Expr::Identifier(id)) => {
            // Truthiness check: !!v("path",s)
            out.push_str("!!v(");
            emit_js_string(&id.value, out);
            out.push_str(",s)");
        }
        Some(condition_expr::Expr::Predicate(pred)) => {
            // Comparison: resolve left, compare with right
            out.push_str("v(");
            emit_js_string(&pred.left, out);
            out.push_str(",s)");
            match pred.operator {
                1 => out.push('>'),      // GT
                2 => out.push('<'),      // LT
                3 => out.push_str("=="), // EQ (use == for loose comparison like Object.is)
                4 => out.push_str("!="), // NEQ
                5 => out.push_str(">="), // GTE
                6 => out.push_str("<="), // LTE
                _ => out.push_str("=="),
            }
            // Right side: resolve as value or emit literal
            emit_js_predicate_right(&pred.right, out);
        }
        Some(condition_expr::Expr::Not(not_cond)) => {
            out.push('!');
            if let Some(inner) = not_cond.condition.as_ref() {
                out.push('(');
                emit_js_condition_expr(inner, out);
                out.push(')');
            } else {
                out.push_str("false");
            }
        }
        Some(condition_expr::Expr::Compound(compound)) => {
            out.push('(');
            if let Some(left) = compound.left.as_ref() {
                emit_js_condition_expr(left, out);
            } else {
                out.push_str("false");
            }
            match compound.op {
                1 => out.push_str("&&"), // AND
                2 => out.push_str("||"), // OR
                _ => out.push_str("&&"),
            }
            if let Some(right) = compound.right.as_ref() {
                emit_js_condition_expr(right, out);
            } else {
                out.push_str("false");
            }
            out.push(')');
        }
        None => out.push_str("false"),
    }
}

/// Emit the right-hand side of a predicate comparison.
/// Literals are emitted directly; identifiers are resolved via v().
fn emit_js_predicate_right(value: &str, out: &mut String) {
    // Check for string literals: "..." or '...'
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        // Emit as JS string (strip outer quotes, re-emit as double-quoted)
        emit_js_string(&value[1..value.len() - 1], out);
        return;
    }
    // Boolean literals
    if value == "true" || value == "false" {
        out.push_str(value);
        return;
    }
    // Numeric literals
    if value
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
        && !value.is_empty()
    {
        out.push_str(value);
        return;
    }
    // Otherwise it's an identifier — resolve it
    out.push_str("v(");
    emit_js_string(value, out);
    out.push_str(",s)");
}

/// Collect all identifier paths referenced by a condition.
fn collect_condition_paths(condition: &ConditionExpr, paths: &mut Vec<String>) {
    match &condition.expr {
        Some(condition_expr::Expr::Identifier(id)) => {
            paths.push(id.value.clone());
        }
        Some(condition_expr::Expr::Predicate(pred)) => {
            paths.push(pred.left.clone());
            // Right side might also be an identifier (not a literal)
            let r = &pred.right;
            if !((r.starts_with('"') && r.ends_with('"'))
                || (r.starts_with('\'') && r.ends_with('\''))
                || r == "true"
                || r == "false"
                || r.bytes()
                    .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-'))
            {
                paths.push(r.clone());
            }
        }
        Some(condition_expr::Expr::Not(not_cond)) => {
            if let Some(inner) = not_cond.condition.as_ref() {
                collect_condition_paths(inner, paths);
            }
        }
        Some(condition_expr::Expr::Compound(compound)) => {
            if let Some(left) = compound.left.as_ref() {
                collect_condition_paths(left, paths);
            }
            if let Some(right) = compound.right.as_ref() {
                collect_condition_paths(right, paths);
            }
        }
        None => {}
    }
}

fn emit_js_node_path(path: &[usize], out: &mut String) {
    out.push('[');
    for (i, index) in path.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{}", index);
    }
    out.push(']');
}

fn emit_js_slot(slot: &SlotLocator, out: &mut String) {
    out.push('[');
    emit_js_node_path(&slot.parent_path, out);
    out.push(',');
    let _ = write!(out, "{}", slot.before_index);
    if slot.order > 0 {
        out.push(',');
        let _ = write!(out, "{}", slot.order);
    }
    out.push(']');
}

fn emit_js_text_parts(parts: &[CompiledAttrPart], out: &mut String) {
    out.push('[');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match part {
            CompiledAttrPart::Static(value) => emit_js_string(value, out),
            CompiledAttrPart::Dynamic(path) => {
                out.push('[');
                emit_js_string(path, out);
                out.push(']');
            }
        }
    }
    out.push(']');
}

fn emit_json_template_section(
    meta: &TemplateSectionMeta,
    out: &mut String,
    functions: &mut ConditionFunctionEmitter,
) {
    out.push_str("\"h\":");
    emit_js_string(&meta.html, out);

    if !meta.text_runs.is_empty() {
        out.push_str(",\"tx\":[");
        for (i, (slot, parts, raw)) in meta.text_runs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_js_slot(slot, out);
            out.push(',');
            emit_js_text_parts(parts, out);
            if *raw {
                out.push_str(",1");
            }
            out.push(']');
        }
        out.push(']');
    }

    if !meta.attr_bindings.is_empty() {
        out.push_str(",\"a\":[");
        for (i, binding) in meta.attr_bindings.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            emit_json_attr_binding(binding, out, functions);
        }
        out.push(']');
    }

    if !meta.attr_groups.is_empty() {
        out.push_str(",\"ag\":[");
        for (i, (path, start, count)) in meta.attr_groups.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_js_node_path(path, out);
            out.push(',');
            let _ = write!(out, "{}", start);
            out.push(',');
            let _ = write!(out, "{}", count);
            out.push(']');
        }
        out.push(']');
    }

    if !meta.conditionals.is_empty() {
        out.push_str(",\"c\":[");
        for (i, ((cond, block_index), slot)) in meta
            .conditionals
            .iter()
            .zip(meta.condition_slots.iter())
            .enumerate()
        {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_json_condition_ref(cond, out, functions);
            out.push(',');
            let _ = write!(out, "{}", block_index);
            out.push(',');
            emit_js_slot(slot, out);
            out.push(']');
        }
        out.push(']');
    }

    if !meta.repeats.is_empty() {
        out.push_str(",\"r\":[");
        for (i, ((collection, item_var, block_index), slot)) in meta
            .repeats
            .iter()
            .zip(meta.repeat_slots.iter())
            .enumerate()
        {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_js_string(collection, out);
            out.push(',');
            emit_js_string(item_var, out);
            out.push(',');
            let _ = write!(out, "{}", block_index);
            out.push(',');
            emit_js_slot(slot, out);
            out.push(']');
        }
        out.push(']');
    }

    if !meta.events.is_empty() {
        out.push_str(",\"e\":[");
        for (i, ((event, handler, args), target)) in meta
            .events
            .iter()
            .zip(meta.event_targets.iter())
            .enumerate()
        {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            emit_js_string(event, out);
            out.push(',');
            emit_js_string(handler, out);
            out.push(',');
            emit_js_event_args(args, out);
            out.push(',');
            emit_js_node_path(target, out);
            out.push(']');
        }
        out.push(']');
    }
}

/// Compile a template body into a [`TemplateMeta`] struct.
///
/// Performs a single forward pass over the input bytes to build an intermediate
/// section, then finalizes that section into marker-free client HTML plus
/// locator metadata. During the forward pass:
///
/// - **Multi-byte char boundary check** — ensures we don't split UTF-8 (e.g. emoji).
/// - **`{{{expr}}}`** — triple-brace raw binding → text run metadata.
/// - **`{{expr}}`** — double-brace escaped binding → text run metadata.
/// - **regular start tags** — attrs are compiled into explicit `a[]` metadata plus
///   SSR binding markers that are later stripped from client `h`.
/// - **`<if condition="…">`** — parsed via [`parse_if_block`] → conditional slot.
/// - **`<for each="v in coll">`** — parsed via [`parse_for_block`] → repeat slot.
/// - **`<outlet …>`** — normalized to `<outlet></outlet>`.
/// - **`@event="…"`** (inside a tag) — parsed into `e[]` entries plus
///   SSR `data-ev="COUNT"` markers that are later replaced by client locators.
/// - **Everything else** — copied verbatim to the intermediate static HTML.
///
fn compile_to_metadata(
    component: &str,
    input: &str,
    root_events: Vec<EventBinding>,
) -> Result<TemplateMeta> {
    let mut blocks = Vec::new();
    let mut root = compile_section(component, input, &mut blocks)?;
    finalize_template_section(&mut root);
    for block in &mut blocks {
        finalize_template_section(block);
    }
    Ok(TemplateMeta {
        root,
        blocks,
        root_events,
    })
}

fn compile_section(
    component: &str,
    input: &str,
    blocks: &mut Vec<TemplateSectionMeta>,
) -> Result<TemplateSectionMeta> {
    let mut meta = TemplateSectionMeta {
        html: String::with_capacity(input.len()),
        text_bindings: Vec::new(),
        text_marker_offsets: Vec::new(),
        text_runs: Vec::new(),
        attr_bindings: Vec::new(),
        attr_groups: Vec::new(),
        conditionals: Vec::new(),
        condition_slots: Vec::new(),
        repeats: Vec::new(),
        repeat_slots: Vec::new(),
        events: Vec::new(),
        event_targets: Vec::new(),
    };

    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if !input.is_char_boundary(i) {
            meta.html.push(bytes[i] as char);
            i += 1;
            continue;
        }

        if let Some(next) = compile_text_binding_at(input, i, &mut meta) {
            i = next;
            continue;
        }

        // <if condition="...">...</if> → marker + conditional
        if bytes[i] == b'<' {
            let remaining = &input[i..];
            if remaining.starts_with("<!--") {
                if let Some(close) = remaining.find("-->") {
                    i += close + 3;
                    continue;
                }
            }

            if remaining.starts_with("<if ") || remaining.starts_with("<if\n") {
                if let Some((cond, body, consumed)) = parse_if_block(remaining) {
                    let block_index = blocks.len();
                    blocks.push(TemplateSectionMeta::default());
                    let block = compile_section(component, &body, blocks)?;
                    blocks[block_index] = block;
                    let idx = meta.conditionals.len();
                    meta.conditionals.push((cond, block_index));
                    meta.html.push_str(&format!("<!--c:{idx}-->"));
                    i += consumed;
                    continue;
                }
            }

            // <for each="item in collection">...</for> → marker + repeat
            if remaining.starts_with("<for ") || remaining.starts_with("<for\n") {
                if let Some((collection, item_var, body, consumed)) = parse_for_block(remaining) {
                    let block_index = blocks.len();
                    blocks.push(TemplateSectionMeta::default());
                    let block = compile_section(component, &body, blocks)?;
                    blocks[block_index] = block;
                    let idx = meta.repeats.len();
                    meta.repeats.push((collection, item_var, block_index));
                    meta.html.push_str(&format!("<!--r:{idx}-->"));
                    i += consumed;
                    continue;
                }
            }

            if let Some((open_end, close_start, close_end)) = find_style_element_bounds(input, i) {
                meta.html.push_str(&input[i..open_end]);
                compile_style_content(&input[open_end..close_start], &mut meta);
                meta.html.push_str(&input[close_start..close_end]);
                i = close_end;
                continue;
            }

            // <outlet ... /> → keep as <outlet></outlet>
            if remaining.starts_with("<outlet") {
                if let Some(close) = find_tag_close(remaining) {
                    meta.html.push_str("<outlet></outlet>");
                    i += close + 1;
                    continue;
                }
            }

            if let Some((tag_html, consumed)) = parse_regular_tag(component, remaining, &mut meta)?
            {
                meta.html.push_str(&tag_html);
                i += consumed;
                continue;
            }
        }

        // @event attributes → replace with a per-element event-count marker
        if bytes[i] == b'@' && is_inside_tag(input, i) {
            if let Some((event_name, handler, args, consumed)) =
                parse_event_attr(component, input, i)?
            {
                meta.events.push((event_name, handler, args));
                meta.html.push_str("data-ev=\"1\"");
                i += consumed;
                continue;
            }
        }

        // w-ref stays in static HTML as-is — the runtime binds from the DOM directly.
        // No metadata entry needed; the attribute value IS the property name.

        // Copy character
        let ch = &input[i..];
        if let Some(c) = ch.chars().next() {
            meta.html.push(c);
            i += c.len_utf8();
        } else {
            i += 1;
        }
    }

    Ok(meta)
}

fn compile_text_binding_at(
    input: &str,
    index: usize,
    meta: &mut TemplateSectionMeta,
) -> Option<usize> {
    let bytes = input.as_bytes();

    if index + 4 < bytes.len()
        && bytes[index] == b'{'
        && bytes[index + 1] == b'{'
        && bytes[index + 2] == b'{'
    {
        if let Some(end) = find_brace_end(input, index + 3, 3) {
            let expr = input[index + 3..end].trim();
            let idx = meta.text_bindings.len();
            meta.text_bindings.push((expr.to_string(), true));
            emit_text_marker(meta, idx);
            return Some(end + 3);
        }
    }

    if index + 3 < bytes.len() && bytes[index] == b'{' && bytes[index + 1] == b'{' {
        if let Some(end) = find_brace_end(input, index + 2, 2) {
            let expr = input[index + 2..end].trim();
            let idx = meta.text_bindings.len();
            meta.text_bindings.push((expr.to_string(), false));
            emit_text_marker(meta, idx);
            return Some(end + 2);
        }
    }

    None
}

fn emit_text_marker(meta: &mut TemplateSectionMeta, index: usize) {
    meta.text_marker_offsets.push(meta.html.len());
    meta.html.push_str(TEXT_MARKER_PREFIX);
    let _ = write!(meta.html, "{index}");
    meta.html.push_str(TEXT_MARKER_SUFFIX);
}

fn compile_style_content(input: &str, meta: &mut TemplateSectionMeta) {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    while index < len {
        if index + 1 < len && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            if let Some(close) = input[index + 2..].find("*/") {
                let end = index + 2 + close + 2;
                let comment = &input[index..end];
                if compile_css_signal_comment(comment, meta) {
                    index = end;
                    continue;
                }
                if comment_policy::is_legal_css_comment(comment) {
                    meta.html.push_str(comment);
                }
                index = end;
                continue;
            }
        }

        if comment_policy::is_css_line_comment_start(input, index) {
            let end = comment_policy::find_css_line_comment_end(input, index + 2);
            let comment = &input[index..end];
            if comment_policy::is_legal_css_comment(comment) {
                meta.html.push_str(comment);
            }
            index = end;
            continue;
        }

        if let Some(next) = compile_text_binding_at(input, index, meta) {
            index = next;
            continue;
        }

        let Some(ch) = input[index..].chars().next() else {
            break;
        };
        meta.html.push(ch);
        index += ch.len_utf8();
    }
}

fn compile_css_signal_comment(comment: &str, meta: &mut TemplateSectionMeta) -> bool {
    if let Some(signal) = comment_policy::parse_css_signal_comment(comment) {
        let idx = meta.text_bindings.len();
        meta.text_bindings.push((signal.path, signal.raw));
        emit_text_marker(meta, idx);
        return true;
    }

    false
}

fn find_style_element_bounds(input: &str, index: usize) -> Option<(usize, usize, usize)> {
    let (content_start, close_start, close_end) = style_element_bounds(&input[index..])?;
    Some((
        index + content_start,
        index + close_start,
        index + close_end,
    ))
}

#[derive(Clone)]
enum FragmentNode {
    Element(FragmentElement),
    Text(String),
    TextMarker(usize),
    Comment(String),
}

#[derive(Clone)]
struct FragmentElement {
    tag_name: String,
    attrs: Vec<FragmentAttr>,
    children: Vec<FragmentNode>,
    self_closing: bool,
}

#[derive(Clone)]
struct FragmentAttr {
    name: String,
    value: Option<String>,
}

fn finalize_template_section(meta: &mut TemplateSectionMeta) {
    let raw_html = std::mem::take(&mut meta.html);
    let text_marker_offsets = std::mem::take(&mut meta.text_marker_offsets);
    let nodes = parse_fragment_nodes(&raw_html, &text_marker_offsets);
    let text_bindings = meta.text_bindings.clone();
    let mut finalized_html = String::with_capacity(raw_html.len());
    let mut text_runs = Vec::new();
    let mut attr_groups = Vec::new();
    let mut condition_slots = vec![None; meta.conditionals.len()];
    let mut repeat_slots = vec![None; meta.repeats.len()];
    let mut event_targets = vec![None; meta.events.len()];
    let mut event_cursor = 0usize;

    process_fragment_children(
        &nodes,
        &[],
        &text_bindings,
        &mut finalized_html,
        &mut text_runs,
        &mut attr_groups,
        &mut condition_slots,
        &mut repeat_slots,
        &mut event_targets,
        &mut event_cursor,
    );
    debug_assert_eq!(event_cursor, meta.events.len());

    meta.html = finalized_html;
    meta.text_runs = text_runs;
    meta.attr_groups = attr_groups;
    meta.condition_slots = condition_slots.into_iter().flatten().collect();
    meta.repeat_slots = repeat_slots.into_iter().flatten().collect();
    meta.event_targets = event_targets.into_iter().flatten().collect();
}

#[allow(clippy::too_many_arguments)]
fn process_fragment_children(
    nodes: &[FragmentNode],
    parent_path: &[usize],
    text_bindings: &[(String, bool)],
    out: &mut String,
    text_runs: &mut Vec<(SlotLocator, Vec<CompiledAttrPart>, bool)>,
    attr_groups: &mut Vec<(Vec<usize>, usize, usize)>,
    condition_slots: &mut [Option<SlotLocator>],
    repeat_slots: &mut [Option<SlotLocator>],
    event_targets: &mut [Option<Vec<usize>>],
    event_cursor: &mut usize,
) {
    let mut child_index = 0usize;
    let mut index = 0usize;
    // Browser HTML parsing merges adjacent emitted text into a single Text node.
    // Track that here so marker-free locator paths match the real client DOM.
    let mut previous_emitted_text = false;
    let mut slot_orders = std::collections::BTreeMap::<usize, usize>::new();

    while index < nodes.len() {
        if let Some((parts, consumed, has_dynamic, is_raw)) =
            collect_text_run(&nodes[index..], text_bindings)
        {
            if has_dynamic {
                // Decode HTML entities in static parts so that runtime
                // textContent assignment renders decoded characters
                // (e.g. `&gt;` → `>`).  Static-only text runs are
                // baked into `meta.h` and parsed via innerHTML which
                // handles entity decoding natively.
                let decoded_parts = parts
                    .into_iter()
                    .map(|part| match part {
                        CompiledAttrPart::Static(s) => {
                            let decoded = html_escape::decode_html_entities(&s);
                            CompiledAttrPart::Static(decoded.into_owned())
                        }
                        other => other,
                    })
                    .collect();
                let order = slot_orders.get(&child_index).copied().unwrap_or(0);
                slot_orders.insert(child_index, order + 1);
                text_runs.push((
                    SlotLocator {
                        parent_path: parent_path.to_vec(),
                        before_index: child_index,
                        order,
                    },
                    decoded_parts,
                    is_raw,
                ));
            } else {
                let static_text = collect_static_text(&parts);
                if !static_text.is_empty() {
                    out.push_str(&static_text);
                    if !previous_emitted_text {
                        child_index += 1;
                    }
                    previous_emitted_text = true;
                }
            }
            index += consumed;
            continue;
        }

        match &nodes[index] {
            FragmentNode::TextMarker(_) => {}
            FragmentNode::Comment(data) => {
                if let Some(idx) = parse_marker_index(data, "c:") {
                    if let Some(slot) = condition_slots.get_mut(idx) {
                        let order = slot_orders.get(&child_index).copied().unwrap_or(0);
                        slot_orders.insert(child_index, order + 1);
                        *slot = Some(SlotLocator {
                            parent_path: parent_path.to_vec(),
                            before_index: child_index,
                            order,
                        });
                    }
                } else if let Some(idx) = parse_marker_index(data, "r:") {
                    if let Some(slot) = repeat_slots.get_mut(idx) {
                        let order = slot_orders.get(&child_index).copied().unwrap_or(0);
                        slot_orders.insert(child_index, order + 1);
                        *slot = Some(SlotLocator {
                            parent_path: parent_path.to_vec(),
                            before_index: child_index,
                            order,
                        });
                    }
                } else {
                    out.push_str("<!--");
                    out.push_str(data);
                    out.push_str("-->");
                    child_index += 1;
                    previous_emitted_text = false;
                }
            }
            FragmentNode::Text(text) => {
                out.push_str(text);
                if !text.is_empty() {
                    if !previous_emitted_text {
                        child_index += 1;
                    }
                    previous_emitted_text = true;
                }
            }
            FragmentNode::Element(element) => {
                let mut element_path = parent_path.to_vec();
                element_path.push(child_index);
                serialize_fragment_element(
                    element,
                    &element_path,
                    text_bindings,
                    out,
                    text_runs,
                    attr_groups,
                    condition_slots,
                    repeat_slots,
                    event_targets,
                    event_cursor,
                );
                child_index += 1;
                previous_emitted_text = false;
            }
        }

        index += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn serialize_fragment_element(
    element: &FragmentElement,
    element_path: &[usize],
    text_bindings: &[(String, bool)],
    out: &mut String,
    text_runs: &mut Vec<(SlotLocator, Vec<CompiledAttrPart>, bool)>,
    attr_groups: &mut Vec<(Vec<usize>, usize, usize)>,
    condition_slots: &mut [Option<SlotLocator>],
    repeat_slots: &mut [Option<SlotLocator>],
    event_targets: &mut [Option<Vec<usize>>],
    event_cursor: &mut usize,
) {
    out.push('<');
    out.push_str(&element.tag_name);

    for attr in &element.attrs {
        if let Some((start, count)) = parse_attr_group_marker(&attr.name) {
            attr_groups.push((element_path.to_vec(), start, count));
            continue;
        }

        if attr.name == "data-ev" {
            if let Some(count) = attr.value.as_deref().and_then(parse_event_marker_count) {
                let target_path = element_path.to_vec();
                for slot in event_targets.iter_mut().skip(*event_cursor).take(count) {
                    *slot = Some(target_path.clone());
                }
                *event_cursor += count;
            }
            continue;
        }

        out.push(' ');
        out.push_str(&attr.name);
        if let Some(value) = &attr.value {
            out.push_str("=\"");
            emit_html_attr_value(value, out);
            out.push('"');
        }
    }

    if element.self_closing {
        out.push_str(" />");
        return;
    }

    out.push('>');
    process_fragment_children(
        &element.children,
        element_path,
        text_bindings,
        out,
        text_runs,
        attr_groups,
        condition_slots,
        repeat_slots,
        event_targets,
        event_cursor,
    );
    out.push_str("</");
    out.push_str(&element.tag_name);
    out.push('>');
}

fn collect_text_run(
    nodes: &[FragmentNode],
    text_bindings: &[(String, bool)],
) -> Option<(Vec<CompiledAttrPart>, usize, bool, bool)> {
    let mut parts = Vec::new();
    let mut consumed = 0usize;
    let mut has_dynamic = false;
    let mut is_raw = false;

    for node in nodes {
        match node {
            FragmentNode::Text(text) => {
                if !text.is_empty() {
                    parts.push(CompiledAttrPart::Static(text.clone()));
                }
                consumed += 1;
            }
            FragmentNode::Comment(_) => break,
            FragmentNode::TextMarker(index) => {
                if let Some((path, raw)) = text_bindings.get(*index) {
                    parts.push(CompiledAttrPart::Dynamic(path.clone()));
                    has_dynamic = true;
                    if *raw {
                        is_raw = true;
                    }
                    consumed += 1;
                } else {
                    break;
                }
            }
            FragmentNode::Element(_) => break,
        }
    }

    if consumed == 0 {
        return None;
    }

    Some((parts, consumed, has_dynamic, is_raw))
}

fn collect_static_text(parts: &[CompiledAttrPart]) -> String {
    let mut text = String::new();
    for part in parts {
        if let CompiledAttrPart::Static(value) = part {
            text.push_str(value);
        }
    }
    text
}

fn parse_marker_index(data: &str, prefix: &str) -> Option<usize> {
    data.strip_prefix(prefix)?.parse().ok()
}

fn parse_attr_group_marker(name: &str) -> Option<(usize, usize)> {
    if let Some(start) = name.strip_prefix("data-w-b-") {
        return start.parse::<usize>().ok().map(|index| (index, 1));
    }

    let rest = name.strip_prefix("data-w-c-")?;
    let mut parts = rest.split('-');
    let start = parts.next()?.parse::<usize>().ok()?;
    let count = parts.next()?.parse::<usize>().ok()?;
    Some((start, count))
}

fn parse_event_marker_count(value: &str) -> Option<usize> {
    let count = value.parse::<usize>().ok()?;
    (count > 0).then_some(count)
}

fn emit_html_attr_value(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

fn parse_fragment_nodes(input: &str, text_marker_offsets: &[usize]) -> Vec<FragmentNode> {
    let mut roots = Vec::new();
    let mut stack: Vec<FragmentElement> = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    while index < len {
        let remaining = &input[index..];
        if let Some((marker_index, marker_end)) =
            find_text_marker_comment(input, index, 0, text_marker_offsets)
        {
            push_fragment_node(
                &mut roots,
                &mut stack,
                FragmentNode::TextMarker(marker_index),
            );
            index = marker_end;
            continue;
        }

        if remaining.starts_with("<!--") {
            if let Some(close) = remaining.find("-->") {
                push_fragment_node(
                    &mut roots,
                    &mut stack,
                    FragmentNode::Comment(remaining[4..close].to_string()),
                );
                index += close + 3;
                continue;
            }
        }

        if remaining.starts_with("</") {
            if let Some(close) = find_tag_close(remaining) {
                if let Some(element) = stack.pop() {
                    push_fragment_node(&mut roots, &mut stack, FragmentNode::Element(element));
                }
                index += close + 1;
                continue;
            }
        }

        if remaining.starts_with('<') {
            if let Some((mut element, consumed)) = parse_fragment_start_tag(remaining) {
                if element.self_closing {
                    push_fragment_node(&mut roots, &mut stack, FragmentNode::Element(element));
                } else if element.tag_name.eq_ignore_ascii_case("style") {
                    if let Some((content_offset, close_offset, close_end)) =
                        style_element_bounds(remaining)
                    {
                        let content_start = index + content_offset;
                        let close_start = index + close_offset;
                        push_style_raw_text_nodes(
                            &mut element.children,
                            &input[content_start..close_start],
                            content_start,
                            text_marker_offsets,
                        );
                        push_fragment_node(&mut roots, &mut stack, FragmentNode::Element(element));
                        index += close_end;
                        continue;
                    }

                    let content_start = index + consumed;
                    push_style_raw_text_nodes(
                        &mut element.children,
                        &input[content_start..],
                        content_start,
                        text_marker_offsets,
                    );
                    push_fragment_node(&mut roots, &mut stack, FragmentNode::Element(element));
                    index = len;
                    continue;
                } else {
                    stack.push(element);
                }
                index += consumed;
                continue;
            }
        }

        let next = remaining.find('<').unwrap_or(remaining.len());
        let text = &remaining[..next];
        if !text.is_empty() {
            push_fragment_node(&mut roots, &mut stack, FragmentNode::Text(text.to_string()));
        }
        index += next.max(1);
    }

    while let Some(element) = stack.pop() {
        push_fragment_node(&mut roots, &mut stack, FragmentNode::Element(element));
    }

    roots
}

fn push_style_raw_text_nodes(
    children: &mut Vec<FragmentNode>,
    input: &str,
    base_offset: usize,
    text_marker_offsets: &[usize],
) {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;
    let mut text_start = 0usize;

    while index + MIN_TEXT_MARKER_LEN <= len {
        if let Some((marker_index, marker_end)) =
            find_text_marker_comment(input, index, base_offset, text_marker_offsets)
        {
            if text_start < index {
                children.push(FragmentNode::Text(input[text_start..index].to_string()));
            }
            children.push(FragmentNode::TextMarker(marker_index));
            index = marker_end;
            text_start = marker_end;
            continue;
        }
        index += 1;
    }

    if text_start < len {
        children.push(FragmentNode::Text(input[text_start..].to_string()));
    }
}

fn find_text_marker_comment(
    input: &str,
    index: usize,
    base_offset: usize,
    text_marker_offsets: &[usize],
) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    if bytes.get(index..index + TEXT_MARKER_PREFIX.len()) != Some(TEXT_MARKER_PREFIX.as_bytes()) {
        return None;
    }
    if text_marker_offsets
        .binary_search(&(base_offset + index))
        .is_err()
    {
        return None;
    }

    let mut cursor = index + TEXT_MARKER_PREFIX.len();
    let digit_start = cursor;
    let mut marker_index = 0usize;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        marker_index = marker_index
            .checked_mul(TEXT_MARKER_INDEX_RADIX)?
            .checked_add((bytes[cursor] - b'0') as usize)?;
        cursor += 1;
    }
    if cursor == digit_start
        || bytes.get(cursor..cursor + TEXT_MARKER_SUFFIX.len())
            != Some(TEXT_MARKER_SUFFIX.as_bytes())
    {
        return None;
    }

    Some((marker_index, cursor + TEXT_MARKER_SUFFIX.len()))
}

fn push_fragment_node(
    roots: &mut Vec<FragmentNode>,
    stack: &mut [FragmentElement],
    node: FragmentNode,
) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn parse_fragment_start_tag(input: &str) -> Option<(FragmentElement, usize)> {
    if !input.starts_with('<') || input.starts_with("</") || input.starts_with("<!--") {
        return None;
    }

    let end = find_tag_close(input)?;
    let tag_content = &input[1..end];
    let tag_body = tag_content.trim_end();
    let self_closing = tag_body.ends_with('/');
    let tag_body = if self_closing {
        tag_body[..tag_body.len().saturating_sub(1)].trim_end()
    } else {
        tag_body
    };

    let body_bytes = tag_body.as_bytes();
    let mut cursor = 0usize;
    while cursor < body_bytes.len() && !body_bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    let tag_name = &tag_body[..cursor];
    if tag_name.is_empty() {
        return None;
    }

    let mut attrs = Vec::new();
    while cursor < body_bytes.len() {
        while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= body_bytes.len() {
            break;
        }

        let attr_start = cursor;
        while cursor < body_bytes.len()
            && !body_bytes[cursor].is_ascii_whitespace()
            && body_bytes[cursor] != b'='
        {
            cursor += 1;
        }
        if attr_start == cursor {
            cursor += 1;
            continue;
        }

        let name = tag_body[attr_start..cursor].to_string();
        while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        let mut value: Option<String> = None;
        if cursor < body_bytes.len() && body_bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }

            if cursor < body_bytes.len()
                && (body_bytes[cursor] == b'"' || body_bytes[cursor] == b'\'')
            {
                let quote = body_bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < body_bytes.len() && body_bytes[cursor] != quote {
                    cursor += 1;
                }
                value = Some(tag_body[value_start..cursor].to_string());
                if cursor < body_bytes.len() {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < body_bytes.len() && !body_bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                value = Some(tag_body[value_start..cursor].to_string());
            }
        }

        attrs.push(FragmentAttr { name, value });
    }

    Some((
        FragmentElement {
            tag_name: tag_name.to_string(),
            attrs,
            children: Vec::new(),
            self_closing: self_closing || is_void_element(tag_name),
        },
        end + 1,
    ))
}

fn is_void_element(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

// ── Parsing helpers ────────────────────────────────────────────────

/// Find the position of `count` consecutive closing braces (`}`).
/// Returns `Some(start_of_closing_braces)` or `None` if not found.
fn find_brace_end(input: &str, start: usize, count: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = start;
    while i + count - 1 < bytes.len() {
        let all_close = (0..count).all(|j| bytes[i + j] == b'}');
        if all_close {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Check whether byte position `pos` is inside an HTML tag (between `<` and `>`).
/// Scans backward from `pos` — returns `true` if we hit `<` before `>`.
fn is_inside_tag(input: &str, pos: usize) -> bool {
    let bytes = input.as_bytes();
    let mut i = pos;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'>' {
            return false;
        }
        if bytes[i] == b'<' {
            return true;
        }
    }
    false
}

/// Strip `<template shadowrootmode="…">…</template>` wrapper, returning inner content.
/// Returns the input unchanged if no wrapper is present.
fn strip_template_wrapper(html: &str) -> &str {
    if !html.starts_with("<template") {
        return html;
    }
    let Some(open_end) = find_tag_close(html) else {
        return html;
    };
    let inner_start = open_end + 1;
    let inner_end = html.rfind("</template>").unwrap_or(html.len());
    if inner_start >= inner_end {
        return "";
    }
    &html[inner_start..inner_end]
}

fn find_next_block_token(input: &str, cursor: usize, token: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let token_bytes = token.as_bytes();
    let mut index = cursor;
    let mut in_tag = false;
    let mut in_comment = false;
    let mut quote: Option<u8> = None;

    while index + token_bytes.len() <= bytes.len() {
        if in_comment {
            if bytes[index..].starts_with(b"-->") {
                in_comment = false;
                index += 3;
                continue;
            }
            index += 1;
            continue;
        }

        if let Some(active) = quote {
            if bytes[index] == active {
                quote = None;
            }
            index += 1;
            continue;
        }

        if in_tag {
            match bytes[index] {
                b'"' | b'\'' => quote = Some(bytes[index]),
                b'>' => in_tag = false,
                _ => {}
            }
            index += 1;
            continue;
        }

        if &bytes[index..index + token_bytes.len()] == token_bytes {
            return Some(index);
        }

        if bytes[index] == b'<' {
            if bytes[index..].starts_with(b"<!--") {
                in_comment = true;
                index += 4;
                continue;
            }
            in_tag = true;
        }

        index += 1;
    }

    None
}

fn find_next_block_open(input: &str, cursor: usize, name: &str) -> Option<usize> {
    let token = format!("<{name}");
    let bytes = input.as_bytes();
    let token_bytes = token.as_bytes();
    let mut search = cursor;

    while let Some(index) = find_next_block_token(input, search, &token) {
        let next = bytes.get(index + token_bytes.len()).copied();
        if match next {
            None => true,
            Some(byte) => byte == b'>' || byte.is_ascii_whitespace(),
        } {
            return Some(index);
        }
        search = index + 1;
    }

    None
}

fn find_next_block_close(input: &str, cursor: usize, name: &str) -> Option<usize> {
    let token = format!("</{name}>");
    find_next_block_token(input, cursor, &token)
}

fn find_matching_block_end(input: &str, name: &str) -> Option<usize> {
    let open_token = format!("<{name}");
    let close_token = format!("</{name}>");
    if !input.starts_with(&open_token) {
        return None;
    }

    let mut depth = 1usize;
    let mut cursor = open_token.len();

    while cursor < input.len() {
        let next_open = find_next_block_open(input, cursor, name);
        let next_close = find_next_block_close(input, cursor, name);

        match (next_open, next_close) {
            (_, None) => return None,
            (Some(open_pos), Some(close_pos)) if open_pos < close_pos => {
                depth += 1;
                cursor = open_pos + open_token.len();
            }
            (_, Some(close_pos)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(close_pos);
                }
                cursor = close_pos + close_token.len();
            }
        }
    }

    None
}

/// Parse `<if condition="EXPR">BODY</if>` → `(condition, body, bytes_consumed)`.
///
/// Only handles the outermost `<if>` — nested `<if>` blocks inside the body
/// are left as raw HTML for the client runtime to process.
fn parse_if_block(input: &str) -> Option<(ConditionExpr, String, usize)> {
    let close_tag = "</if>";
    let end_pos = find_matching_block_end(input, "if")?;
    let tag_content = &input[..end_pos];

    // Extract condition
    let cond_start = tag_content.find("condition=\"")? + "condition=\"".len();
    let cond_end = tag_content[cond_start..].find('"')? + cond_start;
    let condition = compile_condition_expr(&tag_content[cond_start..cond_end]);

    // Extract body (after the closing >)
    let body_start = find_tag_close(tag_content)? + 1;
    let body = tag_content[body_start..].trim().to_string();

    Some((condition, body, end_pos + close_tag.len()))
}

fn compile_condition_expr(input: &str) -> ConditionExpr {
    let parser = ConditionParser::new();
    match parser.parse(input) {
        Ok(condition) => condition,
        // Component HTML is compiled after the main parser has already validated
        // condition syntax. Keep the emitted metadata structurally valid if that
        // earlier guarantee regresses.
        Err(_) => ConditionExpr::identifier(input),
    }
}

/// Parse `<for each="item in collection">BODY</for>` → `(collection, item_var, body, consumed)`.
///
/// The `each` attribute must follow the `"item in collection"` pattern.
/// The body template retains `{{expr}}` mustaches — they are resolved by the
/// client runtime during reconciliation.
fn parse_for_block(input: &str) -> Option<(String, String, String, usize)> {
    let close_tag = "</for>";
    let end_pos = find_matching_block_end(input, "for")?;
    let tag_content = &input[..end_pos];

    // Extract each="item in collection"
    let each_start = tag_content.find("each=\"")? + "each=\"".len();
    let each_end = tag_content[each_start..].find('"')? + each_start;
    let each_val = &tag_content[each_start..each_end];

    let parts: Vec<&str> = each_val.splitn(3, ' ').collect();
    if parts.len() != 3 || parts[1] != "in" {
        return None;
    }
    let item_var = parts[0].to_string();
    let collection = parts[2].to_string();

    // Extract body
    let body_start = find_tag_close(tag_content)? + 1;
    let body = tag_content[body_start..].trim().to_string();

    Some((collection, item_var, body, end_pos + close_tag.len()))
}

/// Build a [`Diagnostic`] for an invalid `@event` handler.
///
/// Names the owning component (and element, when known) so the build error
/// points at the exact template to fix instead of only echoing the expression.
fn invalid_event_handler(
    component: &str,
    element: Option<&str>,
    event_name: &str,
    raw: &str,
) -> Diagnostic {
    let mut diag = Diagnostic::error(format!("invalid @{event_name} handler"))
        .code(crate::diagnostic::codes::INVALID_EVENT_HANDLER)
        .component(component)
        .snippet(format!("@{event_name}=\"{raw}\""))
        .help(format!(
            "use @{event_name}=\"{{handler()}}\" or @{event_name}=\"{{handler(e)}}\" \
             to pass the event"
        ));
    if let Some(tag) = element {
        diag = diag.element(tag);
    }
    diag
}

/// Parse `@event="{handler()}"` or `@event={handler()}` into event metadata.
///
/// Supports three value quoting styles:
/// - `@click="{onClick()}"` — quoted with braces inside
/// - `@click={onClick()}` — unquoted braces
/// - `@click='onClick()'` — single-quoted
///
/// The handler name and full argument list are extracted from the function call syntax.
/// `component` is the owning component tag, used only for error messages.
fn parse_event_attr(component: &str, input: &str, pos: usize) -> Result<Option<ParsedEventAttr>> {
    let remaining = &input[pos..];
    let Some(eq_pos) = remaining.find('=') else {
        return Ok(None);
    };
    let event_name = remaining[1..eq_pos].to_string(); // skip @

    let after_eq = &remaining[eq_pos + 1..];
    if after_eq.is_empty() {
        return Ok(None);
    }

    let first = after_eq.as_bytes()[0];
    let (raw_value, total_consumed) = if first == b'"' || first == b'\'' {
        let value_start = eq_pos + 2;
        let Some(close) = remaining[value_start..].find(first as char) else {
            return Ok(None);
        };
        (
            &remaining[value_start..value_start + close],
            value_start + close + 1,
        )
    } else if first == b'{' {
        let value_start = eq_pos + 2;
        let Some(close) = remaining[value_start..].find('}') else {
            return Ok(None);
        };
        (
            &remaining[value_start..value_start + close],
            value_start + close + 1,
        )
    } else {
        return Ok(None);
    };

    let inner = raw_value
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(raw_value)
        .trim();

    // Extract method name and argument specs from "handler(...)".
    if inner.is_empty() {
        return Ok(None);
    }
    match parse_event_handler(inner) {
        EventHandler::Valid(handler_name, args) => {
            Ok(Some((event_name, handler_name, args, total_consumed)))
        }
        EventHandler::Empty => Ok(None),
        EventHandler::Invalid(_raw) => {
            Err(invalid_event_handler(component, None, &event_name, inner).into())
        }
    }
}

fn parse_regular_tag(
    component: &str,
    input: &str,
    meta: &mut TemplateSectionMeta,
) -> Result<Option<(String, usize)>> {
    if !input.starts_with('<') || input.starts_with("</") || input.starts_with("<!") {
        return Ok(None);
    }

    let bytes = input.as_bytes();
    if bytes.len() < 2 || !bytes[1].is_ascii_alphabetic() {
        return Ok(None);
    }

    let Some(end) = find_tag_close(input) else {
        return Ok(None);
    };
    let tag_content = &input[1..end];
    let tag_body = tag_content.trim_end();
    let self_closing = tag_body.ends_with('/');
    let tag_body = if self_closing {
        tag_body[..tag_body.len().saturating_sub(1)].trim_end()
    } else {
        tag_body
    };

    let body_bytes = tag_body.as_bytes();
    let mut cursor = 0;
    while cursor < body_bytes.len() && !body_bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    let tag_name = &tag_body[..cursor];
    if tag_name.is_empty() {
        return Ok(None);
    }

    let mut out = String::with_capacity(tag_body.len() + 24);
    out.push('<');
    out.push_str(tag_name);

    let binding_start = meta.attr_bindings.len();
    let event_start = meta.events.len();
    let mut binding_count = 0usize;

    while cursor < body_bytes.len() {
        while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= body_bytes.len() {
            break;
        }

        let attr_start = cursor;
        while cursor < body_bytes.len()
            && !body_bytes[cursor].is_ascii_whitespace()
            && body_bytes[cursor] != b'='
        {
            cursor += 1;
        }
        if attr_start == cursor {
            cursor += 1;
            continue;
        }

        let name = &tag_body[attr_start..cursor];

        while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        let mut value: Option<&str> = None;
        if cursor < body_bytes.len() && body_bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < body_bytes.len() && body_bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }

            if cursor < body_bytes.len()
                && (body_bytes[cursor] == b'"' || body_bytes[cursor] == b'\'')
            {
                let quote = body_bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < body_bytes.len() && body_bytes[cursor] != quote {
                    cursor += 1;
                }
                value = Some(&tag_body[value_start..cursor]);
                if cursor < body_bytes.len() {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < body_bytes.len() && !body_bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                value = Some(&tag_body[value_start..cursor]);
            }
        }

        let raw_attr = tag_body[attr_start..cursor].trim();
        if raw_attr.is_empty() {
            continue;
        }

        if let Some(event_name) = name.strip_prefix('@') {
            let Some(raw_value) = value else {
                continue;
            };
            match parse_event_handler(raw_value) {
                EventHandler::Valid(handler_name, args) => {
                    meta.events
                        .push((event_name.to_string(), handler_name, args));
                }
                EventHandler::Invalid(raw) => {
                    return Err(
                        invalid_event_handler(component, Some(tag_name), event_name, &raw).into(),
                    );
                }
                EventHandler::Empty => {}
            }
            continue;
        }

        if name == "w-ref" {
            // Validate: w-ref must use {braces} to bind to a component property.
            // This is a fatal build-time check — the runtime also validates.
            if let Some(val) = value {
                if !val.starts_with('{') || !val.ends_with('}') {
                    return Err(Diagnostic::error("invalid w-ref binding")
                        .code(crate::diagnostic::codes::INVALID_W_REF)
                        .component(component)
                        .element(tag_name)
                        .snippet(format!("w-ref=\"{val}\""))
                        .help(format!(
                            "use w-ref={{{val}}} with braces to bind to a component property"
                        ))
                        .into());
                }
            }
            out.push(' ');
            out.push_str(raw_attr);
            continue;
        }

        if let Some(bool_name) = name.strip_prefix('?') {
            if let Some(raw_value) = value.and_then(extract_single_handlebars) {
                meta.attr_bindings.push(CompiledAttrBinding::Boolean {
                    name: bool_name.to_string(),
                    condition: compile_condition_expr(&raw_value),
                });
                binding_count += 1;
            }
            continue;
        }

        if name.starts_with(':') {
            if let Some(raw_value) = value.and_then(extract_single_handlebars) {
                // Strip the ':' prefix and convert to camelCase for the JS property name.
                // The runtime uses el[name] = value directly — no conversion needed.
                let prop_name = webui_protocol::attrs::attribute_to_camel(
                    name.strip_prefix(':').unwrap_or(name),
                );
                meta.attr_bindings.push(CompiledAttrBinding::Complex {
                    name: prop_name,
                    value: raw_value,
                });
                binding_count += 1;
            }
            continue;
        }

        if let Some(raw_value) = value {
            if let Some(parts) = parse_attr_parts(raw_value) {
                let is_simple =
                    parts.len() == 1 && matches!(parts.first(), Some(CompiledAttrPart::Dynamic(_)));
                if is_simple {
                    if let Some(CompiledAttrPart::Dynamic(path)) = parts.into_iter().next() {
                        meta.attr_bindings.push(CompiledAttrBinding::Simple {
                            name: name.to_string(),
                            value: path,
                        });
                    }
                } else {
                    meta.attr_bindings.push(CompiledAttrBinding::Template {
                        name: name.to_string(),
                        parts,
                    });
                }
                binding_count += 1;
                continue;
            }
        }

        out.push(' ');
        out.push_str(raw_attr);
    }

    if binding_count == 1 {
        out.push_str(" data-w-b-");
        out.push_str(&binding_start.to_string());
    } else if binding_count > 1 {
        out.push_str(" data-w-c-");
        out.push_str(&binding_start.to_string());
        out.push('-');
        out.push_str(&binding_count.to_string());
    }

    let event_count = meta.events.len().saturating_sub(event_start);
    if event_count > 0 {
        out.push_str(" data-ev=\"");
        let _ = write!(out, "{}", event_count);
        out.push('"');
    }

    if self_closing {
        out.push_str(" />");
    } else {
        out.push('>');
    }

    Ok(Some((out, end + 1)))
}

enum EventHandler {
    Valid(String, Vec<EventArg>),
    Invalid(String),
    Empty,
}

fn parse_event_handler(raw_value: &str) -> EventHandler {
    let inner = raw_value
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(raw_value)
        .trim();
    if inner.is_empty() {
        return EventHandler::Empty;
    }
    match inner.find('(') {
        Some(paren) => {
            let close = match inner.rfind(')') {
                Some(close) if close > paren => close,
                _ => return EventHandler::Invalid(inner.to_string()),
            };
            if !inner[close + 1..].trim().is_empty() {
                return EventHandler::Invalid(inner.to_string());
            }
            let handler_name = inner[..paren].trim();
            if !is_valid_identifier(handler_name) {
                return EventHandler::Invalid(inner.to_string());
            }
            let Some(args) = parse_event_args(&inner[paren + 1..close]) else {
                return EventHandler::Invalid(inner.to_string());
            };
            EventHandler::Valid(handler_name.to_string(), args)
        }
        None => EventHandler::Invalid(inner.to_string()),
    }
}

fn parse_event_args(raw_args: &str) -> Option<Vec<EventArg>> {
    if raw_args.trim().is_empty() {
        return Some(Vec::new());
    }

    let raw_parts = split_event_args(raw_args)?;
    let mut args = Vec::with_capacity(raw_parts.len());
    for arg in raw_parts {
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            return None;
        }
        args.push(parse_event_arg(trimmed)?);
    }
    Some(args)
}

fn split_event_args(raw_args: &str) -> Option<Vec<&str>> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (idx, byte) in raw_args.bytes().enumerate() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == q {
                quote = None;
            }
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
            continue;
        }
        if byte == b',' {
            args.push(&raw_args[start..idx]);
            start = idx + 1;
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    args.push(&raw_args[start..]);
    Some(args)
}

fn parse_event_arg(arg: &str) -> Option<EventArg> {
    if arg.is_empty() {
        return None;
    }
    if arg == "e" {
        return Some(EventArg::Event);
    }
    if arg == "true" {
        return Some(EventArg::Bool(true));
    }
    if arg == "false" {
        return Some(EventArg::Bool(false));
    }
    if arg == "null" {
        return Some(EventArg::Null);
    }
    if let Some(value) = parse_quoted_event_string(arg) {
        return Some(EventArg::String(value));
    }
    if is_quoted_event_arg_start(arg) {
        return None;
    }
    if is_number_literal(arg) {
        return Some(EventArg::Number(arg.to_string()));
    }
    is_valid_event_path(arg).then(|| EventArg::Path(arg.to_string()))
}

fn parse_quoted_event_string(arg: &str) -> Option<String> {
    let bytes = arg.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if (quote != b'"' && quote != b'\'') || bytes[bytes.len() - 1] != quote {
        return None;
    }
    Some(
        arg[1..arg.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\'", "'"),
    )
}

fn is_number_literal(arg: &str) -> bool {
    !arg.is_empty()
        && arg.parse::<f64>().is_ok()
        && arg
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'))
}

fn is_quoted_event_arg_start(arg: &str) -> bool {
    matches!(arg.as_bytes().first(), Some(b'"' | b'\''))
}

fn is_valid_event_path(path: &str) -> bool {
    let mut parts = path.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    if !is_valid_identifier(first) {
        return false;
    }
    for part in parts {
        if part.is_empty() || (!is_valid_identifier(part) && !is_ascii_digits(part)) {
            return false;
        }
    }
    true
}

fn is_valid_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !is_identifier_start(first) {
        return false;
    }
    bytes.all(is_identifier_continue)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn extract_single_handlebars(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(inner) = trimmed
        .strip_prefix("{{{")
        .and_then(|s| s.strip_suffix("}}}"))
    {
        return Some(inner.trim().to_string());
    }

    if let Some(inner) = trimmed
        .strip_prefix("{{")
        .and_then(|s| s.strip_suffix("}}"))
    {
        return Some(inner.trim().to_string());
    }

    None
}

fn parse_attr_parts(value: &str) -> Option<Vec<CompiledAttrPart>> {
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut last = 0;
    let mut dynamic_found = false;
    let mut parts = Vec::new();

    while i < len {
        if i + 2 < len && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let brace_count = if i + 2 < len && bytes[i + 2] == b'{' {
                3
            } else {
                2
            };

            if let Some(end) = find_brace_end(value, i + brace_count, brace_count) {
                if last < i {
                    parts.push(CompiledAttrPart::Static(value[last..i].to_string()));
                }
                parts.push(CompiledAttrPart::Dynamic(
                    value[i + brace_count..end].trim().to_string(),
                ));
                dynamic_found = true;
                i = end + brace_count;
                last = i;
                continue;
            }
        }

        i += 1;
    }

    if !dynamic_found {
        return None;
    }

    if last < len {
        parts.push(CompiledAttrPart::Static(value[last..].to_string()));
    }

    Some(parts)
}

/// Extract `@event` bindings from the opening `<template>` tag.
///
/// These become "root events" (`re` array) attached to the host element
/// rather than to an element inside the shadow DOM.
fn extract_root_events(component: &str, html: &str) -> Result<Vec<EventBinding>> {
    if !html.starts_with("<template") {
        return Ok(Vec::new());
    }
    let Some(close) = find_tag_close(html) else {
        return Ok(Vec::new());
    };
    let tag = &html[..close];
    let mut events = Vec::new();
    let bytes = tag.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'@' {
            if let Some((name, handler, args, consumed)) = parse_event_attr(component, tag, i)? {
                events.push((name, handler, args));
                i += consumed;
                continue;
            }
        }
        i += 1;
    }

    Ok(events)
}

fn extract_adopted_stylesheet_specifier(html: &str) -> Option<String> {
    if !html.starts_with("<template") {
        return None;
    }
    let close = find_tag_close(html)?;
    let tag = &html[..close];
    let attr = "shadowrootadoptedstylesheets=\"";
    let start = tag.find(attr)? + attr.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: compile a template and unwrap. The vast majority of tests
    /// exercise valid templates; tests that assert on authoring errors call
    /// [`super::generate_compiled_template`] directly and inspect the `Result`.
    #[allow(clippy::disallowed_methods)]
    fn generate_compiled_template(tag_name: &str, html_content: &str) -> String {
        super::generate_compiled_template(tag_name, html_content).expect("valid template compiles")
    }

    #[allow(clippy::disallowed_methods)]
    fn generate_compiled_template_payload(
        tag_name: &str,
        html_content: &str,
    ) -> CompiledTemplatePayload {
        super::generate_compiled_template_with_root_source(
            tag_name,
            html_content,
            html_content,
            false,
            false,
        )
        .expect("valid template compiles")
    }

    fn assert_no_client_markers(result: &str) {
        assert!(!result.contains("<!--t:"), "text markers should be removed");
        assert!(
            !result.contains("<!--c:"),
            "conditional markers should be removed"
        );
        assert!(
            !result.contains("<!--r:"),
            "repeat markers should be removed"
        );
        assert!(
            !result.contains("data-w-b-"),
            "attribute binding markers should be removed"
        );
        assert!(
            !result.contains("data-w-c-"),
            "attribute binding range markers should be removed"
        );
        assert!(
            !result.contains("data-ev="),
            "event markers should be removed"
        );
    }

    #[test]
    fn test_skip_event_attributes() {
        let mut plugin = WebUIParserPlugin::new();
        assert_eq!(plugin.classify_attribute("@click"), AttributeAction::Skip);
        assert_eq!(plugin.classify_attribute("@keydown"), AttributeAction::Skip);
        assert_eq!(plugin.classify_attribute("w-ref"), AttributeAction::Keep);
        assert_eq!(plugin.classify_attribute("class"), AttributeAction::Keep);
    }

    #[test]
    fn test_metadata_has_text_bindings() {
        let result = generate_compiled_template("my-comp", "<h1>{{title}}</h1>");
        assert_no_client_markers(&result);
        assert!(result.contains(r#""h":"<h1></h1>""#));
        assert!(result.contains("\"title\""));
        assert!(result.contains(r#","tx":[[[[0],0],[["title"]]]]"#));
        assert!(!result.contains("{{"));
    }

    #[test]
    fn test_metadata_emits_template_roots_and_observed_attrs() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<h1>{{displayValue}}</h1><input ?readonly="{{readOnly}}" />"#,
        );

        assert!(result.contains(r#","tr":["displayValue","readOnly"]"#));
        assert!(result.contains(r#","ta":["display-value","displayValue","readonly","readOnly"]"#));
    }

    #[test]
    fn test_metadata_excludes_repeat_scope_roots() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="item in items"><p>{{item.title}} {{heading}}</p></for>"#,
        );
        assert!(result.contains(r#","tr":["items","heading"]"#));
    }

    #[test]
    fn test_metadata_emits_event_feature_flag() {
        let result =
            generate_compiled_template("my-comp", r#"<button @click="{onClick()}">Go</button>"#);

        assert!(result.contains(r#","tf":1"#));
    }

    #[test]
    fn test_metadata_strips_html_comments_without_processing_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<!-- {{title}} @click="{bad()}" --><div>hello</div>"#,
        );

        assert_no_client_markers(&result);
        assert!(result.contains(r#""h":"<div>hello</div>""#));
        assert!(!result.contains("title"));
        assert!(!result.contains("@click"));
        assert!(!result.contains("-->"));
    }

    #[test]
    fn test_metadata_strips_style_comments_without_processing_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<style>/* prose {{ignored}} */ .x { color: {{color}}; } /*! @license {{kept}} */</style>"#,
        );

        assert_no_client_markers(&result);
        assert!(!result.contains("ignored"));
        assert!(result.contains(r#"["color"]"#));
        assert!(!result.contains(r#"[["kept"]]"#));
        assert!(result.contains("/*! @license {{kept}} */"));
    }

    #[test]
    fn test_metadata_processes_style_signal_comments() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<style>:root{/*{{{tokens.light}}}*/}</style>"#,
        );

        assert_no_client_markers(&result);
        assert!(result.contains(r#"["tokens.light"]"#));
        assert!(!result.contains("/*"));
        assert!(!result.contains("*/"));
    }

    #[test]
    fn test_metadata_keeps_legal_style_comment_with_html_like_tag_as_raw_text() {
        let result = generate_compiled_template(
            "my-component",
            r#"<style>:host { display: block; }/*! @license The <my-component> element. */.container { padding: 16px; }</style><div>hello</div>"#,
        );

        assert_no_client_markers(&result);
        assert!(result.contains("@license The <my-component> element."));
        assert!(!result.contains("</my-component>"));
        assert!(result.contains("</style><div>hello</div>"));
    }

    #[test]
    fn test_metadata_keeps_marker_like_text_in_legal_style_comment_literal() {
        let result = generate_compiled_template(
            "my-component",
            r#"<p>{{title}}</p><style>/*! @license <!--t:0--> */.x { color: red; }</style>"#,
        );

        assert!(result.contains("<!--t:0-->"));
        assert!(result.contains(
            r#""h":"<p></p><style>/*! @license <!--t:0--> */.x { color: red; }</style>""#
        ));
        assert!(result.contains(r#","tx":[[[[0],0],[["title"]]]]"#));
        assert!(!result.contains(r#"[[[1],0],[["title"]]]"#));
    }

    #[test]
    fn test_metadata_keeps_marker_like_style_text_between_real_signal_markers() {
        let result = generate_compiled_template(
            "my-component",
            r#"<style>/*{{first}}*/📚/*! @license <!--t:0--> <my-component> *//*{{{second}}}*/</style><div>done</div>"#,
        );

        assert!(result.contains(r#"["first"]"#));
        assert!(result.contains(r#"["second"]"#));
        assert!(result.contains("📚/*! @license <!--t:0--> <my-component> */"));
        assert!(!result.contains("</my-component>"));
        assert!(result.contains("</style><div>done</div>"));
    }

    #[test]
    fn test_metadata_strips_style_line_comments_without_processing_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            "<style>// {{ignored}}\n.x { color: {{color}}; }\n//! @license {{kept}}</style>",
        );

        assert_no_client_markers(&result);
        assert!(!result.contains("ignored"));
        assert!(result.contains(r#"["color"]"#));
        assert!(!result.contains(r#"[["kept"]]"#));
        assert!(result.contains("//! @license {{kept}}"));
    }

    #[test]
    fn test_metadata_has_conditionals() {
        let payload = generate_compiled_template_payload(
            "my-comp",
            r#"<if condition="state == 'done'"><span>yes</span></if>"#,
        );
        let result = payload.template_json;
        assert_no_client_markers(&result);
        assert!(result.contains(r#""c":[[[0,["state"]],0,[[],0]]]"#));
        assert!(payload
            .template_functions
            .contains(r#"function(v,s){return v("state",s)=="done"}"#));
        assert!(result.contains("<span>yes</span>"));
        assert!(!result.contains("<if"));
    }

    #[test]
    fn test_metadata_has_conditionals_with_gt_in_condition_attr() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<if condition="count > 0"><span>yes</span></if>"#,
        );
        assert_no_client_markers(&result);
        assert!(
            result.contains(r#","c":["#),
            "conditional metadata expected"
        );
        assert!(result.contains("<span>yes</span>"));
        assert!(!result.contains("<if"));
    }

    #[test]
    fn test_parse_if_block_ignores_close_marker_inside_attr_value() {
        let input = r#"<if condition="count > 0"><div data-note="</if>">yes</div></if>"#;
        let (_, body, consumed) = parse_if_block(input).expect("if block should parse");

        assert_eq!(body, r#"<div data-note="</if>">yes</div>"#);
        assert_eq!(consumed, input.len());
    }

    #[test]
    fn test_metadata_has_for_loops() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="item in items"><p>{{item.name}}</p></for>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains("\"items\""));
        assert!(result.contains("\"item\""));
        assert!(!result.contains("<for"));
    }

    #[test]
    fn test_parse_for_block_ignores_open_marker_inside_attr_value() {
        let input =
            r#"<for each="item in items"><div data-note="<for fake>">{{item.name}}</div></for>"#;
        let (collection, item_var, body, consumed) =
            parse_for_block(input).expect("for block should parse");

        assert_eq!(collection, "items");
        assert_eq!(item_var, "item");
        assert_eq!(body, r#"<div data-note="<for fake>">{{item.name}}</div>"#);
        assert_eq!(consumed, input.len());
    }

    #[test]
    fn test_metadata_has_for_loops_with_gt_in_other_attr() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="item in items" data-note="a > b"><p>{{item.name}}</p></for>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains("\"items\""));
        assert!(result.contains("\"item\""));
        assert!(!result.contains("<for"));
        assert!(!result.contains("{{item.name}}"));
    }

    #[test]
    fn test_metadata_has_events() {
        let result =
            generate_compiled_template("my-comp", r#"<button @click="{onClick()}">Go</button>"#);
        assert_no_client_markers(&result);
        assert!(result.contains("\"click\""));
        assert!(result.contains("\"onClick\""));
        assert!(!result.contains("@click"));
    }

    #[test]
    fn test_metadata_has_events_unquoted() {
        let result =
            generate_compiled_template("my-comp", r#"<button @click={onClick()}>Go</button>"#);
        assert_no_client_markers(&result);
        assert!(result.contains("\"click\""));
        assert!(result.contains("\"onClick\""));
    }

    #[test]
    fn test_metadata_has_events_with_arg() {
        let result = generate_compiled_template("my-comp", r#"<input @keydown="{onKey(e)}" />"#);
        assert_no_client_markers(&result);
        assert!(result.contains("\"keydown\""));
        assert!(result.contains("\"onKey\""));
        assert!(result.contains(r#"["e"]"#)); // event argument
    }

    #[test]
    fn test_metadata_has_event_path_args() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<button @click="{selectItem(item.id, e, 'ok', 7, true, null)}">Go</button>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains(r#"["p","item.id"]"#));
        assert!(result.contains(r#"["e"]"#));
        assert!(result.contains(r#"["s","ok"]"#));
        assert!(result.contains(r#"["n",7]"#));
        assert!(result.contains(r#"["b",1]"#));
        assert!(result.contains(r#"["z"]"#));
    }

    #[test]
    fn test_metadata_has_simple_attr_bindings() {
        let result = generate_compiled_template("my-comp", r#"<input title="{{title}}" />"#);
        assert_no_client_markers(&result);
        assert!(result.contains(r#","a":["#));
        assert!(result.contains(r#","ag":[[[0],0,1]]"#));
        assert!(result.contains(r#"["title",0,"title"]"#));
        assert!(!result.contains(r#"title=\"<!--t:"#));
    }

    #[test]
    fn test_metadata_has_template_attr_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<a href="/product/{{handle}}" class="card {{variant}}">Go</a>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains(r#","a":["#));
        assert!(result.contains(r#","ag":[[[0],0,2]]"#));
        assert!(result.contains(r#"["href",3,["/product/",["handle"]]]"#));
        assert!(result.contains(r#"["class",3,["card ",["variant"]]]"#));
        assert!(!result.contains(r#"/product/<!--t:"#));
    }

    #[test]
    fn test_metadata_has_boolean_attr_bindings() {
        let payload = generate_compiled_template_payload(
            "my-comp",
            r#"<a ?data-active="{{page == 'dashboard'}}">Go</a>"#,
        );
        let result = payload.template_json;
        assert_no_client_markers(&result);
        assert!(result.contains(r#","a":["#));
        assert!(result.contains(r#","ag":[[[0],0,1]]"#));
        assert!(result.contains(r#"["data-active",2,[0,["page"]]]"#));
        assert!(payload
            .template_functions
            .contains(r#"function(v,s){return v("page",s)=="dashboard"}"#));
    }

    #[test]
    fn test_metadata_has_complex_attr_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<child-view :config="{{settings}}"></child-view>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains(r#","a":["#));
        assert!(result.contains(r#","ag":[[[0],0,1]]"#));
        assert!(result.contains(r#""config",1,"settings""#));
    }

    #[test]
    fn test_w_ref_stays_in_html() {
        let result = generate_compiled_template("my-comp", r#"<input w-ref="{myInput}" />"#);
        // w-ref stays in the static HTML — runtime binds from the DOM directly
        assert!(result.contains("w-ref"));
        assert!(result.contains("myInput"));
    }

    #[test]
    fn test_w_ref_unquoted_stays_in_html() {
        let result = generate_compiled_template("my-comp", r#"<span w-ref={myLabel}>0</span>"#);
        assert!(result.contains("w-ref"));
        assert!(result.contains("myLabel"));
    }

    #[test]
    fn test_w_ref_without_braces_is_fatal() {
        let err =
            super::generate_compiled_template("mail-inbox-page", r#"<input w-ref="myInput" />"#)
                .expect_err("non-braced w-ref must fail the build");
        assert!(err.to_string().contains("invalid w-ref binding"));
    }

    #[test]
    fn test_w_ref_error_names_component_and_element() {
        let err =
            super::generate_compiled_template("mail-inbox-page", r#"<input w-ref="myInput" />"#)
                .expect_err("non-braced w-ref must fail the build");
        assert!(err
            .to_string()
            .contains("component <mail-inbox-page> · element <input>"));
    }

    #[test]
    fn test_metadata_has_root_events() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open" @click="{onClick(e)}"><div>hi</div></template>"#,
        );
        assert!(result.contains(r#","re":["#));
        assert!(result.contains("\"click\""));
        assert!(result.contains("\"onClick\""));
    }

    #[test]
    fn test_strips_shadowrootmode() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open"><p>{{title}}</p></template>"#,
        );
        assert!(!result.contains("shadowrootmode"));
        assert_no_client_markers(&result);
        assert!(result.contains(r#","tx":[[[[0],0],[["title"]]]]"#));
    }

    #[test]
    fn test_root_template_allows_gt_in_quoted_attr_values() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open" data-note="a > b" @click="{onClick()}"><p>hi</p></template>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains(r#""h":"<p>hi</p>""#));
        assert!(result.contains(r#","re":[["click","onClick",[]]]"#));
    }

    #[test]
    fn test_outlet_converted() {
        let result = generate_compiled_template("my-comp", r#"<main><outlet /></main>"#);
        assert!(result.contains("<outlet></outlet>"));
    }

    #[test]
    fn test_emoji_safe() {
        let result = generate_compiled_template("my-comp", r#"<span>📚 {{title}}</span>"#);
        assert!(result.contains("📚"));
        assert_no_client_markers(&result);
        assert!(result.contains(r#","tx":[[[[0],0],["📚 ",["title"]]]]"#));
    }

    #[test]
    fn test_boolean_attr_in_for() {
        let payload = generate_compiled_template_payload(
            "my-comp",
            r#"<for each="s in sections"><a ?active="{{s.id == sectionId}}">{{s.name}}</a></for>"#,
        );
        let result = payload.template_json;
        assert_no_client_markers(&result);
        assert!(result.contains(r#"["active",2,[0,["s.id","sectionId"]]]"#));
        assert!(payload
            .template_functions
            .contains(r#"function(v,s){return v("s.id",s)==v("sectionId",s)}"#));
        assert!(!result.contains("?active"));
    }

    #[test]
    fn test_deduplicates_components() {
        let mut plugin = WebUIParserPlugin::new();
        let comp = Component {
            tag_name: "test-el".to_string(),
            html_content: "<p>hi</p>".to_string(),
            css_content: None,
            css_tokens: Vec::new(),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            source_path: std::path::PathBuf::new(),
            class_name: None,
            has_script: false,
        };
        plugin
            .register_component_template("test-el", &comp, &comp.html_content)
            .unwrap();
        plugin
            .register_component_template("test-el", &comp, &comp.html_content)
            .unwrap();
        assert_eq!(plugin.take_component_templates().unwrap().len(), 1);
    }

    #[test]
    fn test_scriptless_component_template_is_auto_element_marked() {
        let mut plugin = WebUIParserPlugin::new();
        let mut comp = Component {
            tag_name: "test-el".to_string(),
            html_content: "<p>hi</p>".to_string(),
            css_content: None,
            css_tokens: Vec::new(),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            source_path: std::path::PathBuf::new(),
            class_name: None,
            has_script: false,
        };

        plugin
            .register_component_template("test-el", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates().unwrap();
        assert!(templates[0].template_json.contains(r#","ae":1"#));

        comp.has_script = true;
        let mut plugin = WebUIParserPlugin::new();
        plugin
            .register_component_template("test-el", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates().unwrap();
        assert!(!templates[0].template_json.contains(r#","ae":1"#));
    }

    #[test]
    fn test_compiled_template_preserves_link_node_in_static_html() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open"><link rel="stylesheet" href="my-comp.css"><p>hi</p></template>"#,
        );
        assert!(result.contains(r#"rel=\"stylesheet\""#));
        assert!(result.contains(r#"href=\"my-comp.css\""#));
        assert!(result.contains(r#"<p>hi</p>"#));
        assert!(!result.contains(",css:"));
    }

    #[test]
    fn test_compiled_template_preserves_inline_style_in_static_html() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open"><style>.root{color:red}</style><p>hi</p></template>"#,
        );
        assert!(result.contains(r#""h":"<style>.root{color:red}</style><p>hi</p>""#));
    }

    #[test]
    fn test_compiled_template_emits_adopted_stylesheet_specifier() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open" shadowrootadoptedstylesheets="my-comp"><p>hi</p></template>"#,
        );
        assert!(result.contains(r#","sa":"my-comp""#));
    }

    #[test]
    fn test_plugin_uses_processed_link_template_html() {
        let mut plugin = WebUIParserPlugin::new();

        let comp = Component {
            tag_name: "test-el".to_string(),
            html_content: "<p>hi</p>".to_string(),
            css_content: Some(".root { color: red; }".to_string()),
            css_tokens: Vec::new(),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            source_path: std::path::PathBuf::new(),
            class_name: None,
            has_script: false,
        };

        plugin
            .register_component_template(
                "test-el",
                &comp,
                r#"<template shadowrootmode="open"><link rel="stylesheet" href="test-el.css"><p>hi</p></template>"#,
            )
            .unwrap();
        let templates = plugin.take_component_templates().unwrap();
        assert_eq!(templates.len(), 1);
        assert!(templates[0].template_json.contains(r#"rel=\"stylesheet\""#));
        assert!(templates[0]
            .template_json
            .contains(r#"href=\"test-el.css\""#));
        assert!(templates[0].template_json.contains(r#"<p>hi</p>"#));
    }

    #[test]
    fn test_plugin_preserves_root_events_from_raw_template_source() {
        let mut plugin = WebUIParserPlugin::new();

        let comp = Component {
            tag_name: "test-el".to_string(),
            html_content:
                r#"<template shadowrootmode="open" @click="{onClick(e)}"><p>hi</p></template>"#
                    .to_string(),
            css_content: Some(".root { color: red; }".to_string()),
            css_tokens: Vec::new(),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            source_path: std::path::PathBuf::new(),
            class_name: None,
            has_script: false,
        };

        plugin
            .register_component_template(
                "test-el",
                &comp,
                r#"<template shadowrootmode="open"><link rel="stylesheet" href="test-el.css"><p>hi</p></template>"#,
            )
            .unwrap();
        let templates = plugin.take_component_templates().unwrap();
        assert_eq!(templates.len(), 1);
        assert!(templates[0]
            .template_json
            .contains(r#","re":[["click","onClick",[["e"]]]]"#));
    }

    #[test]
    fn test_plugin_uses_processed_module_template_html() {
        let mut plugin = WebUIParserPlugin::new();

        let comp = Component {
            tag_name: "test-el".to_string(),
            html_content: "<p>hi</p>".to_string(),
            css_content: Some(".root { color: red; }".to_string()),
            css_tokens: Vec::new(),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            source_path: std::path::PathBuf::new(),
            class_name: None,
            has_script: false,
        };

        plugin
            .register_component_template(
                "test-el",
                &comp,
                r#"<template shadowrootmode="open" shadowrootadoptedstylesheets="test-el"><p>hi</p></template>"#,
            )
            .unwrap();
        let templates = plugin.take_component_templates().unwrap();
        assert_eq!(templates.len(), 1);
        assert!(templates[0].template_json.contains(r#","sa":"test-el""#));
    }

    #[test]
    fn test_nested_if_inside_for_compiles_nested_blocks() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="item in items"><if condition="item.active"><span>{{item.name}}</span></if></for>"#,
        );

        assert_no_client_markers(&result);
        assert!(
            result.contains(r#","b":["#),
            "compiled block table expected"
        );
        assert!(
            result.contains(r#"["items","item",0,[[],0]]"#),
            "repeat block index expected"
        );
        assert!(
            result.contains(r#"[[0,["item.active"]],1,[[],0]]"#),
            "nested conditional block index expected"
        );
        assert!(!result.contains("<if"), "nested if should be compiled");
        assert!(!result.contains("{{"), "nested bindings should be compiled");
    }

    #[test]
    fn test_for_inside_if_compiles_nested_blocks() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<if condition="showList"><for each="item in items"><p>{{item.name}}</p></for></if>"#,
        );

        assert_no_client_markers(&result);
        assert!(
            result.contains(r#","b":["#),
            "compiled block table expected"
        );
        assert!(
            result.contains(r#"[[0,["showList"]],0,[[],0]]"#),
            "conditional block index expected"
        );
        assert!(
            result.contains(r#"["items","item",1,[[],0]]"#),
            "nested repeat block index expected"
        );
        assert!(!result.contains("<for"), "nested for should be compiled");
        assert!(!result.contains("{{"), "nested bindings should be compiled");
    }

    #[test]
    fn test_nested_for_inside_for_compiles_nested_blocks() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="group in groups"><div><for each="item in group.items"><button data-group="{{group.name}}" ?disabled="{{item.disabled}}">{{item.label}}</button></for></div></for>"#,
        );

        assert_no_client_markers(&result);
        assert!(
            result.contains(r#","b":["#),
            "compiled block table expected"
        );
        assert!(
            result.contains(r#"["groups","group",0,[[],0]]"#),
            "outer repeat block index expected"
        );
        assert!(
            result.contains(r#"["group.items","item",1,[[0],0]]"#),
            "inner repeat block index expected"
        );
        assert!(!result.contains("<for"), "nested for should be compiled");
        assert!(!result.contains("{{"), "nested bindings should be compiled");
        assert!(
            !result.contains("?disabled"),
            "nested boolean attrs should be compiled"
        );
    }

    #[test]
    fn test_multiple_events_on_same_element() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<input @keydown="{onKey(e)}" @focus="{onFocus()}" />"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains("\"keydown\""), "keydown event expected");
        assert!(result.contains("\"onKey\""), "onKey handler expected");
        assert!(result.contains("\"focus\""), "focus event expected");
        assert!(result.contains("\"onFocus\""), "onFocus handler expected");
    }

    #[test]
    fn test_regular_tag_uses_single_event_marker_per_element() {
        let mut meta = TemplateSectionMeta::default();
        let (tag_html, _) = parse_regular_tag(
            "test-input",
            r#"<input @keydown="{onKey(e)}" @focus="{onFocus()}" />"#,
            &mut meta,
        )
        .expect("regular tag parse should succeed")
        .expect("regular tag should produce output");

        assert_eq!(tag_html, r#"<input data-ev="2" />"#);
    }

    #[test]
    fn test_empty_template() {
        let result = generate_compiled_template("my-comp", "");
        assert!(result.contains(r#""h":"""#), "empty html expected");
        // No optional arrays should be present
        assert!(!result.contains(r#","tx":"#), "no text bindings");
        assert!(!result.contains(r#","c":"#), "no conditionals");
        assert!(!result.contains(r#","r":"#), "no repeats");
        assert!(!result.contains(r#","e":"#), "no events");
        assert!(!result.contains(r#","re":"#), "no root events");
    }

    #[test]
    fn test_whitespace_only_template() {
        let result = generate_compiled_template("my-comp", "   \n  ");
        assert!(
            result.contains(r#""h":"""#),
            "whitespace-only should produce empty html"
        );
    }

    #[test]
    fn test_multiple_text_bindings() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<p>{{first}}</p><p>{{second}}</p><p>{{third}}</p>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains(
            r#","tx":[[[[0],0],[["first"]]],[[[1],0],[["second"]]],[[[2],0],[["third"]]]]"#
        ));
        assert!(result.contains("\"first\""));
        assert!(result.contains("\"second\""));
        assert!(result.contains("\"third\""));
    }

    #[test]
    fn test_multiple_conditionals() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<if condition="a"><span>A</span></if><if condition="b"><span>B</span></if>"#,
        );
        assert_no_client_markers(&result);
    }

    #[test]
    fn test_locator_paths_merge_adjacent_static_text_nodes() {
        let result = generate_compiled_template(
            "mp-product-grid",
            r#"
<template shadowrootmode="open">
  <if condition="products.length == 0 && query">
    <p class="empty-results">There are no products that match "<strong>{{query}}</strong>"</p>
  </if>
  <div class="grid">
    <for each="product in products">
      <mp-product-card
        handle="{{product.handle}}"
        title="{{product.title}}"
        price="{{product.price}}"
        gradient="{{product.gradient}}"
        image-url="{{product.imageUrl}}"
        image-loading="lazy"
        variant="grid"
      ></mp-product-card>
    </for>
  </div>
</template>
"#,
        );

        assert_no_client_markers(&result);
    }

    #[test]
    fn test_mixed_bindings_events_refs() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<button w-ref="{myBtn}" @click="{onClick()}">{{label}}</button>"#,
        );
        // w-ref stays in HTML (quotes escaped inside JS string)
        assert_no_client_markers(&result);
        assert!(result.contains("w-ref"));
        assert!(result.contains("myBtn"));
        // event compiled
        assert!(result.contains("\"click\""));
        assert!(result.contains("\"onClick\""));
        // text binding compiled
        assert!(result.contains(r#","tx":[[[[0],0],[["label"]]]]"#));
        assert!(result.contains("\"label\""));
    }

    #[test]
    fn test_event_without_e_arg() {
        let result =
            generate_compiled_template("my-comp", r#"<button @click="{onClick()}">Go</button>"#);
        assert!(
            result.contains(r#"["click","onClick",[],[0]]"#),
            "empty argument list should be preserved"
        );
    }

    #[test]
    fn test_root_event_without_e_arg() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<template shadowrootmode="open" @submit="{onSubmit()}"><form>hi</form></template>"#,
        );
        assert!(result.contains(r#","re":["#));
        assert!(result.contains("\"submit\""));
        assert!(result.contains("\"onSubmit\""));
        assert!(
            result.contains(r#"["submit","onSubmit",[]]"#),
            "empty root argument list should be preserved"
        );
    }

    #[test]
    fn test_triple_brace_raw_binding() {
        let result = generate_compiled_template("my-comp", r#"<div>{{{rawHtml}}}</div>"#);
        assert_no_client_markers(&result);
        assert!(
            result.contains(r#","tx":[[[[0],0],[["rawHtml"]],1]]"#),
            "triple brace produces text locator with raw flag: {result}"
        );
        assert!(
            result.contains("\"rawHtml\""),
            "rawHtml path in text bindings"
        );
    }

    #[test]
    fn test_for_with_complex_body() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<for each="user in users"><div class="card"><h2>{{user.name}}</h2><p>{{user.bio}}</p></div></for>"#,
        );
        assert_no_client_markers(&result);
        assert!(result.contains("\"users\""), "collection path");
        assert!(result.contains("\"user\""), "item variable");
        assert!(
            result.contains(r#","b":["#),
            "compiled block table expected"
        );
        assert!(
            !result.contains("{{user.name}}"),
            "bindings should be compiled"
        );
        assert!(
            !result.contains("{{user.bio}}"),
            "bindings should be compiled"
        );
    }

    #[test]
    fn test_mixed_text_uses_text_runs() {
        let result = generate_compiled_template("my-comp", r#"<p>Hello {{name}}!</p>"#);

        assert_no_client_markers(&result);
        assert!(result.contains(r#""h":"<p></p>""#));
        assert!(result.contains(r#","tx":[[[[0],0],["Hello ",["name"],"!"]]]"#));
    }

    #[test]
    fn test_js_string_escaping() {
        let payload = generate_compiled_template_payload(
            "my-comp",
            r#"<if condition="name == &quot;test&quot;"><span>ok</span></if>"#,
        );
        assert!(
            payload.template_json.contains(r#","c":["#),
            "conditional array present"
        );
        // The right-side literal should remain properly escaped in the AST payload.
        assert!(
            payload
                .template_functions
                .contains(r#"function(v,s){return v("name",s)==v("&quot;test&quot;",s)}"#),
            "condition function with escaped quotes should be present: {}",
            payload.template_functions
        );
        assert!(
            !payload.template_json.contains("function(v,s)"),
            "metadata JSON should not contain executable functions"
        );
    }

    #[test]
    fn test_js_string_escapes_script_breakout_sequences() {
        let result = generate_compiled_template(
            "my-comp",
            r#"<div title="</script><script>alert(1)</script>"></div>"#,
        );

        assert!(
            result.contains(r#"\u003C/script><script>alert(1)\u003C/script>"#),
            "script-closing sequences should be neutralized inside metadata strings: {result}"
        );
        assert!(
            !result.contains(r#"title=\"</script><script>alert(1)</script>\""#),
            "raw </script> must not appear inside serialized metadata strings: {result}"
        );
    }

    #[test]
    fn test_js_string_escapes_line_separator_codepoints() {
        let html = format!("<p>before{}middle{}after</p>", '\u{2028}', '\u{2029}');
        let result = generate_compiled_template("my-comp", &html);

        assert!(
            result.contains(r#"\u2028"#),
            "U+2028 should be escaped in metadata strings: {result}"
        );
        assert!(
            result.contains(r#"\u2029"#),
            "U+2029 should be escaped in metadata strings: {result}"
        );
    }

    #[test]
    fn test_on_element_parsed_encodes_12_bytes() {
        let mut plugin = WebUIParserPlugin::new();
        // Simulate 2 event attributes skipped
        let _ = plugin.classify_attribute("@click");
        let _ = plugin.classify_attribute("@keydown");
        // Call on_element_parsed with 3 binding attributes
        let data = plugin.finish_element(3);
        assert!(data.is_some());
        let bytes = data.unwrap();
        assert_eq!(bytes.len(), 12, "plugin data should be 12 bytes");
        let binding_count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let event_start = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let event_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        assert_eq!(binding_count, 3);
        assert_eq!(event_start, 0);
        assert_eq!(event_count, 2);
    }

    #[test]
    fn test_on_element_parsed_no_data_when_empty() {
        let mut plugin = WebUIParserPlugin::new();
        let data = plugin.finish_element(0);
        assert!(
            data.is_none(),
            "no plugin data when no bindings and no events"
        );
    }

    #[test]
    fn test_on_element_parsed_events_only() {
        let mut plugin = WebUIParserPlugin::new();
        let _ = plugin.classify_attribute("@click");
        let data = plugin.finish_element(0);
        assert!(
            data.is_some(),
            "should emit data for events even with 0 bindings"
        );
        let bytes = data.unwrap();
        let binding_count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let event_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        assert_eq!(binding_count, 0);
        assert_eq!(event_count, 1);
    }

    #[test]
    fn test_event_index_increments_across_elements() {
        let mut plugin = WebUIParserPlugin::new();
        // First element: 2 events
        let _ = plugin.classify_attribute("@click");
        let _ = plugin.classify_attribute("@focus");
        let data1 = plugin.finish_element(0).unwrap();
        let ev_start_1 = u32::from_le_bytes([data1[4], data1[5], data1[6], data1[7]]);
        assert_eq!(ev_start_1, 0, "first element starts at event 0");

        // Second element: 1 event
        let _ = plugin.classify_attribute("@blur");
        let data2 = plugin.finish_element(0).unwrap();
        let ev_start_2 = u32::from_le_bytes([data2[4], data2[5], data2[6], data2[7]]);
        assert_eq!(
            ev_start_2, 2,
            "second element starts at event 2 (after first element's 2 events)"
        );
    }

    #[test]
    fn test_text_run_decodes_html_entities_in_static_parts() {
        // Template: `<nav>{{sectionName}} &gt; {{topicName}}</nav>`
        // The `&gt;` is a static part between two dynamic bindings.
        // On the client, $resolveAttrParts concatenates parts and sets
        // textContent, which does NOT decode entities — so the compiler
        // must decode them in the metadata.
        let result =
            generate_compiled_template("my-comp", "<nav>{{sectionName}} &gt; {{topicName}}</nav>");
        assert_no_client_markers(&result);
        // The static part " > " should be decoded, not " &gt; "
        assert!(
            result.contains(r#"" > ""#),
            "static text run part should contain decoded '>' not '&gt;': {}",
            result
        );
        assert!(
            !result.contains(r#"" &gt; ""#),
            "static text run part should not contain raw '&gt;': {}",
            result
        );
    }

    #[test]
    fn test_invalid_event_handler_error_names_component_and_element() {
        let err = super::generate_compiled_template(
            "mail-inbox-page",
            r#"<button @pointerdown="e.preventDefault()">x</button>"#,
        )
        .expect_err("invalid @event handler must fail the build");
        assert!(err
            .to_string()
            .contains("component <mail-inbox-page> · element <button>"));
    }

    #[test]
    fn test_invalid_root_event_handler_error_names_component() {
        let err = super::generate_compiled_template(
            "my-card",
            r#"<template shadowrootmode="open" @click="e.stopPropagation()"><slot></slot></template>"#,
        )
        .expect_err("invalid root @event handler must fail the build");
        assert!(err.to_string().contains("in component <my-card>"));
    }

    #[test]
    fn test_parse_event_handler_bare_name_is_invalid() {
        assert!(matches!(
            parse_event_handler("{closeMenu}"),
            EventHandler::Invalid(ref name) if name == "closeMenu"
        ));
    }

    #[test]
    fn test_parse_event_attr_bare_name_errors() {
        let input = r#"<button @click="{closeMenu}">Click</button>"#;
        let err = parse_event_attr("test-button", input, 8)
            .expect_err("bare handler name must be rejected");
        assert!(err.to_string().contains("invalid @click handler"));
    }

    #[test]
    fn test_parse_event_handler_with_parens() {
        assert!(matches!(
            parse_event_handler("{onClick()}"),
            EventHandler::Valid(ref name, ref args) if name == "onClick" && args.is_empty()
        ));
    }

    #[test]
    fn test_parse_event_handler_with_event_arg() {
        assert!(matches!(
            parse_event_handler("{onClick(e)}"),
            EventHandler::Valid(ref name, ref args) if name == "onClick" && args == &vec![EventArg::Event]
        ));
    }

    #[test]
    fn test_parse_event_handler_with_mixed_args() {
        assert!(matches!(
            parse_event_handler("{onClick(item.id, e, 'ok', 7, false, null)}"),
            EventHandler::Valid(ref name, ref args)
                if name == "onClick"
                    && args == &vec![
                        EventArg::Path("item.id".to_string()),
                        EventArg::Event,
                        EventArg::String("ok".to_string()),
                        EventArg::Number("7".to_string()),
                        EventArg::Bool(false),
                        EventArg::Null,
                    ]
        ));
    }

    #[test]
    fn test_parse_event_handler_rejects_trailing_tokens() {
        assert!(matches!(
            parse_event_handler("{onClick(e)} trailing"),
            EventHandler::Invalid(ref raw) if raw == "{onClick(e)} trailing"
        ));
        assert!(matches!(
            parse_event_handler("{onClick(e) trailing}"),
            EventHandler::Invalid(ref raw) if raw == "onClick(e) trailing"
        ));
    }

    #[test]
    fn test_parse_event_handler_rejects_empty_args() {
        assert!(matches!(
            parse_event_handler("{onClick(, e)}"),
            EventHandler::Invalid(ref raw) if raw == "onClick(, e)"
        ));
        assert!(matches!(
            parse_event_handler("{onClick(e,)}"),
            EventHandler::Invalid(ref raw) if raw == "onClick(e,)"
        ));
    }

    #[test]
    fn test_parse_event_handler_rejects_malformed_args() {
        assert!(matches!(
            parse_event_handler("{onClick('unterminated)}"),
            EventHandler::Invalid(ref raw) if raw == "onClick('unterminated)"
        ));
        assert!(matches!(
            parse_event_handler("{onClick(other())}"),
            EventHandler::Invalid(ref raw) if raw == "onClick(other())"
        ));
        assert!(matches!(
            parse_event_handler("{onClick(count + 1)}"),
            EventHandler::Invalid(ref raw) if raw == "onClick(count + 1)"
        ));
    }

    #[test]
    fn test_parse_event_handler_rejects_invalid_handler_name() {
        assert!(matches!(
            parse_event_handler("{handler.name()}"),
            EventHandler::Invalid(ref raw) if raw == "handler.name()"
        ));
        assert!(matches!(
            parse_event_handler("{1handler()}"),
            EventHandler::Invalid(ref raw) if raw == "1handler()"
        ));
    }

    #[test]
    fn test_parse_event_handler_empty_value() {
        assert!(matches!(parse_event_handler("{}"), EventHandler::Empty));
    }

    #[test]
    fn test_attribute_to_camel_aria() {
        use webui_protocol::attrs::attribute_to_camel;
        assert_eq!(attribute_to_camel("aria-describedby"), "ariaDescribedBy");
        assert_eq!(attribute_to_camel("aria-labelledby"), "ariaLabelledBy");
        assert_eq!(
            attribute_to_camel("aria-activedescendant"),
            "ariaActiveDescendant"
        );
        assert_eq!(attribute_to_camel("aria-label"), "ariaLabel");
        assert_eq!(attribute_to_camel("aria-hidden"), "ariaHidden");
    }

    #[test]
    fn test_attribute_to_camel_html_global() {
        use webui_protocol::attrs::attribute_to_camel;
        assert_eq!(attribute_to_camel("readonly"), "readOnly");
        assert_eq!(attribute_to_camel("tabindex"), "tabIndex");
        assert_eq!(attribute_to_camel("accesskey"), "accessKey");
        assert_eq!(attribute_to_camel("contenteditable"), "contentEditable");
        assert_eq!(attribute_to_camel("inputmode"), "inputMode");
        assert_eq!(attribute_to_camel("maxlength"), "maxLength");
        assert_eq!(attribute_to_camel("formaction"), "formAction");
    }
}
