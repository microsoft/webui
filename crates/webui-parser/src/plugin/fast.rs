//! FAST parser plugin for the WebUI parser.
//!
//! Tracks component definitions during HTML parsing and generates `<f-template>`
//! wrappers at body end. Converts BTR template syntax (`<if>`, `<for>`, `{{}}`)
//! into FAST-compatible syntax (`<f-when>`, `<f-repeat>`, `{}`).

use super::ParserPlugin;
use crate::component_registry::Component;
use crate::Result;

/// Information about a tracked component for `<f-template>` generation.
struct TrackedComponent {
    tag_name: String,
    html_content: String,
    css_content: Option<String>,
}

/// FAST parser plugin.
///
/// Implements the `ParserPlugin` trait for the FAST framework:
/// - Filters FAST-specific runtime binding attributes (`@click`, `f-ref`, etc.)
/// - Tracks components encountered during parsing
/// - Generates `<f-template>` wrappers with converted BTR→FAST syntax at body end
/// - Emits binding attribute counts as `Plugin` protocol fragment data
pub struct FastParserPlugin {
    /// Components tracked during parsing, in discovery order.
    components: Vec<TrackedComponent>,
}

impl FastParserPlugin {
    /// Create a new FAST parser plugin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }
}

impl Default for FastParserPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserPlugin for FastParserPlugin {
    fn should_skip_attribute(&self, attr_name: &str) -> bool {
        attr_name.starts_with('@')
            || attr_name == "f-ref"
            || attr_name == "f-slotted"
            || attr_name == "f-children"
    }

