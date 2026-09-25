// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Focused FAST declarative-template conversion for WebUI SSR parsing.

mod directive;
mod error;
mod scan;

pub(super) use error::{ConvertError, ConvertErrorKind};

use crate::html_parser::{
    find_comment_close, find_raw_text_end, is_raw_text_element, parse_tag, Tag,
};
use directive::{
    has_f_prefix, is_repeat_expression, parse_directive, validate_attributes, DirectiveKind,
};
use scan::{
    find_matching_end_skip_raw_text, find_tag_end, read_opening_tag_name, scan_named_open_tags,
};
use std::ops::Range;

pub(super) const F_TEMPLATE_NAME: &str = "f-template";
const INNER_TEMPLATE_NAME: &str = "template";

// Build-time FAST style placeholder removed before parsing.
const STYLES_MARKER: &str = "{{styles}}";

// Prefix for declarative shadow-root options on `<f-template>`.
const SHADOW_ROOT_ATTR_PREFIX: &[u8] = b"shadowroot";

// Converted parser and artifact views for one authored `<f-template>`.
pub(super) struct ConvertedTemplate<'a> {
    pub(super) name: Option<&'a str>,
    // Inner `<template>` retained for the client artifact.
    pub(super) artifact_content: String,
    pub(super) parser_content: String,
}

// Convert one `<f-template>` to parser syntax, or return `None` when absent.
pub(super) fn convert_template(
    source: &str,
) -> Result<Option<ConvertedTemplate<'_>>, ConvertError<'_>> {
    let f_templates = scan_named_open_tags(source, 0..source.len(), F_TEMPLATE_NAME);
    let Some(f_template_start) = f_templates.first else {
        return Ok(None);
    };
    if f_templates.count != 1 {
        return Err(ConvertError::new(
            ConvertErrorKind::MultipleFTemplates {
                count: f_templates.count,
            },
            f_template_start,
        ));
    }

    let f_template = parse_tag(&source[f_template_start..])
        .ok_or_else(|| ConvertError::new(ConvertErrorKind::UnclosedTag, f_template_start))?;
    let f_template_end = f_template_start + f_template.close + 1;
    if f_template.self_closing {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: f_template.name,
            },
            f_template_start,
        ));
    }
    let (f_template_close, f_template_close_end) =
        find_matching_end_skip_raw_text(source, f_template.name, f_template_end).ok_or_else(
            || {
                ConvertError::new(
                    ConvertErrorKind::UnclosedElement {
                        tag: f_template.name,
                    },
                    f_template_start,
                )
            },
        )?;
    if first_non_whitespace_non_comment(source, 0..f_template_start).is_some()
        || first_non_whitespace_non_comment(source, f_template_close_end..source.len()).is_some()
    {
        let offset = first_non_whitespace_non_comment(source, 0..f_template_start)
            .or_else(|| {
                first_non_whitespace_non_comment(source, f_template_close_end..source.len())
            })
            .unwrap_or(f_template_start);
        return Err(ConvertError::new(
            ConvertErrorKind::ContentOutsideTemplate,
            offset,
        ));
    }
    let body = f_template_end..f_template_close;
    let inner_templates = scan_named_open_tags(source, body.clone(), INNER_TEMPLATE_NAME);
    let Some(inner_start) = inner_templates.first else {
        return Err(ConvertError::new(
            ConvertErrorKind::MissingInnerTemplate,
            f_template_start,
        ));
    };

    let inner_template = parse_tag(&source[inner_start..body.end]).ok_or_else(|| {
        ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: INNER_TEMPLATE_NAME,
            },
            inner_start,
        )
    })?;
    let inner_open_end = inner_start + inner_template.close + 1;
    if inner_template.self_closing {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: inner_template.name,
            },
            inner_start,
        ));
    }
    let (_, inner_close_end) =
        find_matching_end_skip_raw_text(&source[..body.end], inner_template.name, inner_open_end)
            .ok_or_else(|| {
            ConvertError::new(
                ConvertErrorKind::UnclosedElement {
                    tag: inner_template.name,
                },
                inner_start,
            )
        })?;

    // The inner `<template>` is the sole supported content of `<f-template>`.
    // Reject meaningful siblings around it (before `inner_start` or after
    // `inner_close_end`) so they are never silently dropped from the SSR view
    // while surviving in the client artifact — only whitespace and comments may
    // surround it. This also keeps the artifact anchored to the inner
    // `<template>` so it can never be re-wrapped in an outer `<template>`.
    if let Some(offset) = first_non_whitespace_non_comment(source, body.start..inner_start)
        .or_else(|| first_non_whitespace_non_comment(source, inner_close_end..body.end))
    {
        return Err(ConvertError::new(
            ConvertErrorKind::ContentAroundInnerTemplate,
            offset,
        ));
    }

    // Resolve the wrapper's `name` and declarative-shadow-root options in a
    // single attribute walk, rejecting any other wrapper attribute rather than
    // silently discarding it.
    let (name, shadow_options) = resolve_wrapper_attrs(&f_template, f_template_start)?;

    // The wrapper's shadow-root options belong on the inner `<template>` (the
    // WebUI declarative shadow root), and the generator `{{styles}}` marker is
    // removed so WebUI's own CSS strategy supplies styles at that position.
    // Both the SSR parser view and the retained client artifact receive the
    // same rewrite so their binding order stays aligned.
    let converted = convert_segment(source, inner_start..inner_close_end)?;
    let parser_content = rewrite_inner_template(&converted, &shadow_options).unwrap_or(converted);
    let artifact_source = &source[inner_start..inner_close_end];
    let artifact_content = rewrite_inner_template(artifact_source, &shadow_options)
        .unwrap_or_else(|| artifact_source.to_string());

    Ok(Some(ConvertedTemplate {
        name,
        artifact_content,
        parser_content,
    }))
}

