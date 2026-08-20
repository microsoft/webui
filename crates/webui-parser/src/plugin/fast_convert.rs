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
use directive::{is_repeat_expression, parse_directive, validate_attributes, DirectiveKind};
use scan::{
    find_matching_end_skip_raw_text, find_tag_end, read_opening_tag_name, scan_named_open_tags,
};
use std::ops::Range;

pub(super) const F_TEMPLATE_NAME: &str = "f-template";
const INNER_TEMPLATE_NAME: &str = "template";

/// Converted parser and artifact views for one authored `<f-template>`.
pub(super) struct ConvertedTemplate<'a> {
    pub(super) name: Option<&'a str>,
    pub(super) artifact: Range<usize>,
    pub(super) parser_content: String,
}

/// Convert the single `<f-template>` in `source` to WebUI parser syntax.
///
/// Returns `None` when no `<f-template>` element is present.
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
    let artifact = f_template_end..f_template_close;
    let inner_templates = scan_named_open_tags(source, artifact.clone(), INNER_TEMPLATE_NAME);
    let Some(inner_start) = inner_templates.first else {
        return Err(ConvertError::new(
            ConvertErrorKind::MissingInnerTemplate,
            f_template_start,
        ));
    };
    if inner_templates.count != 1 {
        return Err(ConvertError::new(
            ConvertErrorKind::MultipleInnerTemplates {
                count: inner_templates.count,
            },
            f_template_start,
        ));
    }

    let inner_template = parse_tag(&source[inner_start..artifact.end]).ok_or_else(|| {
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
    let (_, inner_close_end) = find_matching_end_skip_raw_text(
        &source[..artifact.end],
        inner_template.name,
        inner_open_end,
    )
    .ok_or_else(|| {
        ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: inner_template.name,
            },
            inner_start,
        )
    })?;

    let parser_content = convert_segment(source, inner_start..inner_close_end)?;
    let name = attr_ignore_ascii_case(&f_template, "name")
        .map(str::trim)
        .filter(|name| !name.is_empty());

    Ok(Some(ConvertedTemplate {
        name,
        artifact,
        parser_content,
    }))
}

/// Case-insensitive attribute lookup scoped to FAST wrapper resolution.
///
/// FAST `<f-template>` wrappers are recognized ASCII-case-insensitively, so
/// their `name` attribute must be resolved the same way (`<F-TEMPLATE NAME=…>`).
/// This stays inside the FAST plugin subtree; the generic [`Tag::attr`] remains
/// case-sensitive for WebUI directives.
fn attr_ignore_ascii_case<'a>(tag: &Tag<'a>, name: &str) -> Option<&'a str> {
    tag.attrs()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
        .and_then(|attr| attr.value)
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

/// Iterative conversion state for nested FAST directives.
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
            convert_closing_tag(&tag, raw, &mut state)?;
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
    if tag.name.starts_with("f-") {
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
    state: &mut ConvertState,
) -> Result<(), ConvertError<'a>> {
    let Some(kind) = DirectiveKind::from_tag_name(tag.name) else {
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

    state.output.push_str(raw);
    Ok(())
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

/// Emit the WebUI opening tag for a converted FAST directive.
///
/// `<f-repeat>` expressions are validated to the quote-free `item in items`
/// grammar, so the `<for each="…">` value is always safe in double quotes.
/// `<f-when>` conditions may contain a string literal, so the generated
/// `<if condition=…>` delimiter is chosen to avoid clashing with a quote in the
/// expression (the WebUI parser reads the raw attribute value without entity
/// decoding, so entity escaping is not an option).
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

/// Choose the quote delimiter for a generated `<if condition=…>` attribute.
///
/// Returns the ASCII delimiter byte that does not appear in `expression`, so
/// the WebUI parser extracts the condition verbatim. A double quote is
/// preferred (byte-identical to the historical output) and a single quote is
/// used when the expression contains a double-quoted literal. Returns `None`
/// when the expression contains both quote styles and cannot be represented
/// with a raw attribute delimiter.
fn condition_attribute_delimiter(expression: &str) -> Option<u8> {
    let bytes = expression.as_bytes();
    match (bytes.contains(&b'"'), bytes.contains(&b'\'')) {
        (true, true) => None,
        (true, false) => Some(b'\''),
        (false, _) => Some(b'"'),
    }
}
