// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST 2 parser plugin for the WebUI parser.
//!
//! Tracks component definitions during HTML parsing and returns `<f-template>`
//! artifacts after parsing. Converts WebUI Framework template syntax (`<if>`, `<for>`, `{{}}`)
//! into FAST-compatible syntax (`<f-when>`, `<f-repeat>`, `{}`).

use super::{AttributeAction, ParserPlugin, ParserPluginArtifacts};
use crate::component_registry::Component;
use crate::{CssStrategy, Result};
use webui_protocol::FastElementData;

/// Information about a tracked component for `<f-template>` generation.
struct TrackedComponent {
    tag_name: String,
    html_content: String,
    css_content: Option<String>,
}

/// Deprecated FAST 2 parser plugin used by `fast` and `fast-v2`.
///
/// Implements the `ParserPlugin` trait for FAST 2 compatibility:
/// - Filters FAST-specific runtime binding attributes (`@click`, `f-ref`, etc.)
/// - Tracks components encountered during parsing
/// - Returns `<f-template>` artifacts with converted FAST syntax after parsing
/// - Emits binding attribute counts as `Plugin` protocol fragment data
pub struct FastV2ParserPlugin {
    /// Components tracked during parsing, in discovery order.
    components: Vec<TrackedComponent>,
    /// CSS delivery strategy for f-templates.
    css_strategy: CssStrategy,
}

impl FastV2ParserPlugin {
    /// Create a new FAST 2 parser plugin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
            css_strategy: CssStrategy::Link,
        }
    }

    /// Set the CSS delivery strategy for generated f-templates.
    pub fn set_css_strategy(&mut self, strategy: CssStrategy) {
        self.css_strategy = strategy;
    }

    /// Take the individual component f-template strings, keyed by tag name.
    ///
    /// Each value is a complete `<f-template name="tag-name">...</f-template>` string
    /// ready to be appended to a document. This is used by the JSON partial render
    /// endpoint to send only the templates the client needs.
    #[must_use]
    pub fn take_component_templates(&self) -> Vec<(String, String)> {
        self.components
            .iter()
            .map(|comp| {
                let tmpl = generate_f_template(
                    &comp.tag_name,
                    &comp.html_content,
                    comp.css_content.as_deref(),
                    self.css_strategy,
                );
                (comp.tag_name.clone(), tmpl)
            })
            .collect()
    }
}

impl Default for FastV2ParserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserPlugin for FastV2ParserPlugin {
    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        _processed_template: &str,
    ) -> Result<()> {
        // Only track each component once (avoids duplicate <f-template> blocks
        // when a component is used in multiple parent templates)
        if self.components.iter().any(|c| c.tag_name == tag_name) {
            return Ok(());
        }
        self.components.push(TrackedComponent {
            tag_name: tag_name.to_string(),
            html_content: component.html_content.clone(),
            css_content: component.css_content.clone(),
        });
        Ok(())
    }

    fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction {
        if attr_name.starts_with('@')
            || attr_name == "f-ref"
            || attr_name == "f-slotted"
            || attr_name == "f-children"
        {
            AttributeAction::SkipAndCountBinding
        } else {
            AttributeAction::Keep
        }
    }

    fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
        if binding_attribute_count > 0 {
            Some(
                FastElementData {
                    binding_count: binding_attribute_count,
                }
                .encode()
                .to_vec(),
            )
        } else {
            None
        }
    }

    fn into_artifacts(self: Box<Self>) -> ParserPluginArtifacts {
        ParserPluginArtifacts::ComponentTemplates(self.take_component_templates())
    }
}

/// Generate a single f-template HTML string from component data.
///
/// Used by the server to generate templates on demand for route components
/// that weren't encountered during initial parsing.
///
/// Any `shadowroot`-prefixed attributes on the user-supplied wrapping
/// `<template>` (e.g. `shadowrootmode`, `shadowrootclonable`,
/// `shadowrootadoptedstylesheets`) are moved onto the outer
/// `<f-template>` element rather than the inner `<template>`. The inner
/// `<template>` always has every `shadowroot*` attribute stripped, to
/// prevent the browser from auto-activating it as a declarative shadow
/// root inside the f-template body. When CSS Module strategy generates
/// a `shadowrootadoptedstylesheets` specifier, it is merged with any
/// user-supplied value (space-separated, de-duplicated) and emitted on
/// the `<f-template>`.
pub fn generate_f_template(
    tag_name: &str,
    html_content: &str,
    css_content: Option<&str>,
    css_strategy: CssStrategy,
) -> String {
    let mut output = String::with_capacity(256);

    // Pre-scan the user's outer <template> wrapper (if any) to capture
    // every shadowroot* attribute — these go on <f-template>, not on the
    // inner <template>. We also capture any user-supplied
    // shadowrootadoptedstylesheets value so we can merge it with the
    // parser-generated specifier under CSS Module strategy.
    let outer = extract_outer_template_shadowroot_attrs(html_content.trim());

    // Emit the <f-template> opening tag with collected shadowroot* attrs
    // and (under Module strategy) the merged adopted-stylesheets value.
    output.push_str("<f-template name=\"");
    output.push_str(tag_name);
    output.push('"');
    for attr in &outer.shadowroot_attrs_other {
        output.push(' ');
        output.push_str(attr);
    }
    let parser_adopted = if css_strategy == CssStrategy::Module && css_content.is_some() {
        Some(tag_name)
    } else {
        None
    };
    if let Some(merged) = merge_adopted_stylesheets(outer.adopted.as_deref(), parser_adopted) {
        output.push_str(" shadowrootadoptedstylesheets=\"");
        output.push_str(&merged);
        output.push('"');
    }
    output.push_str(">\n");

    let converted = convert_btr_to_fast(html_content);
    let trimmed = minify_inter_tag_whitespace(converted.trim());

    // Build the CSS injection string based on the configured strategy
    let css_injection = match css_strategy {
        CssStrategy::Link => css_content.map(|_| {
            let mut s = String::with_capacity(40 + tag_name.len());
            s.push_str("<link rel=\"stylesheet\" href=\"");
            s.push_str(tag_name);
            s.push_str(".css\">");
            s
        }),
        CssStrategy::Style => css_content.map(|css| {
            let mut s = String::with_capacity(15 + css.len());
            s.push_str("<style>");
            s.push_str(css.trim());
            s.push_str("</style>");
            s
        }),
        CssStrategy::Module => None,
    };

    if trimmed.starts_with("<template") {
        if let Some(close_pos) = find_tag_close(&trimmed) {
            output.push_str(&trimmed[..close_pos]);
            output.push('>');
            if let Some(ref injection) = css_injection {
                output.push_str(injection);
            }
            output.push_str(&trimmed[close_pos + 1..]);
        } else {
            output.push_str(&trimmed);
        }
    } else {
        output.push_str("<template>");
        if let Some(ref injection) = css_injection {
            output.push_str(injection);
        }
        output.push_str(&trimmed);
        output.push_str("</template>");
    }

    output.push_str("\n</f-template>\n");
    output
}