// Resolve the wrapper name and shadow-root options in one attribute walk.
fn resolve_wrapper_attrs<'a>(
    tag: &Tag<'a>,
    wrapper_start: usize,
) -> Result<(Option<&'a str>, String), ConvertError<'a>> {
    let mut name = None;
    let mut shadow_options = String::new();
    for attr in tag.attrs() {
        if attr.name.eq_ignore_ascii_case("name") {
            name = attr.value.map(str::trim).filter(|value| !value.is_empty());
        } else if is_shadow_root_attr(attr.name) {
            shadow_options.push(' ');
            shadow_options.push_str(attr.raw);
        } else {
            return Err(ConvertError::new(
                ConvertErrorKind::UnsupportedWrapperAttribute {
                    attribute: attr.name,
                },
                wrapper_start + attr.raw_range.start,
            ));
        }
    }
    Ok((name, shadow_options))
}

// Whether `name` is a declarative-shadow-root option (`shadowroot*`).
#[inline]
fn is_shadow_root_attr(name: &str) -> bool {
    name.len() >= SHADOW_ROOT_ATTR_PREFIX.len()
        && name.as_bytes()[..SHADOW_ROOT_ATTR_PREFIX.len()]
            .eq_ignore_ascii_case(SHADOW_ROOT_ATTR_PREFIX)
}

// Apply shadow options and remove only a generated leading `{{styles}}`.
fn rewrite_inner_template(inner: &str, shadow_options: &str) -> Option<String> {
    let tag = parse_tag(inner)?;
    if !tag.name.eq_ignore_ascii_case(INNER_TEMPLATE_NAME) {
        return None;
    }
    let name_end = 1 + tag.name.len();
    let open_end = tag.close + 1;
    let after_open = &inner[open_end..];
    let leading_ws = after_open.len() - after_open.trim_start().len();
    let marker_present = after_open[leading_ws..].starts_with(STYLES_MARKER);

    if shadow_options.is_empty() && !marker_present {
        return None;
    }

    let mut out = String::with_capacity(inner.len() + shadow_options.len());
    out.push_str(&inner[..name_end]);
    out.push_str(shadow_options);
    out.push_str(&inner[name_end..open_end]);
    if marker_present {
        out.push_str(&after_open[..leading_ws]);
        out.push_str(&after_open[leading_ws + STYLES_MARKER.len()..]);
    } else {
        out.push_str(after_open);
    }
    Some(out)
}

fn first_non_whitespace_non_comment(source: &str, range: Range<usize>) -> Option<usize> {
    let mut cursor = range.start;
    let bytes = source.as_bytes();
    while cursor < range.end {
        while cursor < range.end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= range.end {
            return None;
        }
        let remaining = &source[cursor..range.end];
        if remaining.starts_with("<!--") {
            if let Some(length) = find_comment_close(remaining) {
                cursor += length;
                continue;
            }
        }
        return Some(cursor);
    }
    None
}

// Iterative conversion state for nested FAST directives.
struct ConvertState {
    output: String,
    directives: Vec<DirectiveFrame>,
}

#[derive(Clone, Copy)]
struct DirectiveFrame {
    kind: DirectiveKind,
    offset: usize,
}

fn convert_segment<'a>(source: &'a str, range: Range<usize>) -> Result<String, ConvertError<'a>> {
    let mut state = ConvertState {
        output: String::with_capacity(range.len()),
        // Static templates never push a directive frame, so defer the heap
        // allocation until the first `<f-when>`/`<f-repeat>` is converted. The
        // first push still allocates the same small capacity, so directive-heavy
        // templates are unaffected.
        directives: Vec::new(),
    };
    let mut cursor = range.start;

    while cursor < range.end {
        let Some(relative) = source[cursor..range.end].find('<') else {
            state.output.push_str(&source[cursor..range.end]);
            break;
        };
        let start = cursor + relative;
        state.output.push_str(&source[cursor..start]);

        let remaining = &source[start..range.end];
        if remaining.starts_with("<!--") {
            let Some(length) = find_comment_close(remaining) else {
                state.output.push_str(remaining);
                cursor = range.end;
                continue;
            };
            let end = start + length;
            state.output.push_str(&source[start..end]);
            cursor = end;
            continue;
        }

        let Some(tag) = parse_tag(remaining) else {
            // Not a parseable element: a declaration/empty-name tag with a
            // terminator is copied verbatim, an unterminated `<name` is an
            // unclosed-tag diagnostic, and a bare `<` is emitted as text.
            if let Some(end) = find_tag_end(source, start, range.end) {
                state.output.push_str(&source[start..end]);
                cursor = end;
            } else if read_opening_tag_name(source, start, range.end).is_some() {
                return Err(ConvertError::new(ConvertErrorKind::UnclosedTag, start));
            } else {
                state.output.push('<');
                cursor = start + 1;
            }
            continue;
        };

        // Raw-text elements (`<script>`, `<style>`, …) are opaque: copy the
        // whole element verbatim so tag-like text inside is never mistaken for a
        // FAST directive or wrapper.
        if !tag.closing && is_raw_text_element(tag.name) {
            let raw_end = start + find_raw_text_end(remaining, tag.name, tag.close + 1);
            state.output.push_str(&source[start..raw_end]);
            cursor = raw_end;
            continue;
        }

        let end = start + tag.close + 1;
        let raw = &source[start..end];
        if tag.closing {
            convert_closing_tag(&tag, raw, start, &mut state)?;
        } else {
            convert_opening_tag(&tag, raw, start, &mut state)?;
        }
        cursor = end;
    }

    if let Some(frame) = state.directives.first().copied() {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: frame.kind.tag_name(),
            },
            frame.offset,
        ));
    }

    Ok(state.output)
}

