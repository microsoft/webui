// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::diagnostic::{codes, Diagnostic};
use crate::html_parser::{self as html, Event, Walker};
use crate::{ParserError, Result};
use std::ops::Range;

pub(crate) const INTERNAL_FAST_BINDING_ATTR: &str = "data-webui-internal-fast-binding";

pub(crate) struct PreparedComponentTemplate {
    pub(crate) tag_name: String,
    pub(crate) html_content: String,
    pub(crate) plugin_parse_content: Option<String>,
    pub(crate) plugin_template_content: Option<String>,
}

struct FTemplateSource {
    name: Option<String>,
    inner: Range<usize>,
}

pub(crate) fn prepare_component_template(
    tag_name: &str,
    html_content: &str,
) -> Result<PreparedComponentTemplate> {
    let Some(source) = find_f_template_source(html_content)? else {
        return Ok(PreparedComponentTemplate {
            tag_name: tag_name.to_string(),
            html_content: html_content.to_string(),
            plugin_parse_content: None,
            plugin_template_content: None,
        });
    };

    let resolved_tag = source.name.as_deref().unwrap_or(tag_name).to_string();
    let template_content = html_content[source.inner].trim();
    let html_content = convert_fast_template_to_webui(template_content, false);
    let plugin_parse_content = convert_fast_template_to_webui(template_content, true);
    let plugin_parse_content =
        (plugin_parse_content != html_content).then_some(plugin_parse_content);
    Ok(PreparedComponentTemplate {
        tag_name: resolved_tag,
        html_content,
        plugin_parse_content,
        plugin_template_content: Some(template_content.to_string()),
    })
}

fn find_f_template_source(html_content: &str) -> Result<Option<FTemplateSource>> {
    let mut found: Option<FTemplateSource> = None;
    let mut stack = Vec::with_capacity(1);
    stack.push(0..html_content.len());

    while let Some(range) = stack.pop() {
        for event in Walker::new_range(html_content, range.start, range.end) {
            let Event::Element(element) = event else {
                continue;
            };

            if element.name().eq_ignore_ascii_case("f-template") {
                if found.is_some() {
                    return Err(multiple_f_templates_error(html_content, element.start));
                }
                if !element.self_closing() && element.close_end() == element.content_end() {
                    return Err(unclosed_f_template_error(html_content, element.start));
                }
                found = Some(FTemplateSource {
                    name: element
                        .attr("name")
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                    inner: element.inner(),
                });
            }

            if !element.self_closing() && !element.is_void() {
                stack.push(element.inner());
            }
        }
    }

    Ok(found)
}

#[cold]
#[inline(never)]
fn multiple_f_templates_error(source: &str, offset: usize) -> ParserError {
    Diagnostic::error("multiple <f-template> elements are not supported")
        .code(codes::UNSUPPORTED_MULTIPLE_F_TEMPLATES)
        .at_offset(source, offset)
        .snippet("<f-template>")
        .help(
            "keep only one <f-template> per component file; multiple f-template blocks are not currently supported",
        )
        .into()
}

#[cold]
#[inline(never)]
fn unclosed_f_template_error(source: &str, offset: usize) -> ParserError {
    Diagnostic::error("unclosed <f-template> tag")
        .code(codes::UNCLOSED_HTML_TAG)
        .at_offset(source, offset)
        .snippet("<f-template>")
        .help("add the matching </f-template> closing tag")
        .into()
}

fn convert_fast_template_to_webui(input: &str, include_binding_markers: bool) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    while index < len {
        if bytes[index] == b'<' {
            if let Some(consumed) =
                try_convert_fast_tag(input, index, &mut result, include_binding_markers)
            {
                index += consumed;
                continue;
            }
        }
        index = push_char_at(input, index, &mut result);
    }

    result
}

fn try_convert_fast_tag(
    input: &str,
    pos: usize,
    result: &mut String,
    include_binding_markers: bool,
) -> Option<usize> {
    let remaining = &input[pos..];
    let tag = html::parse_tag(remaining)?;

    if tag.closing {
        if tag.name.eq_ignore_ascii_case("f-repeat") {
            result.push_str("</for>");
            return Some(tag.close + 1);
        }
        if tag.name.eq_ignore_ascii_case("f-when") {
            result.push_str("</if>");
            return Some(tag.close + 1);
        }
        return None;
    }

    if tag.name.eq_ignore_ascii_case("f-repeat") {
        push_webui_directive_tag("for", "each", tag.attr("value"), tag.self_closing, result);
        return Some(tag.close + 1);
    }
    if tag.name.eq_ignore_ascii_case("f-when") {
        push_webui_directive_tag(
            "if",
            "condition",
            tag.attr("value"),
            tag.self_closing,
            result,
        );
        return Some(tag.close + 1);
    }

    if !tag.attrs().any(|attr| {
        is_fast_client_only_attr(attr.name) || attr.value.and_then(single_braced_value).is_some()
    }) {
        return None;
    }

    push_webui_opening_tag(&tag, result, include_binding_markers);
    Some(tag.close + 1)
}

fn push_webui_directive_tag(
    tag_name: &str,
    attr_name: &str,
    value: Option<&str>,
    self_closing: bool,
    result: &mut String,
) {
    result.push('<');
    result.push_str(tag_name);
    if let Some(expr) = value.map(strip_fast_value_expression) {
        if !expr.is_empty() {
            result.push(' ');
            result.push_str(attr_name);
            result.push_str("=\"");
            result.push_str(expr);
            result.push('"');
        }
    }
    if self_closing {
        result.push_str("></");
        result.push_str(tag_name);
        result.push('>');
    } else {
        result.push('>');
    }
}

fn push_webui_opening_tag(tag: &html::Tag<'_>, result: &mut String, include_binding_markers: bool) {
    result.push('<');
    result.push_str(tag.name);
    let mut stripped_binding_count = 0usize;
    for attr in tag.attrs() {
        if is_fast_client_only_attr(attr.name) {
            stripped_binding_count += 1;
            continue;
        }
        result.push(' ');
        if let Some(expr) = attr.value.and_then(single_braced_value) {
            result.push_str(attr.name);
            result.push_str("=\"{{");
            result.push_str(expr);
            result.push_str("}}\"");
        } else {
            result.push_str(attr.raw);
        }
    }
    if include_binding_markers {
        for _ in 0..stripped_binding_count {
            result.push(' ');
            result.push_str(INTERNAL_FAST_BINDING_ATTR);
        }
    }
    if tag.self_closing {
        result.push_str(" />");
    } else {
        result.push('>');
    }
}

fn is_fast_client_only_attr(name: &str) -> bool {
    name.starts_with('@')
        || name.starts_with(':')
        || matches!(name, "f-ref" | "f-slotted" | "f-children")
}

fn strip_fast_value_expression(value: &str) -> &str {
    double_braced_value(value)
        .or_else(|| single_braced_value(value))
        .unwrap_or_else(|| value.trim())
}

fn double_braced_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") && !trimmed.starts_with("{{{") {
        let inner = trimmed[2..trimmed.len() - 2].trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

fn single_braced_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
        let inner = trimmed[1..trimmed.len() - 1].trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

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