    fn on_parse_component(&mut self, tag_name: &str, component: &Component) -> Result<()> {
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

    fn on_element_parsed(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
        if binding_attribute_count > 0 {
            Some(binding_attribute_count.to_le_bytes().to_vec())
        } else {
            None
        }
    }

    fn on_body_end(&mut self) -> Option<String> {
        if self.components.is_empty() {
            return None;
        }

        let mut output = String::with_capacity(self.components.len() * 256);

        for comp in &self.components {
            output.push_str("<f-template name=\"");
            output.push_str(&comp.tag_name);
            output.push_str("\">\n");

            let converted = convert_btr_to_fast(&comp.html_content);
            let trimmed = minify_inter_tag_whitespace(converted.trim());

            if trimmed.starts_with("<template") {
                if let Some(close_pos) = trimmed.find('>') {
                    output.push_str(&trimmed[..=close_pos]);
                    if comp.css_content.is_some() {
                        output.push_str("<link rel=\"stylesheet\" href=\"./");
                        output.push_str(&comp.tag_name);
                        output.push_str(".css\">");
                    }
                    output.push_str(&trimmed[close_pos + 1..]);
                } else {
                    output.push_str(&trimmed);
                }
            } else {
                output.push_str("<template>");
                if comp.css_content.is_some() {
                    output.push_str("<link rel=\"stylesheet\" href=\"./");
                    output.push_str(&comp.tag_name);
                    output.push_str(".css\">");
                }
                output.push_str(&trimmed);
                output.push_str("</template>");
            }

            output.push_str("\n</f-template>\n");
        }

        Some(output)
    }
}

/// Convert BTR template syntax to FAST syntax in HTML content.
///
/// Performs the following transformations without regex:
/// - `<if condition="EXPR">` → `<f-when value="{EXPR}">`
/// - `</if>` → `</f-when>`
/// - `<for each="EXPR">` → `<f-repeat value="{EXPR}">`
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
        result.push(bytes[i] as char);
        i += 1;
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

/// Convert `<if condition="EXPR">` to `<f-when value="{EXPR}">`.
/// Returns bytes consumed on success.
fn convert_if_tag(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>'
    let close = tag_str.find('>')?;
    let tag_content = &tag_str[..=close];

    // Find condition="..." attribute
    let attr_value = extract_attribute_value(tag_content, "condition")?;

    result.push_str("<f-when value=\"{");
    result.push_str(attr_value);
    result.push_str("}\">");

    Some(close + 1)
}

/// Convert `<for each="EXPR">` to `<f-repeat value="{EXPR}">`.
/// Returns bytes consumed on success.
fn convert_for_tag(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>'
    let close = tag_str.find('>')?;
    let tag_content = &tag_str[..=close];

    // Find each="..." attribute
    let attr_value = extract_attribute_value(tag_content, "each")?;

    result.push_str("<f-repeat value=\"{");
    result.push_str(attr_value);
    result.push_str("}\">");

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
    // Find the closing '>'
    let close = tag_str.find('>')?;
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
            converted.push(tag_bytes[j] as char);
            j += 1;
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
                converted.push(tag_bytes[j] as char);
                j += 1;
            }
        }
    }

    result.push_str(&converted);
    Some(close + 1)
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
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Strip `shadowrootmode` attribute from a `<template ...>` opening tag.
/// Returns `Some(bytes_consumed)` if a `<template` tag was found and processed.
fn strip_shadowrootmode(tag_str: &str, result: &mut String) -> Option<usize> {
    // Find the closing '>'
    let close = tag_str.find('>')?;
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
    use super::*;
    use std::path::PathBuf;

    fn make_component(tag: &str, html: &str, css: Option<&str>) -> Component {
        Component {
            tag_name: tag.to_string(),
            html_content: html.to_string(),
            css_content: css.map(ToString::to_string),
            source_path: PathBuf::from("/test"),
            class_name: None,
        }
    }

    // --- should_skip_attribute ---

    #[test]
    fn skip_event_binding() {
        let plugin = FastParserPlugin::new();
        assert!(plugin.should_skip_attribute("@click"));
        assert!(plugin.should_skip_attribute("@input"));
        assert!(plugin.should_skip_attribute("@custom-event"));
    }

    #[test]
    fn skip_f_ref() {
        let plugin = FastParserPlugin::new();
        assert!(plugin.should_skip_attribute("f-ref"));
    }

    #[test]
    fn skip_f_slotted() {
        let plugin = FastParserPlugin::new();
        assert!(plugin.should_skip_attribute("f-slotted"));
    }

    #[test]
    fn skip_f_children() {
        let plugin = FastParserPlugin::new();
        assert!(plugin.should_skip_attribute("f-children"));
    }

    #[test]
    fn do_not_skip_normal_attributes() {
        let plugin = FastParserPlugin::new();
        assert!(!plugin.should_skip_attribute("class"));
        assert!(!plugin.should_skip_attribute("id"));
        assert!(!plugin.should_skip_attribute("data-value"));
        assert!(!plugin.should_skip_attribute(":title"));
        assert!(!plugin.should_skip_attribute("f-other"));
    }

    // --- on_element_parsed ---

    #[test]
    fn element_parsed_zero_count_returns_none() {
        let mut plugin = FastParserPlugin::new();
        assert!(plugin.on_element_parsed(0).is_none());
    }

    #[test]
    fn element_parsed_nonzero_count_returns_le_bytes() {
        let mut plugin = FastParserPlugin::new();
        let data = plugin.on_element_parsed(3);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &3u32.to_le_bytes());
    }

    #[test]
    fn element_parsed_large_count() {
        let mut plugin = FastParserPlugin::new();
        let data = plugin.on_element_parsed(256);
        assert!(data.is_some());
        let bytes = data.as_deref().unwrap_or_default();
        assert_eq!(bytes, &256u32.to_le_bytes());
    }

    // --- on_body_end ---

    #[test]
    fn body_end_no_components_returns_none() {
        let mut plugin = FastParserPlugin::new();
        assert!(plugin.on_body_end().is_none());
    }

    #[test]
    fn body_end_simple_component() {
        let mut plugin = FastParserPlugin::new();
        let comp = make_component("my-comp", "<div>hello</div>", Some("div { color: red; }"));
        plugin.on_parse_component("my-comp", &comp).unwrap();

        let output = plugin.on_body_end();
        assert!(output.is_some());
        let html = output.unwrap_or_default();

        assert!(html.contains("<f-template name=\"my-comp\">"));
        assert!(html.contains("</f-template>"));
        assert!(html.contains("<template>"));
        assert!(html.contains("</template>"));
        assert!(html.contains("<link rel=\"stylesheet\" href=\"./my-comp.css\">"));
        assert!(html.contains("<div>hello</div>"));
    }

    #[test]
    fn body_end_component_without_css() {
        let mut plugin = FastParserPlugin::new();
        let comp = make_component("no-css", "<span>text</span>", None);
        plugin.on_parse_component("no-css", &comp).unwrap();

        let output = plugin.on_body_end();
        assert!(output.is_some());
        let html = output.unwrap_or_default();

        assert!(html.contains("<f-template name=\"no-css\">"));
        assert!(!html.contains("<link rel=\"stylesheet\""));
        assert!(html.contains("<span>text</span>"));
    }

    #[test]
    fn body_end_multiple_components() {
        let mut plugin = FastParserPlugin::new();
        let comp1 = make_component("comp-a", "<div>A</div>", None);
        let comp2 = make_component("comp-b", "<div>B</div>", Some("b { }"));
        plugin.on_parse_component("comp-a", &comp1).unwrap();
        plugin.on_parse_component("comp-b", &comp2).unwrap();

        let output = plugin.on_body_end();
        assert!(output.is_some());
        let html = output.unwrap_or_default();

        assert!(html.contains("<f-template name=\"comp-a\">"));
        assert!(html.contains("<f-template name=\"comp-b\">"));
    }

    #[test]
    fn body_end_deduplicates_same_component() {
        let mut plugin = FastParserPlugin::new();
        let comp = make_component(
            "my-button",
            "<button><slot></slot></button>",
            Some(".btn{}"),
        );

        // Simulate the same component encountered multiple times
        // (e.g., <my-button> used in todo-app, todo-item, etc.)
        plugin.on_parse_component("my-button", &comp).unwrap();
        plugin.on_parse_component("my-button", &comp).unwrap();
        plugin.on_parse_component("my-button", &comp).unwrap();

        let output = plugin.on_body_end().unwrap();
        let count = output.matches("<f-template name=\"my-button\">").count();
        assert_eq!(
            count, 1,
            "Expected exactly 1 <f-template> for my-button, got {count}"
        );
    }

    #[test]
    fn body_end_deduplicates_mixed_components() {
        let mut plugin = FastParserPlugin::new();
        let btn = make_component("my-button", "<button><slot></slot></button>", None);
        let card = make_component("my-card", "<div><slot></slot></div>", Some(".card{}"));

        // my-button used in two different parent templates, my-card used once
        plugin.on_parse_component("my-button", &btn).unwrap();
        plugin.on_parse_component("my-card", &card).unwrap();
        plugin.on_parse_component("my-button", &btn).unwrap();

        let output = plugin.on_body_end().unwrap();

        assert_eq!(output.matches("<f-template name=\"my-button\">").count(), 1);
        assert_eq!(output.matches("<f-template name=\"my-card\">").count(), 1);
    }

    // --- BTR→FAST syntax conversion ---

    #[test]
    fn convert_if_to_f_when() {
        let input = r#"<if condition="isComplete"><span>Done</span></if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-when value="{isComplete}"><span>Done</span></f-when>"#
        );
    }

    #[test]
    fn convert_for_to_f_repeat() {
        let input = r#"<for each="tag in tags"><span>{{tag}}</span></for>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-repeat value="{tag in tags}"><span>{{tag}}</span></f-repeat>"#
        );
    }

    #[test]
    fn convert_nested_if_and_for() {
        let input = r#"<if condition="show"><for each="x in items"><p>{{x}}</p></for></if>"#;
        let output = convert_btr_to_fast(input);
        assert_eq!(
            output,
            r#"<f-when value="{show}"><f-repeat value="{x in items}"><p>{{x}}</p></f-repeat></f-when>"#
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
    fn body_end_with_btr_conversion() {
        let mut plugin = FastParserPlugin::new();
        let html = r#"<div><if condition="visible"><span>hi</span></if></div>"#;
        let comp = make_component("my-widget", html, None);
        plugin.on_parse_component("my-widget", &comp).unwrap();

        let output = plugin.on_body_end();
        assert!(output.is_some());
        let result = output.unwrap_or_default();

        assert!(result.contains("<f-when value=\"{visible}\">"));
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
    fn body_end_strips_shadowrootmode_from_f_template() {
        let mut plugin = FastParserPlugin::new();
        let comp = make_component(
            "my-comp",
            r#"<template shadowrootmode="open" @click="{onClick(e)}"><div>{{title}}</div></template>"#,
            Some("div { color: red; }"),
        );
        plugin.on_parse_component("my-comp", &comp).unwrap();
        let output = plugin.on_body_end().unwrap();
        // f-template content should NOT have shadowrootmode
        assert!(!output.contains("shadowrootmode"));
        // But should keep @click (framework attr kept in f-template mode)
        assert!(output.contains("@click"));
        // Should have the converted content
        assert!(output.contains("{{title}}"));
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
        let mut plugin = FastParserPlugin::new();
        // A JS component whose HTML contains a non-JS component using
        // declarative shadow DOM (template with shadowrootmode).
        let html = r#"<div><child-comp><template shadowrootmode="open"><p>inner</p></template></child-comp></div>"#;
        let comp = make_component("parent-comp", html, None);
        plugin.on_parse_component("parent-comp", &comp).unwrap();

        let output = plugin.on_body_end().unwrap();

        assert!(output.contains("<f-template name=\"parent-comp\">"));
        // shadowrootmode must be stripped from the inner non-JS component's template
        assert!(!output.contains("shadowrootmode"));
        // The inner template structure should be preserved (without shadowrootmode)
        assert!(output.contains("<child-comp><template><p>inner</p></template></child-comp>"));
    }
}
