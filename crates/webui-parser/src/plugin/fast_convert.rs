// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Focused FAST declarative-template conversion for WebUI SSR parsing.

mod directive;
mod error;
mod scan;

pub(super) use error::{ConvertError, ConvertErrorKind};

use crate::html_parser::{find_comment_close, find_matching_end, parse_tag, Tag};
use directive::{
    directive_expression, is_repeat_expression, validate_attributes, validate_directive_attributes,
    DirectiveKind,
};
use scan::{find_tag_end, read_opening_tag_name, scan_named_open_tags};
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

    let f_template_end = find_tag_end(source, f_template_start, source.len())
        .ok_or_else(|| ConvertError::new(ConvertErrorKind::UnclosedTag, f_template_start))?;
    let f_template = parse_tag(&source[f_template_start..f_template_end])
        .ok_or_else(|| ConvertError::new(ConvertErrorKind::UnclosedTag, f_template_start))?;
    if f_template.self_closing {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: f_template.name,
            },
            f_template_start,
        ));
    }
    let (f_template_close, f_template_close_end) =
        find_matching_end(source, f_template.name, f_template_end).ok_or_else(|| {
            ConvertError::new(
                ConvertErrorKind::UnclosedElement {
                    tag: f_template.name,
                },
                f_template_start,
            )
        })?;
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

    let inner_open_end = find_tag_end(source, inner_start, artifact.end).ok_or_else(|| {
        ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: INNER_TEMPLATE_NAME,
            },
            inner_start,
        )
    })?;
    let inner_template = parse_tag(&source[inner_start..inner_open_end]).ok_or_else(|| {
        ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: INNER_TEMPLATE_NAME,
            },
            inner_start,
        )
    })?;
    if inner_template.self_closing {
        return Err(ConvertError::new(
            ConvertErrorKind::UnclosedElement {
                tag: inner_template.name,
            },
            inner_start,
        ));
    }
    let (_, inner_close_end) =
        find_matching_end(&source[..artifact.end], inner_template.name, inner_open_end)
            .ok_or_else(|| {
                ConvertError::new(
                    ConvertErrorKind::UnclosedElement {
                        tag: inner_template.name,
                    },
                    inner_start,
                )
            })?;

    let parser_content = convert_segment(source, inner_start..inner_close_end)?;
    let name = f_template
        .attr("name")
        .map(str::trim)
        .filter(|name| !name.is_empty());

    Ok(Some(ConvertedTemplate {
        name,
        artifact,
        parser_content,
    }))
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
        directives: Vec::with_capacity(4),
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

        let Some(end) = find_tag_end(source, start, range.end) else {
            if read_opening_tag_name(source, start, range.end).is_some() {
                return Err(ConvertError::new(ConvertErrorKind::UnclosedTag, start));
            }
            state.output.push('<');
            cursor = start + 1;
            continue;
        };
        convert_tag(source, start, end, &mut state)?;
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

fn convert_tag<'a>(
    source: &'a str,
    start: usize,
    end: usize,
    state: &mut ConvertState,
) -> Result<(), ConvertError<'a>> {
    let Some(tag) = parse_tag(&source[start..end]) else {
        state.output.push_str(&source[start..end]);
        return Ok(());
    };

    let raw = &source[start..end];
    if tag.closing {
        convert_closing_tag(&tag, raw, state)
    } else {
        convert_opening_tag(&tag, raw, start, state)
    }
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
    validate_directive_attributes(tag, offset)?;
    let expression = directive_expression(tag, kind, offset)?;
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

    state.output.push_str(kind.output_open());
    state.output.push_str(expression);
    state.output.push_str("\">");
    state.directives.push(DirectiveFrame { kind, offset });
    Ok(())
}
