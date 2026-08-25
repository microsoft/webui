// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Deterministic scanner for compile-time region declarations.

use std::collections::HashSet;

use crate::error::{Error, Result};

use super::REGION_TAG;

const HTML_COMMENT_OPEN: &str = "<!--";
const HTML_COMMENT_CLOSE: &str = "-->";

#[derive(Debug)]
pub(super) struct RegionDeclaration {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) name: String,
    pub(super) layout: Option<String>,
}

pub(super) fn parse_declarations(template: &str) -> Result<Vec<RegionDeclaration>> {
    let mut declarations = Vec::new();
    let mut names = HashSet::new();
    let mut cursor = 0;

    while let Some(offset) = template[cursor..].find('<') {
        let start = cursor + offset;
        if template[start..].starts_with(HTML_COMMENT_OPEN) {
            let comment_start = start + HTML_COMMENT_OPEN.len();
            let Some(close_offset) = template[comment_start..].find(HTML_COMMENT_CLOSE) else {
                break;
            };
            cursor = comment_start + close_offset + HTML_COMMENT_CLOSE.len();
            continue;
        }

        let bytes = template.as_bytes();
        let mut name_start = start + 1;
        let is_end = bytes.get(name_start) == Some(&b'/');
        if is_end {
            name_start += 1;
        }
        let Some(first) = bytes.get(name_start).copied() else {
            break;
        };
        if matches!(first, b'!' | b'?') {
            cursor = find_tag_end(template, start).map_or(template.len(), |end| end + 1);
            continue;
        }

        let mut name_end = name_start;
        while bytes
            .get(name_end)
            .is_some_and(|byte| is_tag_name_byte(*byte))
        {
            name_end += 1;
        }
        if name_end == name_start {
            cursor = template[name_start..]
                .chars()
                .next()
                .map_or(template.len(), |ch| name_start + ch.len_utf8());
            continue;
        }

        let tag_name = &template[name_start..name_end];
        let is_region = tag_name.eq_ignore_ascii_case(REGION_TAG);
        let open_end =
            find_tag_end(template, start).ok_or_else(|| invalid_unclosed_tag(is_region))?;
        let self_closing = template[name_end..open_end].trim_end().ends_with('/');

        if !is_end && !self_closing && is_raw_text_tag(tag_name) {
            cursor = if tag_name.eq_ignore_ascii_case("plaintext") {
                template.len()
            } else {
                find_closing_tag(template, open_end + 1, tag_name)
                    .map_or(template.len(), |(_, end)| end)
            };
            continue;
        }
        if is_end || !is_region {
            cursor = open_end + 1;
            continue;
        }

        let attributes = parse_region_attributes(&template[name_end..open_end])?;
        let name = attributes.name;
        if !names.insert(name.clone()) {
            return Err(Error::Build(format!(
                "Template region '{name}' is declared more than once."
            )));
        }
        let end = if self_closing {
            open_end + 1
        } else {
            let content_start = open_end + 1;
            let (close_start, close_end) = find_closing_tag(template, content_start, REGION_TAG)
                .ok_or_else(|| {
                    Error::Build(format!("Template region '{name}' has no closing tag."))
                })?;
            if !template[content_start..close_start].trim().is_empty() {
                return Err(Error::Build(format!(
                    "Template region '{name}' must be empty; configure its content in config.json."
                )));
            }
            close_end
        };
        declarations.push(RegionDeclaration {
            start,
            end,
            name,
            layout: attributes.layout,
        });
        cursor = end;
    }

    Ok(declarations)
}

struct RegionAttributes {
    name: String,
    layout: Option<String>,
}