/// Captured `shadowroot*` attribute state from a user-supplied outer
/// `<template>` wrapper.
#[derive(Default, Debug)]
struct OuterTemplateShadowRootAttrs {
    /// Raw `shadowroot*` attribute tokens to echo back onto the
    /// `<f-template>` (e.g. `shadowrootmode="open"`,
    /// `shadowrootclonable`).
    ///
    /// Excludes `shadowrootadoptedstylesheets` because that one is
    /// always handled separately to merge with parser-generated values.
    shadowroot_attrs_other: Vec<String>,
    /// Value of `shadowrootadoptedstylesheets`, when present.
    adopted: Option<String>,
}

/// Pre-scan the input's outer `<template>` opening tag and collect every
/// `shadowroot`-prefixed attribute. Returns an empty result when the
/// input does not start with a `<template>` wrapper or the tag is
/// malformed. Uses [`find_tag_close`] so values containing an unquoted
/// `>` (e.g. `data-note="a > b"`) are handled correctly.
fn extract_outer_template_shadowroot_attrs(html: &str) -> OuterTemplateShadowRootAttrs {
    let mut out = OuterTemplateShadowRootAttrs::default();
    if !html.starts_with("<template") {
        return out;
    }
    let Some(close) = find_tag_close(html) else {
        return out;
    };
    // Inner attribute span is between `<template` and the closing `>` (or `/>`).
    let attr_start = "<template".len();
    let mut attr_end = close;
    if attr_end > 0 && html.as_bytes()[attr_end - 1] == b'/' {
        attr_end -= 1;
    }
    if attr_end <= attr_start {
        return out;
    }
    let inner = &html[attr_start..attr_end];
    let inner_bytes = inner.as_bytes();
    let inner_len = inner_bytes.len();
    let mut j = 0;
    while j < inner_len {
        if is_whitespace(inner_bytes[j]) {
            j += 1;
            continue;
        }
        let name_start = j;
        while j < inner_len && inner_bytes[j] != b'=' && !is_whitespace(inner_bytes[j]) {
            j += 1;
        }
        let name_end = j;
        let name = &inner[name_start..name_end];

        let value_end = if j < inner_len && inner_bytes[j] == b'=' {
            j += 1;
            if j < inner_len && (inner_bytes[j] == b'"' || inner_bytes[j] == b'\'') {
                let quote = inner_bytes[j];
                j += 1;
                while j < inner_len && inner_bytes[j] != quote {
                    j += 1;
                }
                if j < inner_len {
                    j += 1;
                }
            } else {
                while j < inner_len && !is_whitespace(inner_bytes[j]) {
                    j += 1;
                }
            }
            j
        } else {
            name_end
        };

        if !name.starts_with("shadowroot") {
            continue;
        }

        if name == "shadowrootadoptedstylesheets" {
            out.adopted = Some(
                extract_attr_value_raw(&inner[name_start..value_end])
                    .unwrap_or("")
                    .to_string(),
            );
            continue;
        }

        // Preserve the raw `name` or `name="value"` token verbatim so
        // boolean attrs (e.g. `shadowrootclonable`) round-trip correctly.
        out.shadowroot_attrs_other
            .push(inner[name_start..value_end].to_string());
    }

    out
}

/// Extract the value portion of a raw `name="value"` (or `name='value'`)
/// attribute token. Returns `None` when no `=` is present (boolean attr)
/// or the token is malformed. The returned slice excludes surrounding
/// quotes.
fn extract_attr_value_raw(raw: &str) -> Option<&str> {
    let eq_pos = raw.find('=')?;
    let after = &raw[eq_pos + 1..];
    let trimmed = after.trim_start();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        return stripped.strip_suffix('"');
    }
    if let Some(stripped) = trimmed.strip_prefix('\'') {
        return stripped.strip_suffix('\'');
    }
    Some(trimmed)
}

/// Merge two `shadowrootadoptedstylesheets` value strings into a single
/// space-separated, de-duplicated list. Returns `None` when both inputs
/// are absent (or only contain whitespace). Tokens from `user` come
/// first in their original order, then unique tokens from `parser`.
fn merge_adopted_stylesheets(user: Option<&str>, parser: Option<&str>) -> Option<String> {
    let mut tokens: Vec<&str> = Vec::new();
    if let Some(u) = user {
        for tok in u.split_whitespace() {
            if !tokens.contains(&tok) {
                tokens.push(tok);
            }
        }
    }
    if let Some(p) = parser {
        for tok in p.split_whitespace() {
            if !tokens.contains(&tok) {
                tokens.push(tok);
            }
        }
    }
    if tokens.is_empty() {
        return None;
    }
    let total: usize = tokens.iter().map(|t| t.len() + 1).sum();
    let mut out = String::with_capacity(total);
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(t);
    }
    Some(out)
}

/// Convert WebUI Framework template syntax to FAST syntax in HTML content.
///
/// Performs the following transformations without regex:
/// - `<if condition="EXPR">` → `<f-when value="{{EXPR}}">`
/// - `</if>` → `</f-when>`
/// - `<for each="EXPR">` → `<f-repeat value="{{EXPR}}">`
/// - `</for>` → `</f-repeat>`
/// - `{{expr}}` inside `:attr` complex attribute values → `{expr}`
/// - Strips every `shadowroot`-prefixed attribute from any `<template>`
///   tag (e.g. `shadowrootmode`, `shadowrootclonable`,
///   `shadowrootadoptedstylesheets`). In f-template context the inner
///   `<template>` must not carry these attributes — they belong on the
///   outer `<f-template>` element so the browser does not auto-activate
///   the inner template as a declarative shadow root.
fn convert_btr_to_fast(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            if let Some(consumed) = try_convert_tag(input, i, &mut result) {
                i += consumed;
                continue;
            }
        }

        // Check for {{expr}} inside :attr values — handled at attribute level
        // in try_convert_tag. Outside of tags, double-braces are left as-is
        // (they're text content interpolation for FAST, kept unchanged).
        i = push_char_at(input, i, &mut result);
    }

    result
}

