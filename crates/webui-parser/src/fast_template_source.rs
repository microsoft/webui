// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::diagnostic::{codes, Diagnostic};
use crate::html_parser::{self as html, Event, Walker};
use crate::{ParserError, Result};
use microsoft_fast_convert::{convert_template, ConvertError};
use std::borrow::Cow;
use std::ops::Range;

pub(crate) const INTERNAL_FAST_BINDING_ATTR: &str = "data-webui-internal-fast-binding";
const CONVERTER_FALLBACK_NAME: &str = "webui-fallback";
const WEBUI_CONVERTER_SYNTAX: &str = "webui-prerelease";

pub(crate) struct PreparedComponentTemplate {
    pub(crate) tag_name: String,
    pub(crate) html_content: String,
    pub(crate) plugin_parse_content: Option<String>,
    pub(crate) plugin_template_content: Option<String>,
}

struct FTemplateSource {
    name: Option<String>,
    start: usize,
    inner: Range<usize>,
}

pub(crate) fn prepare_component_template(
    tag_name: &str,
    html_content: &str,
) -> Result<PreparedComponentTemplate> {
    let Some(source) = find_f_template_source(html_content) else {
        return Ok(PreparedComponentTemplate {
            tag_name: tag_name.to_string(),
            html_content: html_content.to_string(),
            plugin_parse_content: None,
            plugin_template_content: None,
        });
    };

    let resolved_tag = source.name.as_deref().unwrap_or(tag_name).to_string();
    let converter_input = converter_input(html_content, &source);
    let converted = convert_template(&converter_input, WEBUI_CONVERTER_SYNTAX)
        .map_err(|error| converter_error(html_content, source.start, error))?;
    let template_content = html_content[source.inner].trim();
    let html_content = strip_fast_client_attributes(&converted, false);
    let plugin_parse_content = strip_fast_client_attributes(&converted, true);
    let plugin_parse_content =
        (plugin_parse_content != html_content).then_some(plugin_parse_content);
    Ok(PreparedComponentTemplate {
        tag_name: resolved_tag,
        html_content,
        plugin_parse_content,
        plugin_template_content: Some(template_content.to_string()),
    })
}

fn find_f_template_source(html_content: &str) -> Option<FTemplateSource> {
    let mut stack = Vec::with_capacity(1);
    stack.push(0..html_content.len());

    while let Some(range) = stack.pop() {
        for event in Walker::new_range(html_content, range.start, range.end) {
            let Event::Element(element) = event else {
                continue;
            };

            if element.name().eq_ignore_ascii_case("f-template") {
                return Some(FTemplateSource {
                    name: element
                        .attr("name")
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                    start: element.start,
                    inner: element.inner(),
                });
            }

            if !element.self_closing() && !element.is_void() {
                stack.push(element.inner());
            }
        }
    }

    None
}

fn converter_input<'a>(html_content: &'a str, source: &FTemplateSource) -> Cow<'a, str> {
    if source.name.is_some() {
        return Cow::Borrowed(html_content);
    }

    let mut normalized =
        String::with_capacity(html_content.len() + CONVERTER_FALLBACK_NAME.len() + 20);
    normalized.push_str(&html_content[..source.start]);
    normalized.push_str("<f-template name=\"");
    normalized.push_str(CONVERTER_FALLBACK_NAME);
    normalized.push_str("\">");
    normalized.push_str(&html_content[source.inner.start..]);
    Cow::Owned(normalized)
}

#[cold]
#[inline(never)]
fn converter_error(source: &str, offset: usize, error: ConvertError) -> ParserError {
    if matches!(error, ConvertError::MultipleFTemplates { .. }) {
        return Diagnostic::error("multiple <f-template> elements are not supported")
            .code(codes::UNSUPPORTED_MULTIPLE_F_TEMPLATES)
            .at_offset(source, offset)
            .snippet("<f-template>")
            .help(
                "keep only one <f-template> per component file; multiple f-template blocks are not currently supported",
            )
            .into();
    }

    let help = match &error {
        ConvertError::MissingFTemplateName | ConvertError::EmptyFTemplateName => {
            "add a non-empty name attribute to <f-template>, for example <f-template name=\"my-component\">"
        }
        ConvertError::MissingInnerTemplate | ConvertError::MultipleInnerTemplates { .. } => {
            "keep exactly one inner <template> element inside <f-template>"
        }
        ConvertError::MissingValueAttribute { .. }
        | ConvertError::InvalidDirectiveValue { .. } => {
            "add the required value=\"{{expression}}\" attribute to the FAST directive"
        }
        ConvertError::InvalidRepeatExpression { .. } => {
            "use an f-repeat expression in \"item in items\" form"
        }
        ConvertError::UnclosedElement { .. } | ConvertError::UnclosedTag { .. } => {
            "close the reported FAST template element or opening tag"
        }
        ConvertError::UnsupportedFAttribute { .. }
        | ConvertError::UnsupportedFElement { .. } => {
            "remove the unsupported FAST construct or replace it with supported declarative syntax"
        }
        _ => "fix the reported FAST declarative template syntax",
    };

    let code = if matches!(
        error,
        ConvertError::UnclosedElement { .. } | ConvertError::UnclosedTag { .. }
    ) {
        codes::UNCLOSED_HTML_TAG
    } else {
        codes::INVALID_FAST_TEMPLATE
    };

    Diagnostic::error(format!("invalid FAST template: {error}"))
        .code(code)
        .at_offset(source, offset)
        .snippet("<f-template>")
        .help(help)
        .into()
}

fn strip_fast_client_attributes(input: &str, include_binding_markers: bool) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;

    while index < len {
        if bytes[index] == b'<' {
            if let Some(consumed) =
                try_strip_fast_attributes(input, index, &mut result, include_binding_markers)
            {
                index += consumed;
                continue;
            }
        }
        index = push_char_at(input, index, &mut result);
    }

    result
}

fn try_strip_fast_attributes(
    input: &str,
    pos: usize,
    result: &mut String,
    include_binding_markers: bool,
) -> Option<usize> {
    let remaining = &input[pos..];
    let tag = html::parse_tag(remaining)?;

    if tag.closing {
        return None;
    }

    if !tag.attrs().any(|attr| is_fast_client_only_attr(attr.name)) {
        return None;
    }

    push_webui_opening_tag(&tag, result, include_binding_markers);
    Some(tag.close + 1)
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
        result.push_str(attr.raw);
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