fn parse_region_attributes(raw: &str) -> Result<RegionAttributes> {
    let bytes = raw.as_bytes();
    let mut cursor = 0;
    let mut name = None;
    let mut layout = None;

    while cursor < bytes.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        if bytes.get(cursor) == Some(&b'/') && raw[cursor + 1..].trim().is_empty() {
            break;
        }

        let attribute_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| is_attribute_name_byte(*byte))
        {
            cursor += 1;
        }
        if attribute_start == cursor {
            return Err(invalid_declaration("attributes are malformed"));
        }
        let attribute = &raw[attribute_start..cursor];
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            return Err(invalid_declaration(&format!(
                "attribute '{attribute}' requires a non-empty value"
            )));
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let (value, next) = parse_attribute_value(raw, cursor)?;
        cursor = next;
        if value.is_empty() {
            return Err(invalid_declaration(&format!(
                "attribute '{attribute}' requires a non-empty value"
            )));
        }

        if attribute.eq_ignore_ascii_case("name") {
            if name.replace(value).is_some() {
                return Err(invalid_declaration(&format!(
                    "attribute '{attribute}' is declared more than once"
                )));
            }
        } else if attribute.eq_ignore_ascii_case("layout") {
            if layout.replace(value).is_some() {
                return Err(invalid_declaration(&format!(
                    "attribute '{attribute}' is declared more than once"
                )));
            }
        } else {
            return Err(invalid_declaration(&format!(
                "unsupported attribute '{attribute}'; use only 'name' and optional 'layout'"
            )));
        }
    }

    let name = name
        .filter(|value| valid_region_name(value))
        .ok_or_else(|| {
            invalid_declaration("region name is missing or contains invalid characters")
        })?;
    Ok(RegionAttributes { name, layout })
}

fn parse_attribute_value(raw: &str, cursor: usize) -> Result<(String, usize)> {
    let bytes = raw.as_bytes();
    match bytes.get(cursor).copied() {
        Some(quote @ (b'"' | b'\'')) => {
            let value_start = cursor + 1;
            let Some(close_offset) = raw[value_start..].find(char::from(quote)) else {
                return Err(invalid_declaration("quoted attribute value is not closed"));
            };
            let value_end = value_start + close_offset;
            Ok((raw[value_start..value_end].to_string(), value_end + 1))
        }
        Some(_) => {
            let value_start = cursor;
            let mut value_end = cursor;
            while bytes
                .get(value_end)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'/')
            {
                value_end += 1;
            }
            Ok((raw[value_start..value_end].to_string(), value_end))
        }
        None => Err(invalid_declaration("attribute value is missing")),
    }
}

fn find_tag_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut quote = None;
    let mut cursor = start;
    while let Some(byte) = bytes.get(cursor).copied() {
        match (quote, byte) {
            (Some(active), current) if active == current => quote = None,
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn find_closing_tag(source: &str, start: usize, target: &str) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = start;
    while let Some(offset) = source[cursor..].find('<') {
        let tag_start = cursor + offset;
        let name_start = tag_start + 2;
        if bytes.get(tag_start + 1) != Some(&b'/') {
            cursor = tag_start + 1;
            continue;
        }
        let mut name_end = name_start;
        while bytes
            .get(name_end)
            .is_some_and(|byte| is_tag_name_byte(*byte))
        {
            name_end += 1;
        }
        let has_tag_name_delimiter = bytes
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'));
        if has_tag_name_delimiter && source[name_start..name_end].eq_ignore_ascii_case(target) {
            let end = find_tag_end(source, tag_start)?;
            return Some((tag_start, end + 1));
        }
        cursor = name_end.max(tag_start + 1);
    }
    None
}

fn is_raw_text_tag(name: &str) -> bool {
    name.eq_ignore_ascii_case("script")
        || name.eq_ignore_ascii_case("style")
        || name.eq_ignore_ascii_case("textarea")
        || name.eq_ignore_ascii_case("title")
        || name.eq_ignore_ascii_case("xmp")
        || name.eq_ignore_ascii_case("iframe")
        || name.eq_ignore_ascii_case("noembed")
        || name.eq_ignore_ascii_case("noframes")
        || name.eq_ignore_ascii_case("noscript")
        || name.eq_ignore_ascii_case("plaintext")
}

fn is_tag_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

fn is_attribute_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn valid_region_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && name.split('.').all(|segment| !segment.is_empty())
}

#[cold]
#[inline(never)]
fn invalid_declaration(message: &str) -> Error {
    Error::Build(format!("Invalid <{REGION_TAG}> declaration: {message}."))
}

#[cold]
#[inline(never)]
fn invalid_unclosed_tag(is_region: bool) -> Error {
    if is_region {
        invalid_declaration("region start tag is not closed")
    } else {
        Error::Build("Template contains an unclosed HTML start tag.".to_string())
    }
}