fn convert_opening_tag<'a>(
    tag: &Tag<'a>,
    raw: &'a str,
    start: usize,
    state: &mut ConvertState,
) -> Result<(), ConvertError<'a>> {
    if let Some(kind) = DirectiveKind::from_tag_name(tag.name) {
        return convert_directive(tag, kind, start, state);
    }
    if has_f_prefix(tag.name) {
        return Err(ConvertError::new(
            ConvertErrorKind::UnsupportedFElement { tag: tag.name },
            start,
        ));
    }

    validate_attributes(tag, start)?;
    state.output.push_str(raw);
    Ok(())
}

fn convert_closing_tag<'a>(
    tag: &Tag<'a>,
    raw: &'a str,
    start: usize,
    state: &mut ConvertState,
) -> Result<(), ConvertError<'a>> {
    let Some(kind) = DirectiveKind::from_tag_name(tag.name) else {
        // An unsupported `f-*` closing tag (`</f-foo>`, or a stray
        // `</f-template>`) has no supported opening form and must not leak into
        // the SSR view; reject it at its own offset, mirroring the unsupported
        // opening-element rejection. Ordinary closing tags pass through.
        if has_f_prefix(tag.name) {
            return Err(ConvertError::new(
                ConvertErrorKind::UnsupportedFElement { tag: tag.name },
                start,
            ));
        }
        state.output.push_str(raw);
        return Ok(());
    };

    if state.directives.last().map(|frame| frame.kind) == Some(kind) {
        state.directives.pop();
        state.output.push_str(kind.output_close());
        return Ok(());
    }
    if state.directives.iter().any(|frame| frame.kind == kind) {
        if let Some(unclosed) = state.directives.last().copied() {
            return Err(ConvertError::new(
                ConvertErrorKind::UnclosedElement {
                    tag: unclosed.kind.tag_name(),
                },
                unclosed.offset,
            ));
        }
    }

    // No matching opening directive anywhere on the stack: a stray
    // `</f-when>`/`</f-repeat>` is malformed FAST and must be rejected at its
    // own offset rather than copied verbatim into the WebUI parser view.
    Err(ConvertError::new(
        ConvertErrorKind::UnexpectedClosingDirective {
            tag: kind.tag_name(),
        },
        start,
    ))
}