/// Try to convert a tag starting at position `pos`. Returns `Some(bytes_consumed)`
/// if a conversion was performed, or `None` if this is not a convertible tag.
fn try_convert_tag(input: &str, pos: usize, result: &mut String) -> Option<usize> {
    let remaining = &input[pos..];

    // Check for closing tags first (shorter patterns)
    if remaining.starts_with("</if>") {
        result.push_str("</f-when>");
        return Some(5);
    }
    if remaining.starts_with("</for>") {
        result.push_str("</f-repeat>");
        return Some(6);
    }
    // <route> in f-templates becomes empty <webui-route> (client router mounts components)
    if remaining.starts_with("</route>") {
        result.push_str("</webui-route>");
        return Some(8);
    }

    // <outlet /> is kept as a marker element in f-templates.
    // The client router finds it and replaces it with <webui-route> stubs.
    if starts_with_tag_name(remaining, "outlet") {
        if let Some(close) = remaining.find('>') {
            result.push_str("<outlet></outlet>");
            return Some(close + 1);
        }
    }

    // Check for <if condition="...">
    if starts_with_tag_name(remaining, "if") {
        if let Some(consumed) = convert_if_tag(remaining, result) {
            return Some(consumed);
        }
    }

    // Check for <for each="...">
    if starts_with_tag_name(remaining, "for") {
        if let Some(consumed) = convert_for_tag(remaining, result) {
            return Some(consumed);
        }
    }

    // Check for <route ...> → <webui-route ...> in f-templates
    if starts_with_tag_name(remaining, "route") {
        if let Some(consumed) = convert_route_tag(remaining, result) {
            return Some(consumed);
        }
    }

    // Check for tags with :attr="{{expr}}" complex attribute values
    if remaining.starts_with("<") {
        // Strip every shadowroot* attr from <template> tags so the inner
        // template inside an f-template is never auto-activated as a
        // declarative shadow root by the browser.
        if starts_with_tag_name(remaining, "template") {
            if let Some(consumed) = strip_shadowroot_attrs(remaining, result) {
                return Some(consumed);
            }
        }
        if let Some(consumed) = convert_complex_attrs(remaining, result) {
            return Some(consumed);
        }
    }

    None
}

/// Check if `s` starts with `<name` followed by whitespace or `>`.
fn starts_with_tag_name(s: &str, name: &str) -> bool {
    if !s.starts_with('<') {
        return false;
    }
    let after_bracket = &s[1..];
    if !after_bracket.starts_with(name) {
        return false;
    }
    let after_name = &after_bracket[name.len()..];
    if after_name.is_empty() {
        return false;
    }
    let next = after_name.as_bytes()[0];
    next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' || next == b'>'
}

/// Convert `<if condition="EXPR">` to `<f-when value="{{EXPR}}">`.
/// Returns bytes consumed on success.
fn convert_if_tag(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>' outside of quoted attribute values
    let close = find_tag_close(tag_str)?;
    let tag_content = &tag_str[..=close];

    // Find condition="..." attribute
    let attr_value = extract_attribute_value(tag_content, "condition")?;

    result.push_str("<f-when value=\"{{");
    result.push_str(attr_value);
    result.push_str("}}\">");

    Some(close + 1)
}

/// Convert `<for each="EXPR">` to `<f-repeat value="{{EXPR}}">`.
/// Returns bytes consumed on success.
fn convert_for_tag(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>' outside of quoted attribute values
    let close = find_tag_close(tag_str)?;
    let tag_content = &tag_str[..=close];

    // Find each="..." attribute
    let attr_value = extract_attribute_value(tag_content, "each")?;

    result.push_str("<f-repeat value=\"{{");
    result.push_str(attr_value);
    result.push_str("}}\">");

    Some(close + 1)
}

/// Convert `<route ...>` to `<webui-route ...>` in f-templates.
/// Self-closing routes become `<webui-route ...></webui-route>` (empty).
/// The component attribute is preserved for the client router.
fn convert_route_tag(tag_str: &str, result: &mut String) -> Option<usize> {
    let close = find_tag_close(tag_str)?;
    let is_self_closing = tag_str[..=close].ends_with("/>");
    let tag = &tag_str[..=close];

    result.push_str("<webui-route");

    if let Some(v) = extract_attribute_value(tag, "path") {
        result.push_str(" path=\"");
        result.push_str(v);
        result.push('"');
    }
    if let Some(v) = extract_attribute_value(tag, "name") {
        result.push_str(" name=\"");
        result.push_str(v);
        result.push('"');
    }
    if let Some(v) = extract_attribute_value(tag, "component") {
        result.push_str(" component=\"");
        result.push_str(v);
        result.push('"');
    }
    if let Some(v) = extract_attribute_value(tag, "redirectTo") {
        result.push_str(" redirectto=\"");
        result.push_str(v);
        result.push('"');
    }
    if tag.contains(" exact") {
        result.push_str(" exact");
    }
    if tag.contains(" layout") {
        result.push_str(" layout");
    }
    if let Some(v) = extract_attribute_value(tag, "query") {
        result.push_str(" query=\"");
        result.push_str(v);
        result.push('"');
    }
    result.push_str(" style=\"display:none\"");

    if is_self_closing {
        result.push_str("></webui-route>");
    } else {
        result.push('>');
    }

    Some(close + 1)
}

/// Find the position of the tag-closing `>` that is NOT inside a quoted
/// attribute value.  Returns `None` when no unquoted `>` is found.
fn find_tag_close(tag_str: &str) -> Option<usize> {
    let bytes = tag_str.as_bytes();
    let mut in_quote = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quote = !in_quote,
            b'>' if !in_quote => return Some(i),
            _ => {}
        }
    }
    None
}

/// Extract the value of a named attribute from a tag string.
/// Looks for `name="value"` and returns `value`.
fn extract_attribute_value<'a>(tag: &'a str, attr_name: &str) -> Option<&'a str> {
    // Search for the attribute name followed by '='
    let mut search_from = 0;
    loop {
        let attr_pos = tag[search_from..].find(attr_name)?;
        let abs_pos = search_from + attr_pos;

        // Verify it's preceded by whitespace (not part of another attribute name)
        if abs_pos > 0 {
            let prev = tag.as_bytes()[abs_pos - 1];
            if prev != b' ' && prev != b'\t' && prev != b'\n' && prev != b'\r' {
                search_from = abs_pos + attr_name.len();
                continue;
            }
        }

        let after_name = &tag[abs_pos + attr_name.len()..];
        if !after_name.starts_with('=') {
            search_from = abs_pos + attr_name.len();
            continue;
        }

        let after_eq = &after_name[1..];
        if after_eq.starts_with('"') {
            let value_start = abs_pos + attr_name.len() + 2; // skip `="`
            let end_quote = tag[value_start..].find('"')?;
            return Some(&tag[value_start..value_start + end_quote]);
        }

        return None;
    }
}

