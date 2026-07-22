// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST 3 parser plugin for the WebUI parser.
//!
//! Tracks component definitions during HTML parsing and returns `<f-template>`
//! artifacts after parsing. Converts WebUI Framework template syntax (`<if>`, `<for>`, `{{}}`)
//! into FAST-compatible syntax (`<f-when>`, `<f-repeat>`, `{}`).

use super::{AttributeAction, ComponentTemplateArtifact, ParserPlugin, ParserPluginArtifacts};
use crate::component_registry::Component;
use crate::html_parser::{find_tag_close, opening_tag_name};
use crate::{CssLinkOptions, CssStrategy, Result};
use webui_protocol::FastElementData;

/// Information about a tracked component for `<f-template>` generation.
struct TrackedComponent {
    tag_name: String,
    template_html: String,
}

/// FAST 3 parser plugin used by `fast-v3`.
///
/// Implements the `ParserPlugin` trait for FAST 3 applications:
/// - Filters FAST-specific runtime binding attributes (`@click`, `f-ref`, etc.)
/// - Tracks components encountered during parsing
/// - Returns `<f-template>` artifacts with converted FAST syntax after parsing
/// - Emits binding attribute counts as `Plugin` protocol fragment data
pub struct FastV3ParserPlugin {
    /// Components tracked during parsing, in discovery order.
    components: Vec<TrackedComponent>,
}

impl FastV3ParserPlugin {
    /// Create a new FAST 3 parser plugin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Take the individual component f-template strings, keyed by tag name.
    ///
    /// Each value is a complete `<f-template name="tag-name">...</f-template>` string
    /// ready to be appended to a document. This is used by the JSON partial render
    /// endpoint to send only the templates the client needs.
    #[must_use]
    pub fn take_component_templates(&self) -> Vec<ComponentTemplateArtifact> {
        self.components
            .iter()
            .map(|comp| {
                let tmpl = generate_f_template_from_processed(&comp.tag_name, &comp.template_html);
                ComponentTemplateArtifact::template(comp.tag_name.clone(), tmpl)
            })
            .collect()
    }
}

impl Default for FastV3ParserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserPlugin for FastV3ParserPlugin {
    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        processed_template: &str,
    ) -> Result<()> {
        // Only track each component once (avoids duplicate <f-template> blocks
        // when a component is used in multiple parent templates)
        if self.components.iter().any(|c| c.tag_name == tag_name) {
            return Ok(());
        }
        self.components.push(TrackedComponent {
            tag_name: tag_name.to_string(),
            template_html: processed_template.to_string(),
        });
        let _ = component;
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

    fn into_artifacts(self: Box<Self>) -> Result<ParserPluginArtifacts> {
        Ok(ParserPluginArtifacts::ComponentTemplates(
            self.take_component_templates(),
        ))
    }
}

/// Generate a single f-template HTML string from component data.
///
/// Used by the server to generate templates on demand for route components
/// that weren't encountered during initial parsing.
///
/// This uses default Link-mode CSS filenames. Call
/// [`generate_f_template_with_css_options`] when the build uses custom CSS
/// filename templates or a public CSS base.
pub fn generate_f_template(
    tag_name: &str,
    html_content: &str,
    css_content: Option<&str>,
    css_strategy: CssStrategy,
) -> String {
    generate_f_template_with_css_options(
        tag_name,
        html_content,
        css_content,
        css_strategy,
        &CssLinkOptions::default(),
    )
}

fn generate_f_template_from_processed(tag_name: &str, processed_template: &str) -> String {
    let mut output = String::with_capacity(256);
    output.push_str("<f-template name=\"");
    output.push_str(tag_name);
    output.push_str("\">\n");

    let converted = convert_btr_to_fast(processed_template);
    let trimmed = minify_inter_tag_whitespace(converted.trim());

    if trimmed.starts_with("<template") {
        output.push_str(&trimmed);
    } else {
        output.push_str("<template>");
        output.push_str(&trimmed);
        output.push_str("</template>");
    }

    output.push_str("\n</f-template>\n");
    output
}

