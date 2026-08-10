// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{ConvertError, ConvertErrorKind};
use crate::html_parser::Tag;

const F_REPEAT_NAME: &str = "f-repeat";
const F_WHEN_NAME: &str = "f-when";

pub(super) fn validate_directive_attributes<'a>(
    tag: &Tag<'a>,
    tag_offset: usize,
) -> Result<(), ConvertError<'a>> {
    for attr in tag.attrs() {
        if attr.name.starts_with("f-") {
            return Err(ConvertError::new(
                ConvertErrorKind::UnsupportedFAttribute {
                    attribute: attr.name,
                },
                tag_offset + attr.raw_range.start,
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_attributes<'a>(
    tag: &Tag<'a>,
    tag_offset: usize,
) -> Result<(), ConvertError<'a>> {
    for attr in tag.attrs() {
        if attr.name.starts_with("f-") && !is_supported_f_attribute(attr.name) {
            return Err(ConvertError::new(
                ConvertErrorKind::UnsupportedFAttribute {
                    attribute: attr.name,
                },
                tag_offset + attr.raw_range.start,
            ));
        }
    }
    Ok(())
}

pub(super) fn directive_expression<'a>(
    tag: &Tag<'a>,
    kind: DirectiveKind,
    tag_offset: usize,
) -> Result<&'a str, ConvertError<'a>> {
    let value_attr = tag.attrs().find(|attr| attr.name == "value");
    let Some(value_attr) = value_attr else {
        return Err(ConvertError::new(
            ConvertErrorKind::MissingValueAttribute {
                tag: kind.tag_name(),
            },
            tag_offset,
        ));
    };
    let Some(value) = value_attr.value else {
        return Err(ConvertError::new(
            ConvertErrorKind::MissingValueAttribute {
                tag: kind.tag_name(),
            },
            tag_offset + value_attr.raw_range.start,
        ));
    };
    let value_offset = tag_offset + value_attr.raw_range.start;
    let trimmed = value.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") || trimmed.len() <= 4 {
        return Err(ConvertError::new(
            ConvertErrorKind::InvalidDirectiveValue {
                tag: kind.tag_name(),
                value,
            },
            value_offset,
        ));
    }

    let expression = trimmed[2..trimmed.len() - 2].trim();
    if expression.is_empty() {
        return Err(ConvertError::new(
            ConvertErrorKind::InvalidDirectiveValue {
                tag: kind.tag_name(),
                value,
            },
            value_offset,
        ));
    }
    Ok(expression)
}

pub(super) fn is_repeat_expression(expression: &str) -> bool {
    let mut parts = expression.split_whitespace();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(alias), Some("in"), Some(path), None) => is_identifier(alias) && is_path(path),
        _ => false,
    }
}

#[inline]
fn is_supported_f_attribute(name: &str) -> bool {
    matches!(name, "f-ref" | "f-children" | "f-slotted")
}

fn is_path(path: &str) -> bool {
    let mut parts = path.split('.');
    parts.next().is_some_and(is_identifier) && parts.all(is_identifier)
}

fn is_identifier(identifier: &str) -> bool {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectiveKind {
    Repeat,
    When,
}

impl DirectiveKind {
    #[inline]
    pub(super) fn from_tag_name(name: &str) -> Option<Self> {
        match name {
            F_REPEAT_NAME => Some(Self::Repeat),
            F_WHEN_NAME => Some(Self::When),
            _ => None,
        }
    }

    #[inline]
    pub(super) fn tag_name(self) -> &'static str {
        match self {
            Self::Repeat => F_REPEAT_NAME,
            Self::When => F_WHEN_NAME,
        }
    }

    #[inline]
    pub(super) fn output_open(self) -> &'static str {
        match self {
            Self::Repeat => "<for each=\"",
            Self::When => "<if condition=\"",
        }
    }

    #[inline]
    pub(super) fn output_close(self) -> &'static str {
        match self {
            Self::Repeat => "</for>",
            Self::When => "</if>",
        }
    }
}