/// Convert tags that contain `:attr="{{expr}}"` complex attribute bindings.
/// Replaces `{{expr}}` with `{expr}` in attribute values on `:` prefixed attributes.
/// Returns bytes consumed on success, or `None` if no conversion needed.
fn convert_complex_attrs(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>' outside of quoted attribute values
    let close = find_tag_close(tag_str)?;
    let tag_content = &tag_str[..=close];

    // Quick check: does this tag have both a colon-prefixed attribute and `{{`?
    if !tag_content.contains(":") || !tag_content.contains("{{") {
        return None;
    }

    // Process the tag character by character, converting {{}} to {} in :attr values
    let mut converted = String::with_capacity(tag_content.len());
    let tag_bytes = tag_content.as_bytes();
    let tag_len = tag_bytes.len();
    let mut j = 0;
    let mut in_colon_attr_value = false;

    while j < tag_len {
        if !in_colon_attr_value {
            // Look for :attrname="
            if tag_bytes[j] == b':' && j > 0 && is_whitespace(tag_bytes[j - 1]) {
                // Found a colon-prefixed attribute, scan to the ="
                let attr_start = j;
                j += 1;
                // Skip attribute name
                while j < tag_len && tag_bytes[j] != b'=' && !is_whitespace(tag_bytes[j]) {
                    j += 1;
                }
                if j < tag_len && tag_bytes[j] == b'=' {
                    j += 1;
                    if j < tag_len && tag_bytes[j] == b'"' {
                        // Push everything up to and including the opening quote
                        converted.push_str(&tag_content[attr_start..=j]);
                        j += 1;
                        in_colon_attr_value = true;
                        continue;
                    }
                }
                // Not a valid :attr="..." pattern, output as-is
                converted.push_str(&tag_content[attr_start..j]);
                continue;
            }
            j = push_char_at(tag_content, j, &mut converted);
        } else {
            // Inside a :attr value — convert {{expr}} to {expr}
            if tag_bytes[j] == b'"' {
                // End of attribute value
                converted.push('"');
                j += 1;
                in_colon_attr_value = false;
            } else if j + 1 < tag_len && tag_bytes[j] == b'{' && tag_bytes[j + 1] == b'{' {
                // Found {{ — find matching }}
                let expr_start = j + 2;
                if let Some(end_offset) = tag_content[expr_start..].find("}}") {
                    let expr = &tag_content[expr_start..expr_start + end_offset];
                    converted.push('{');
                    converted.push_str(expr);
                    converted.push('}');
                    j = expr_start + end_offset + 2;
                } else {
                    // No matching }}, output as-is
                    converted.push('{');
                    converted.push('{');
                    j += 2;
                }
            } else {
                j = push_char_at(tag_content, j, &mut converted);
            }
        }
    }

    result.push_str(&converted);
    Some(close + 1)
}

/// Push the single UTF-8 character starting at `pos` in `input` into `out`,
/// returning the byte index immediately after it.
fn push_char_at(input: &str, pos: usize, out: &mut String) -> usize {
    let bytes = input.as_bytes();
    if bytes[pos].is_ascii() {
        out.push(bytes[pos] as char);
        pos + 1
    } else {
        let mut end = pos + 1;
        while end < bytes.len() && !input.is_char_boundary(end) {
            end += 1;
        }
        out.push_str(&input[pos..end]);
        end
    }
}

/// Check if a byte is ASCII whitespace.
fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
}

/// Collapse whitespace-only text between `>` and `<` to eliminate extra DOM
/// text nodes that would shift element indices during hydration.
/// This ensures the f-template DOM structure matches the minified DSD output.
fn minify_inter_tag_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'>' && i + 1 < len {
            result.push('>');
            i += 1;
            // Skip whitespace-only content between > and <
            let ws_start = i;
            while i < len && is_whitespace(bytes[i]) {
                i += 1;
            }
            // If we hit '<', the whitespace was inter-tag — drop it
            // If we hit non-'<', it's meaningful text — keep it
            if i >= len || bytes[i] != b'<' {
                // Keep the whitespace (it's before text content)
                result.push_str(&input[ws_start..i]);
            }
        } else {
            i = push_char_at(input, i, &mut result);
        }
    }

    result
}