/// Generate a FAST 3 f-template with Link CSS filename/href options.
pub fn generate_f_template_with_css_options(
    tag_name: &str,
    html_content: &str,
    css_content: Option<&str>,
    css_strategy: CssStrategy,
    css_link_options: &CssLinkOptions,
) -> String {
    let mut output = String::with_capacity(256);
    output.push_str("<f-template name=\"");
    output.push_str(tag_name);
    output.push_str("\">\n");

    let converted = convert_btr_to_fast(html_content);
    let trimmed = minify_inter_tag_whitespace(converted.trim());

    // Build the CSS injection string based on the configured strategy
    let css_injection = match css_strategy {
        CssStrategy::Link => css_content.map(|css| {
            let href = css_link_options.resolve(tag_name, css);
            let mut s = String::with_capacity(40 + href.href.len());
            s.push_str("<link rel=\"stylesheet\" href=\"");
            s.push_str(&href.href);
            s.push_str("\">");
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
            // Dev owns the wrapper — preserve attributes verbatim.
            // For `CssStrategy::Module` the parser pass enforces
            // `shadowrootadoptedstylesheets`, so by the time we get here
            // either the dev wrote it or the build already failed.
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
        output.push_str("<template");
        if css_strategy == CssStrategy::Module && css_content.is_some() {
            output.push_str(" shadowrootadoptedstylesheets=\"");
            output.push_str(tag_name);
            output.push('"');
        }
        output.push('>');
        if let Some(ref injection) = css_injection {
            output.push_str(injection);
        }
        output.push_str(&trimmed);
        output.push_str("</template>");
    }

    output.push_str("\n</f-template>\n");
    output
}

/// Convert WebUI Framework template syntax to FAST syntax in HTML content.
///
/// Performs the following transformations without regex:
/// - `<if condition="EXPR">` → `<f-when value="{{EXPR}}">`
/// - `</if>` → `</f-when>`
/// - `<for each="EXPR">` → `<f-repeat value="{{EXPR}}">`
/// - `</for>` → `</f-repeat>`
/// - `{{expr}}` inside `:attr` complex attribute values → `{expr}`
/// - Strips `shadowrootmode` attributes from `<template>` tags
///   (in f-template context, shadowrootmode must be removed to prevent
///   the browser from auto-activating it as a declarative shadow root)
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
        if let Some(close) = find_tag_close(remaining) {
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
        // Strip shadowrootmode from <template> tags
        if starts_with_tag_name(remaining, "template") {
            if let Some(consumed) = strip_shadowrootmode(remaining, result) {
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
    opening_tag_name(s).is_some_and(|tag_name| tag_name == name)
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

/// Strip `shadowrootmode` attribute from a `<template ...>` opening tag.
/// Returns `Some(bytes_consumed)` if a `<template` tag was found and processed.
fn strip_shadowrootmode(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>' outside of quoted attribute values
    let close = find_tag_close(tag_str)?;
    let tag_content = &tag_str[..=close];

    // Only process if this tag contains shadowrootmode
    if !tag_content.contains("shadowrootmode") {
        return None;
    }

    // Rebuild the tag without the shadowrootmode attribute
    result.push_str("<template");
    let attr_start = "<template".len();
    let inner = &tag_content[attr_start..close];

    // Scan through the attributes, skipping shadowrootmode
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
            if j < inner_len && inner_bytes[j] == b'"' {
                j += 1; // skip opening quote
                while j < inner_len && inner_bytes[j] != b'"' {
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

        // Skip the shadowrootmode attribute entirely
        if attr_name == "shadowrootmode" {
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
    fn make_component(tag: &str, html: &str, css: Option<&str>) -> Component {
        Component {
            tag_name: tag.to_string(),
            html_content: html.to_string(),
            css_content: css.map(ToString::to_string),
            css_definitions: Vec::new(),
            css_fallback_chains: Vec::new(),
            is_client_owned: false,
        }
    }

    // --- classify_attribute ---

    #[test]
    fn skip_event_binding() {
        let mut plugin = FastV3ParserPlugin::new();
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
        let mut plugin = FastV3ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-ref"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn skip_f_slotted() {
        let mut plugin = FastV3ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-slotted"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn skip_f_children() {
        let mut plugin = FastV3ParserPlugin::new();
        assert_eq!(
            plugin.classify_attribute("f-children"),
            AttributeAction::SkipAndCountBinding
        );
    }

    #[test]
    fn do_not_skip_normal_attributes() {
        let mut plugin = FastV3ParserPlugin::new();
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
        let mut plugin = FastV3ParserPlugin::new();
        assert!(plugin.finish_element(0).is_none());
    }

    #[test]
    fn element_parsed_nonzero_count_returns_le_bytes() {
        let mut plugin = FastV3ParserPlugin::new();
        let data = plugin.finish_element(3);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &3u32.to_le_bytes());
    }

    #[test]
    fn element_parsed_large_count() {
        let mut plugin = FastV3ParserPlugin::new();
        let data = plugin.finish_element(256);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &256u32.to_le_bytes());
    }

    #[test]
    fn component_template_simple_component() {
        let mut plugin = FastV3ParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template(
                "my-comp",
                &comp,
                r#"<template><link rel="stylesheet" href="my-comp.css"><div>hello</div></template>"#,
            )
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let name = &templates[0].tag_name;
        let html = &templates[0].template;
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
        let mut plugin = FastV3ParserPlugin::new();
        let comp = make_component("no-css", "<span>text</span>", None);
        plugin
            .register_component_template("no-css", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let name = &templates[0].tag_name;
        let html = &templates[0].template;
        assert_eq!(name, "no-css");
        assert!(html.contains("<f-template name=\"no-css\">"));
        assert!(!html.contains("<link rel=\"stylesheet\""));
        assert!(html.contains("<span>text</span>"));
    }

    #[test]
    fn component_template_css_strategy_style() {
        let mut plugin = FastV3ParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template(
                "my-comp",
                &comp,
                r#"<template><style>div { color: red; }</style><div>hello</div></template>"#,
            )
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let html = &templates[0].template;
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
        let mut plugin = FastV3ParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin
            .register_component_template(
                "my-comp",
                &comp,
                r#"<template shadowrootadoptedstylesheets="my-comp"><div>hello</div></template>"#,
            )
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let html = &templates[0].template;

        // f-template should NOT contain the CSS module — that is added
        // by the handler at SSR (inline) and SPA-partial time.
        assert!(
            !html.contains(r#"<style type="module""#),
            "Legacy CSS module shape should not be baked into f-template: {html}"
        );
        assert!(
            !html.contains(r#"<script type="importmap""#),
            "CSS module importmap should not be baked into f-template: {html}"
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
        let mut plugin = FastV3ParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", None);
        plugin
            .register_component_template(
                "my-comp",
                &comp,
                r#"<template><div>hello</div></template>"#,
            )
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let html = &templates[0].template;
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
        let mut plugin = FastV3ParserPlugin::new();
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

        let names: Vec<&str> = templates
            .iter()
            .map(|artifact| artifact.tag_name.as_str())
            .collect();
        assert!(names.contains(&"comp-a"));
        assert!(names.contains(&"comp-b"));

        for artifact in &templates {
            assert!(artifact.template.contains("<f-template name="));
        }
    }

    #[test]
    fn component_template_deduplicates_same_component() {
        let mut plugin = FastV3ParserPlugin::new();
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
        assert_eq!(templates[0].tag_name, "my-button");
    }

    #[test]
    fn component_template_deduplicates_mixed_components() {
        let mut plugin = FastV3ParserPlugin::new();
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

        let button_count = templates
            .iter()
            .filter(|artifact| artifact.tag_name == "my-button")
            .count();
        let card_count = templates
            .iter()
            .filter(|artifact| artifact.tag_name == "my-card")
            .count();
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
        let mut plugin = FastV3ParserPlugin::new();
        let html = r#"<div><if condition="visible"><span>hi</span></if></div>"#;
        let comp = make_component("my-widget", html, None);
        plugin
            .register_component_template("my-widget", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let name = &templates[0].tag_name;
        let result = &templates[0].template;
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
        let mut plugin = FastV3ParserPlugin::new();
        let html =
            r#"<template shadowrootmode="open"><h1>Title</h1><main><outlet /></main></template>"#;
        let comp = make_component("my-shell", html, None);
        plugin
            .register_component_template("my-shell", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let result = &templates[0].template;
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
    fn no_strip_when_no_shadowrootmode() {
        let input = r#"<template foo="bar"><div>Hi</div></template>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(output, input);
    }

    #[test]
    fn component_template_strips_shadowrootmode_from_f_template() {
        let mut plugin = FastV3ParserPlugin::new();
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
        let name = &templates[0].tag_name;
        let html = &templates[0].template;
        assert_eq!(name, "my-comp");
        // f-template content should NOT have shadowrootmode
        assert!(!html.contains("shadowrootmode"));
        // But should keep @click (framework attr kept in f-template mode)
        assert!(html.contains("@click"));
        // Should have the converted content
        assert!(html.contains("{{title}}"));
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
        let mut plugin = FastV3ParserPlugin::new();
        // A JS component whose HTML contains a non-JS component using
        // declarative shadow DOM (template with shadowrootmode).
        let html = r#"<div><child-comp><template shadowrootmode="open"><p>inner</p></template></child-comp></div>"#;
        let comp = make_component("parent-comp", html, None);
        plugin
            .register_component_template("parent-comp", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        assert_eq!(templates.len(), 1);
        let name = &templates[0].tag_name;
        let output = &templates[0].template;
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
        let mut plugin = FastV3ParserPlugin::new();
        let html = r#"<ul><for each="item in items"><li>{{item.name}}</li></for></ul>"#;
        let comp = make_component("my-list", html, None);
        plugin
            .register_component_template("my-list", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let result = &templates[0].template;

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
    fn ftemplate_shadow_dom_strips_shadowroot_and_converts_directives() {
        let mut plugin = FastV3ParserPlugin::new();
        let html = r#"<template shadowrootmode="open"><div><if condition="visible">Shown</if><for each="x in list"><span>{{x}}</span></for></div></template>"#;
        let comp = make_component("my-shadow", html, None);
        plugin
            .register_component_template("my-shadow", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let result = &templates[0].template;

        assert!(
            result.contains("<f-when value=\"{{visible}}\">"),
            "Shadow f-template should contain <f-when>, got: {result}"
        );
        assert!(
            result.contains("<f-repeat value=\"{{x in list}}\">"),
            "Shadow f-template should contain <f-repeat>, got: {result}"
        );
        assert!(
            !result.contains("shadowrootmode"),
            "Shadow f-template should strip shadowrootmode, got: {result}"
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
        let mut plugin = FastV3ParserPlugin::new();
        let html =
            r#"<div><if condition="show"><for each="x in items"><p>{{x}}</p></for></if></div>"#;
        let comp = make_component("my-nested", html, None);
        plugin
            .register_component_template("my-nested", &comp, &comp.html_content)
            .unwrap();

        let templates = plugin.take_component_templates();
        let result = &templates[0].template;

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
}