fn convert_directive<'a>(
    tag: &Tag<'a>,
    kind: DirectiveKind,
    offset: usize,
    state: &mut ConvertState,
) -> Result<(), ConvertError<'a>> {
    let expression = parse_directive(tag, kind, offset)?;
    if kind == DirectiveKind::Repeat && !is_repeat_expression(expression) {
        return Err(ConvertError::new(
            ConvertErrorKind::InvalidRepeatExpression { expr: expression },
            offset,
        ));
    }
    if tag.self_closing {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: kind.tag_name(),
            },
            offset,
        ));
    }

    push_directive_open(state, kind, expression, offset)?;
    state.directives.push(DirectiveFrame { kind, offset });
    Ok(())
}

// Emit a converted directive with a safe attribute delimiter.
fn push_directive_open<'a>(
    state: &mut ConvertState,
    kind: DirectiveKind,
    expression: &'a str,
    offset: usize,
) -> Result<(), ConvertError<'a>> {
    match kind {
        DirectiveKind::Repeat => {
            state.output.push_str("<for each=\"");
            state.output.push_str(expression);
            state.output.push_str("\">");
        }
        DirectiveKind::When => {
            let Some(delimiter) = condition_attribute_delimiter(expression) else {
                return Err(ConvertError::new(
                    ConvertErrorKind::ConditionQuoteConflict { value: expression },
                    offset,
                ));
            };
            state.output.push_str("<if condition=");
            state.output.push(char::from(delimiter));
            state.output.push_str(expression);
            state.output.push(char::from(delimiter));
            state.output.push('>');
        }
    }
    Ok(())
}

// Choose a raw delimiter absent from the condition.
fn condition_attribute_delimiter(expression: &str) -> Option<u8> {
    let bytes = expression.as_bytes();
    match (bytes.contains(&b'"'), bytes.contains(&b'\'')) {
        (true, true) => None,
        (true, false) => Some(b'\''),
        (false, _) => Some(b'"'),
    }
}