/// Strip every `shadowroot`-prefixed attribute from a `<template ...>`
/// opening tag. Returns `Some(bytes_consumed)` if a `<template` tag was
/// found and processed.
fn strip_shadowroot_attrs(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>' outside of quoted attribute values
    let close = find_tag_close(tag_str)?;
    let tag_content = &tag_str[..=close];

    // Only process if this tag contains any shadowroot* attribute.
    if !tag_content.contains("shadowroot") {
        return None;
    }

    // Rebuild the tag without any shadowroot* attribute
    result.push_str("<template");
    let attr_start = "<template".len();
    let inner = &tag_content[attr_start..close];

    // Scan through the attributes, skipping shadowroot* attrs.
    let inner_bytes = inner.as_bytes();
    let inner_len = inner_bytes.len();
    let mut j = 0;
    while j < inner_len {
        // Skip whitespace
        if is_whitespace(inner_bytes[j]) {
            j += 1;
            continue;
        }

        // Find the end of this attribute (name="value" or just name)
        let attr_begin = j;
        // Find '=' or whitespace or end
        while j < inner_len && inner_bytes[j] != b'=' && !is_whitespace(inner_bytes[j]) {
            j += 1;
        }
        let attr_name = &inner[attr_begin..j];

        // If there's a '=', consume the value
        let mut attr_end = j;
        if j < inner_len && inner_bytes[j] == b'=' {
            j += 1; // skip '='
            if j < inner_len && (inner_bytes[j] == b'"' || inner_bytes[j] == b'\'') {
                let quote = inner_bytes[j];
                j += 1; // skip opening quote
                while j < inner_len && inner_bytes[j] != quote {
                    j += 1;
                }
                if j < inner_len {
                    j += 1; // skip closing quote
                }
            } else {
                // Unquoted value
                while j < inner_len && !is_whitespace(inner_bytes[j]) {
                    j += 1;
                }
            }
            attr_end = j;
        }

        // Skip every shadowroot* attribute entirely
        if attr_name.starts_with("shadowroot") {
            continue;
        }

        // Keep this attribute
        result.push(' ');
        result.push_str(&inner[attr_begin..attr_end]);
    }

    if tag_content.ends_with("/>") {
        result.push_str("/>");
    } else {
        result.push('>');
    }

    Some(close + 1)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::path::PathBuf;

    fn make_component(tag: &str, html: &str, css: Option<&str>) -> Component {
        Component {
            tag_name: tag.to_string(),
            html_content: html.to_string(),
            css_content: css.map(ToString::to_string),
            css_tokens: Vec::new(),
            source_path: PathBuf::from("/test"),
            class_name: None,
        }
    }

    // --- classify_attribute ---

    #[test]
    fn skip_event_binding() {
        let mut plugin = FastV2ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("@click"),
            AttributeAction::SkipAndCountBinding
        );
        assert_eq!(
            plugin.classify_attribute("@input"),
            AttributeAction::SkipAndCountBinding
        );
        assert_eq!(
            plugin.classify_attribute("@custom-event"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn skip_f_ref() {
        let mut plugin = FastV2ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-ref"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn skip_f_slotted() {
        let mut plugin = FastV2ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-slotted"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn skip_f_children() {
        let mut plugin = FastV2ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-children"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn do_not_skip_normal_attributes() {
        let mut plugin = FastV2ParserPlugin::new();
        assert_eq!(plugin.classify_attribute("class"), AttributeAction::Keep);
        assert_eq!(plugin.classify_attribute("id"), AttributeAction::Keep);
        assert_eq!(
            plugin.classify_attribute("data-value"),
            AttributeAction::Keep
        );
        assert_eq!(plugin.classify_attribute(":title"), AttributeAction::Keep);
        assert_eq!(plugin.classify_attribute("f-other"), AttributeAction::Keep);
    }

    // --- finish_element ---

    #[test]
    fn element_parsed_zero_count_returns_none() {
        let mut plugin = FastV2ParserPlugin::new();
        assert!(plugin.finish_element(0).is_none());
    }

    #[test]
    fn element_parsed_nonzero_count_returns_le_bytes() {
        let mut plugin = FastV2ParserPlugin::new();
        let data = plugin.finish_element(3);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &3u32.to_le_bytes());
    }

    #[test]
    fn element_parsed_large_count() {
        let mut plugin = FastV2ParserPlugin::new();
        let data = plugin.finish_element(256);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &256u32.to_le_bytes());
    }

    #[test]
    fn component_template_simple_component() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template("my-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (name, html) = &templates[0];
        assert_eq!(name, "my-comp");
        assert!(html.contains("<f-template name=\"my-comp\">"));
        assert!(html.contains("</f-template>"));
        assert!(html.contains("<template>"));
        assert!(html.contains("</template>"));
        assert!(html.contains("<link rel=\"stylesheet\" href=\"my-comp.css\">"));
        assert!(html.contains("<div>hello</div>"));
    }

    #[test]
    fn component_template_without_css() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component("no-css", "<span>text</span>", None);
        plugin
            .register_component_template("no-css", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (name, html) = &templates[0];
        assert_eq!(name, "no-css");
        assert!(html.contains("<f-template name=\"no-css\">"));
        assert!(!html.contains("<link rel=\"stylesheet\""));
        assert!(html.contains("<span>text</span>"));
    }

    #[test]
    fn component_template_css_strategy_style() {
        let mut plugin = FastV2ParserPlugin::new();
        plugin.set_css_strategy(crate::CssStrategy::Style);
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template("my-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (_, html) = &templates[0];
        assert!(
            html.contains("<style>div { color: red; }</style>"),
            "Style strategy should inline CSS, got: {html}"
        );
        assert!(
            !html.contains("<link"),
            "Style strategy should not emit <link> tags"
        );
    }

    #[test]
    fn component_template_css_strategy_module() {
        let mut plugin = FastV2ParserPlugin::new();
        plugin.set_css_strategy(crate::CssStrategy::Module);
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template("my-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (_, html) = &templates[0];

        // f-template should NOT contain the CSS module — that is added
        // by the handler (SSR) or prepend_css_module (partials).
        assert!(
            !html.contains(r#"<style type="module""#),
            "CSS module should not be baked into f-template: {html}"
        );
        // shadowrootadoptedstylesheets on the inner template
        assert!(
            html.contains(r#"shadowrootadoptedstylesheets="my-comp""#),
            "Module strategy should add shadowrootadoptedstylesheets, got: {html}"
        );
        // No inline <style> or <link> inside the template
        assert!(
            !html.contains("<style>div"),
            "Module strategy should not have inline <style> inside template"
        );
        assert!(
            !html.contains("<link"),
            "Module strategy should not emit <link> tags"
        );
    }

    #[test]
    fn component_template_css_strategy_module_no_css() {
        let mut plugin = FastV2ParserPlugin::new();
        plugin.set_css_strategy(crate::CssStrategy::Module);
        let comp = make_component("my-comp", "<div>hello</div>", None);
        plugin
            .register_component_template("my-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (_, html) = &templates[0];
        assert!(
            !html.contains("shadowrootadoptedstylesheets"),
            "No CSS = no shadowrootadoptedstylesheets, got: {html}"
        );
        assert!(
            !html.contains("<style"),
            "No CSS = no style tag, got: {html}"
        );
    }

    #[test]
    fn component_template_multiple_components() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp1 = make_component("comp-a", "<div>A</div>", None);
        let comp2 = make_component("comp-b", "<div>B</div>", Some("b { }"));
        plugin
            .register_component_template("comp-a", &comp1, &comp1.html_content)
            .unwrap();
        plugin
            .register_component_template("comp-b", &comp2, &comp2.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 2);

        let names: Vec<&str> = templates.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"comp-a"));
        assert!(names.contains(&"comp-b"));

        for (_, html) in &templates {
            assert!(html.contains("<f-template name="));
        }
    }

    #[test]
    fn component_template_deduplicates_same_component() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component(
            "my-button",
            "<button><slot></slot></button>",
            Some(".btn{}"),
        );

        // Simulate the same component encountered multiple times
        // (e.g., <my-button> used in todo-app, todo-item, etc.)
        plugin
            .register_component_template("my-button", &comp, &comp.html_content)
            .unwrap();
        plugin
            .register_component_template("my-button", &comp, &comp.html_content)
            .unwrap();
        plugin
            .register_component_template("my-button", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(
            templates.len(),
            1,
            "Expected exactly 1 template for my-button, got {}",
            templates.len()
        );
        assert_eq!(templates[0].0, "my-button");
    }

    #[test]
    fn component_template_deduplicates_mixed_components() {
        let mut plugin = FastV2ParserPlugin::new();
        let btn = make_component("my-button", "<button><slot></slot></button>", None);
        let card = make_component("my-card", "<div><slot></slot></div>", Some(".card{}"));

        // my-button used in two different parent templates, my-card used once
        plugin
            .register_component_template("my-button", &btn, &btn.html_content)
            .unwrap();
        plugin
            .register_component_template("my-card", &card, &card.html_content)
            .unwrap();
        plugin
            .register_component_template("my-button", &btn, &btn.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 2);

        let button_count = templates.iter().filter(|(n, _)| n == "my-button").count();
        let card_count = templates.iter().filter(|(n, _)| n == "my-card").count();
        assert_eq!(button_count, 1);
        assert_eq!(card_count, 1);
    }

    // --- FAST syntax conversion ---

    #[test]
    fn convert_if_to_f_when() {
        let input = r#"<if condition="isComplete"><span>Done</span></if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-when value="{{isComplete}}"><span>Done</span></f-when>"#
        );
    }

    #[test]
    fn convert_if_with_greater_than() {
        let input = r#"<if condition="vara > 2">Over two</if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<f-when value="{{vara > 2}}">Over two</f-when>"#);
    }

    #[test]
    fn convert_if_with_greater_than_or_equal() {
        let input = r#"<if condition="vara >= 2">Two and over</if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-when value="{{vara >= 2}}">Two and over</f-when>"#
        );
    }

    #[test]
    fn convert_if_with_less_than() {
        let input = r#"<if condition="x < 10">small</if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<f-when value="{{x < 10}}">small</f-when>"#);
    }

    #[test]
    fn convert_for_to_f_repeat() {
        let input = r#"<for each="tag in tags"><span>{{tag}}</span></for>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-repeat value="{{tag in tags}}"><span>{{tag}}</span></f-repeat>"#
        );
    }

    #[test]
    fn convert_nested_if_and_for() {
        let input = r#"<if condition="show"><for each="x in items"><p>{{x}}</p></for></if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-when value="{{show}}"><f-repeat value="{{x in items}}"><p>{{x}}</p></f-repeat></f-when>"#
        );
    }

    #[test]
    fn convert_complex_attr_double_braces() {
        let input = r#"<div :data="{{config}}"></div>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<div :data="{config}"></div>"#);
    }

    #[test]
    fn convert_multiple_complex_attrs() {
        let input = r#"<div :title="{{name}}" :data-id="{{id}}"></div>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<div :title="{name}" :data-id="{id}"></div>"#);
    }

    #[test]
    fn no_conversion_for_text_double_braces() {
        // Double braces in text content (not in :attr) should stay as-is
        let input = "<span>{{greeting}}</span>";
        let output = convert_btr_to_fast(input);
        assert_eq!(output, "<span>{{greeting}}</span>");
    }

    #[test]
    fn no_conversion_for_regular_attr() {
        // Double braces in non-colon attributes should stay as-is
        let input = r#"<div title="{{name}}"></div>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<div title="{{name}}"></div>"#);
    }

    #[test]
    fn component_template_with_btr_conversion() {
        let mut plugin = FastV2ParserPlugin::new();
        let html = r#"<div><if condition="visible"><span>hi</span></if></div>"#;
        let comp = make_component("my-widget", html, None);
        plugin
            .register_component_template("my-widget", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (name, result) = &templates[0];
        assert_eq!(name, "my-widget");

        assert!(result.contains("<f-when value=\"{{visible}}\">"));
        assert!(result.contains("</f-when>"));
        assert!(!result.contains("<if condition="));
        assert!(!result.contains("</if>"));
    }

    #[test]
    fn passthrough_normal_html() {
        let input = r#"<div class="container"><p>Hello</p></div>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, input);
    }

    #[test]
    fn outlet_kept_as_marker_in_f_template() {
        let input = r#"<div><outlet /></div>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<div><outlet></outlet></div>"#);
    }

    #[test]
    fn outlet_marker_in_component_template() {
        let mut plugin = FastV2ParserPlugin::new();
        let html =
            r#"<template shadowrootmode="open"><h1>Title</h1><main><outlet /></main></template>"#;
        let comp = make_component("my-shell", html, None);
        plugin
            .register_component_template("my-shell", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let (_, result) = &templates[0];
        assert!(
            result.contains("<outlet></outlet>"),
            "outlet should be kept as marker in f-template: {result}"
        );
    }

    #[test]
    fn strip_shadowrootmode_from_template() {
        let input = r#"<template shadowrootmode="open"><div>Hello</div></template>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, "<template><div>Hello</div></template>");
    }

    #[test]
    fn strip_shadowrootmode_preserves_other_attrs() {
        let input =
            r#"<template shadowrootmode="open" @click="{onClick(e)}"><div>Hi</div></template>"#;
        let output = convert_btr_to_fast(input);
        assert!(output.contains(r#"@click="{onClick(e)}""#));
        assert!(!output.contains("shadowrootmode"));
        assert!(output.starts_with("<template "));
    }

    #[test]
    fn strip_shadowroot_attrs_strips_all_shadowroot_prefixed() {
        // The renamed strip helper should strip *every* shadowroot* attribute,
        // not just shadowrootmode. Non-shadowroot attrs are preserved.
        let input = r#"<template shadowrootmode="open" shadowrootclonable shadowrootdelegatesfocus="true" shadowrootadoptedstylesheets="x" data-keep="y"><div>Hi</div></template>"#;
        let output = convert_btr_to_fast(input);
        assert!(!output.contains("shadowrootmode"), "got: {output}");
        assert!(!output.contains("shadowrootclonable"), "got: {output}");
        assert!(
            !output.contains("shadowrootdelegatesfocus"),
            "got: {output}"
        );
        assert!(
            !output.contains("shadowrootadoptedstylesheets"),
            "got: {output}"
        );
        assert!(output.contains(r#"data-keep="y""#), "got: {output}");
    }

    #[test]
    fn no_strip_when_no_shadowrootmode() {
        let input = r#"<template foo="bar"><div>Hi</div></template>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, input);
    }

    #[test]
    fn component_template_moves_shadowrootmode_to_f_template() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component(
            "my-comp",
            r#"<template shadowrootmode="open" @click="{onClick(e)}"><div>{{title}}</div></template>"#,
            Some("div { color: red; }"),
        );
        plugin
            .register_component_template("my-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (name, html) = &templates[0];
        assert_eq!(name, "my-comp");
        // shadowrootmode is moved to the OUTER <f-template>...
        assert!(
            html.contains("<f-template name=\"my-comp\" shadowrootmode=\"open\">"),
            "<f-template> should carry shadowrootmode, got: {html}"
        );
        // ...and is NOT on the inner <template>.
        assert!(
            !html.contains("<template shadowrootmode"),
            "inner <template> should not carry shadowrootmode, got: {html}"
        );
        // And @click is still preserved on the inner template (framework attr).
        assert!(html.contains("@click"));
        // Should have the converted content
        assert!(html.contains("{{title}}"));
    }

    #[test]
    fn component_template_carries_user_shadowrootmode_closed() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component(
            "closed-comp",
            r#"<template shadowrootmode="closed"><div>x</div></template>"#,
            None,
        );
        plugin
            .register_component_template("closed-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        assert!(
            html.contains(r#"<f-template name="closed-comp" shadowrootmode="closed">"#),
            "<f-template> should carry user-supplied shadowrootmode value, got: {html}"
        );
        assert!(
            !html.contains("<template shadowrootmode"),
            "inner <template> should not carry shadowrootmode, got: {html}"
        );
    }

    #[test]
    fn component_template_preserves_other_shadowroot_attrs_on_f_template() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component(
            "clone-comp",
            r#"<template shadowrootmode="open" shadowrootclonable shadowrootdelegatesfocus="true"><div>x</div></template>"#,
            None,
        );
        plugin
            .register_component_template("clone-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        assert!(
            html.contains(r#"shadowrootmode="open""#),
            "<f-template> should carry shadowrootmode, got: {html}"
        );
        assert!(
            html.contains("shadowrootclonable"),
            "<f-template> should carry shadowrootclonable, got: {html}"
        );
        assert!(
            html.contains(r#"shadowrootdelegatesfocus="true""#),
            "<f-template> should carry shadowrootdelegatesfocus, got: {html}"
        );
        // Inner <template> must not carry any of them.
        let inner_template_start = html.find("\n<template").expect("inner template");
        let inner_template_end = html[inner_template_start..]
            .find('>')
            .expect("inner template close");
        let inner_open = &html[inner_template_start..inner_template_start + inner_template_end + 1];
        assert!(
            !inner_open.contains("shadowroot"),
            "inner <template> should not carry any shadowroot* attr, got: {inner_open}"
        );
    }

    #[test]
    fn component_template_default_no_shadowroot_attr_on_f_template_when_not_supplied() {
        // When the user did NOT supply any shadowroot* attribute, the
        // <f-template> should not gain one — <f-template> is not itself
        // declarative shadow DOM and the framework hard-codes
        // attachShadow({ mode: 'open' }) client-side.
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component("plain-comp", r#"<div>x</div>"#, None);
        plugin
            .register_component_template("plain-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        assert!(
            html.contains(r#"<f-template name="plain-comp">"#),
            "<f-template> should not have shadowroot* attrs when user did not supply any, got: {html}"
        );
        assert!(
            !html.contains("shadowroot"),
            "no shadowroot* attr should be emitted, got: {html}"
        );
    }

    #[test]
    fn component_template_module_css_puts_adopted_stylesheets_on_f_template() {
        let mut plugin = FastV2ParserPlugin::new();
        plugin.set_css_strategy(CssStrategy::Module);
        let comp = make_component(
            "mod-comp",
            r#"<template shadowrootmode="open"><div>x</div></template>"#,
            Some("div { color: blue; }"),
        );
        plugin
            .register_component_template("mod-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        // shadowrootadoptedstylesheets is on <f-template>, not on inner <template>.
        assert!(
            html.contains(r#"<f-template name="mod-comp" shadowrootmode="open" shadowrootadoptedstylesheets="mod-comp">"#),
            "<f-template> should carry shadowrootadoptedstylesheets for module CSS strategy, got: {html}"
        );
        let inner_template_start = html.find("\n<template").expect("inner template");
        let inner_close = html[inner_template_start..]
            .find('>')
            .expect("inner template close");
        let inner_open = &html[inner_template_start..inner_template_start + inner_close + 1];
        assert!(
            !inner_open.contains("shadowrootadoptedstylesheets"),
            "inner <template> should not carry shadowrootadoptedstylesheets, got: {inner_open}"
        );
    }

    #[test]
    fn component_template_merges_user_and_module_adopted_stylesheets() {
        let mut plugin = FastV2ParserPlugin::new();
        plugin.set_css_strategy(CssStrategy::Module);
        let comp = make_component(
            "merge-comp",
            r#"<template shadowrootmode="open" shadowrootadoptedstylesheets="theme other"><div>x</div></template>"#,
            Some("div { color: blue; }"),
        );
        plugin
            .register_component_template("merge-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        // User specifiers come first (in order), parser-generated `merge-comp` is
        // appended last, no duplicates.
        assert!(
            html.contains(r#"shadowrootadoptedstylesheets="theme other merge-comp""#),
            "<f-template> should merge user and parser specifiers, got: {html}"
        );
    }

    #[test]
    fn component_template_handles_gt_in_quoted_attr_value() {
        let mut plugin = FastV2ParserPlugin::new();
        let comp = make_component(
            "gt-comp",
            r#"<template shadowrootmode="open" data-note="a > b"><div>x</div></template>"#,
            None,
        );
        plugin
            .register_component_template("gt-comp", &comp, &comp.html_content)
            .unwrap();
        let templates = plugin.take_component_templates();
        let (_, html) = &templates[0];
        // Outer <f-template> gets shadowrootmode; inner <template> retains data-note.
        assert!(
            html.contains(r#"<f-template name="gt-comp" shadowrootmode="open">"#),
            "<f-template> should carry shadowrootmode even when an attr value contains '>', got: {html}"
        );
        assert!(
            html.contains(r#"data-note="a > b""#),
            "data-note should be preserved on the inner <template>, got: {html}"
        );
    }

    #[test]
    fn test_complex_attribute_normalization() {
        // All spacing variants should convert {{}} to {} in :attr values,
        // preserving any whitespace within the expression.
        let cases = [
            (
                r#"<div :foo="{{value}}"></div>"#,
                r#"<div :foo="{value}"></div>"#,
            ),
            (
                r#"<div :foo="{{ value}}"></div>"#,
                r#"<div :foo="{ value}"></div>"#,
            ),
            (
                r#"<div :foo="{{value }}"></div>"#,
                r#"<div :foo="{value }"></div>"#,
            ),
            (
                r#"<div :foo="{{ value }}"></div>"#,
                r#"<div :foo="{ value }"></div>"#,
            ),
        ];
        for (input, expected) in &cases {
            let output = convert_btr_to_fast(input);
            assert_eq!(&output, expected, "normalization failed for input: {input}");
        }
    }

    #[test]
    fn test_non_js_component_inner_template() {
        let mut plugin = FastV2ParserPlugin::new();
        // A JS component whose HTML contains a non-JS component using
        // declarative shadow DOM (template with shadowrootmode).
        let html = r#"<div><child-comp><template shadowrootmode="open"><p>inner</p></template></child-comp></div>"#;
        let comp = make_component("parent-comp", html, None);
        plugin
            .register_component_template("parent-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let (name, output) = &templates[0];
        assert_eq!(name, "parent-comp");

        assert!(output.contains("<f-template name=\"parent-comp\">"));
        // shadowrootmode must be stripped from the inner non-JS component's template
        assert!(!output.contains("shadowrootmode"));
        // The inner template structure should be preserved (without shadowrootmode)
        assert!(output.contains("<child-comp><template><p>inner</p></template></child-comp>"));
    }

    #[test]
    fn route_tags_converted_to_f_route() {
        let input = r#"<route path="/home" name="home" exact><span>Home</span></route>"#;
        let output = convert_btr_to_fast(input);
        assert!(output.starts_with("<webui-route"));
        assert!(output.contains(r#"path="/home""#));
        assert!(output.contains(r#"name="home""#));
        assert!(output.contains(" exact"));
        assert!(output.contains("style=\"display:none\""));
        assert!(output.contains("<span>Home</span>"));
        assert!(output.ends_with("</webui-route>"));
    }

    #[test]
    fn f_route_tags_pass_through() {
        // <webui-route> tags emitted by the parser should pass through unchanged
        let input = r#"<webui-route path="/" name="dashboard" component="cb-page-dashboard" exact style="display:none"></webui-route>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, input);
    }

    // --- End-to-end f-template regression tests ---
    // Verify that generated f-template blocks always use FAST syntax
    // (<f-when>/<f-repeat>) instead of webui syntax (<if>/<for>).

    #[test]
    fn ftemplate_for_loop_converted_to_f_repeat() {
        let mut plugin = FastV2ParserPlugin::new();
        let html = r#"<ul><for each="item in items"><li>{{item.name}}</li></for></ul>"#;
        let comp = make_component("my-list", html, None);
        plugin
            .register_component_template("my-list", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let (_, result) = &templates[0];

        assert!(
            result.contains("<f-repeat value=\"{{item in items}}\">"),
            "f-template should use <f-repeat>, got: {result}"
        );
        assert!(
            result.contains("</f-repeat>"),
            "f-template should close with </f-repeat>, got: {result}"
        );
        assert!(
            !result.contains("<for "),
            "f-template should NOT contain <for>, got: {result}"
        );
    }

    #[test]
    fn ftemplate_shadow_dom_moves_shadowroot_and_converts_directives() {
        let mut plugin = FastV2ParserPlugin::new();
        let html = r#"<template shadowrootmode="open"><div><if condition="visible">Shown</if><for each="x in list"><span>{{x}}</span></for></div></template>"#;
        let comp = make_component("my-shadow", html, None);
        plugin
            .register_component_template("my-shadow", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let (_, result) = &templates[0];

        assert!(
            result.contains("<f-when value=\"{{visible}}\">"),
            "Shadow f-template should contain <f-when>, got: {result}"
        );
        assert!(
            result.contains("<f-repeat value=\"{{x in list}}\">"),
            "Shadow f-template should contain <f-repeat>, got: {result}"
        );
        // shadowrootmode is moved to the outer <f-template>...
        assert!(
            result.contains(r#"<f-template name="my-shadow" shadowrootmode="open">"#),
            "<f-template> should carry shadowrootmode, got: {result}"
        );
        // ...not on the inner <template>.
        assert!(
            !result.contains("<template shadowrootmode"),
            "inner <template> should NOT carry shadowrootmode, got: {result}"
        );
        assert!(
            !result.contains("<if "),
            "Shadow f-template should NOT contain <if>, got: {result}"
        );
        assert!(
            !result.contains("<for "),
            "Shadow f-template should NOT contain <for>, got: {result}"
        );
    }

    #[test]
    fn ftemplate_nested_if_and_for_both_converted() {
        let mut plugin = FastV2ParserPlugin::new();
        let html =
            r#"<div><if condition="show"><for each="x in items"><p>{{x}}</p></for></if></div>"#;
        let comp = make_component("my-nested", html, None);
        plugin
            .register_component_template("my-nested", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let (_, result) = &templates[0];

        assert!(
            result.contains("<f-when value=\"{{show}}\">"),
            "Nested f-template should contain <f-when>, got: {result}"
        );
        assert!(
            result.contains("<f-repeat value=\"{{x in items}}\">"),
            "Nested f-template should contain <f-repeat>, got: {result}"
        );
        assert!(
            !result.contains("<if "),
            "Nested f-template should NOT contain <if>, got: {result}"
        );
        assert!(
            !result.contains("<for "),
            "Nested f-template should NOT contain <for>, got: {result}"
        );
    }

    #[test]
    fn convert_btr_preserves_utf8_text_content() {
        let input = "<span>✓ passed</span>";
        let output = convert_btr_to_fast(input);
        assert_eq!(output, "<span>✓ passed</span>");
    }

    #[test]
    fn convert_btr_preserves_utf8_multibyte_chars() {
        let input = "<span>✓ yes ✗ no ⭐ star</span>";
        let output = convert_btr_to_fast(input);
        assert_eq!(output, "<span>✓ yes ✗ no ⭐ star</span>");
    }

    #[test]
    fn convert_btr_preserves_utf8_with_directives() {
        let input = r#"<if condition="x"><span>✓</span></if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, r#"<f-when value="{{x}}"><span>✓</span></f-when>"#);
    }

    #[test]
    fn minify_preserves_utf8_text_content() {
        let input = "<span>✓</span><span>✗</span>";
        let output = minify_inter_tag_whitespace(input);
        assert_eq!(output, "<span>✓</span><span>✗</span>");
    }

    #[test]
    fn minify_preserves_utf8_between_elements() {
        let input = "<div> ✓ </div>";
        let output = minify_inter_tag_whitespace(input);
        assert_eq!(output, "<div> ✓ </div>");
    }

    #[test]
    fn convert_complex_attrs_preserves_utf8() {
        let input = r#"<span :title="{{label}}" data-icon="⭐">text</span>"#;
        let output = convert_btr_to_fast(input);
        assert!(
            output.contains("⭐"),
            "UTF-8 star should be preserved: {output}"
        );
    }

    #[test]
    fn find_tag_close_skips_quoted_gt() {
        assert_eq!(
            find_tag_close(r#"<if condition="a > b">"#),
            Some(21) // the `>` after the closing quote
        );
        assert_eq!(find_tag_close(r#"<if condition="a >= b">"#), Some(22));
        assert_eq!(find_tag_close("<br>"), Some(3));
        assert_eq!(find_tag_close("<br/>"), Some(4));
    }
}
