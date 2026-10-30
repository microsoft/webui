// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Strict scanner for trusted template region markers.

use std::collections::HashSet;

use crate::error::{Error, Result};

use super::{Region, REGION_TAG};

const REGION_OPEN: &str = "<webui-press-region";
const REGION_CLOSE: &str = "</webui-press-region>";

pub(super) fn parse_declarations(template: &str) -> Result<Vec<Region>> {
    let mut declarations: Vec<Region> = Vec::new();
    let mut names = HashSet::new();
    let mut parents: Vec<usize> = Vec::new();
    let mut cursor = 0;

    while let Some(offset) = template[cursor..].find('<') {
        let start = cursor + offset;
        if template[start..].starts_with("<!--") {
            cursor = template[start + 4..]
                .find("-->")
                .map_or(template.len(), |end| start + 4 + end + 3);
            continue;
        }
        if template[start..].starts_with(REGION_CLOSE) {
            let index = parents.pop().ok_or_else(|| {
                invalid_declaration("region closing tag has no matching start tag")
            })?;
            let region = &mut declarations[index];
            region.content_end = start;
            region.end = start + REGION_CLOSE.len();
            cursor = region.end;
            continue;
        }
        if !template[start..].starts_with(REGION_OPEN) {
            cursor = start + 1;
            continue;
        }
        let name_end = start + REGION_OPEN.len();
        let Some(next) = template.as_bytes().get(name_end).copied() else {
            return Err(invalid_declaration("region start tag is incomplete"));
        };
        if !next.is_ascii_whitespace() && !matches!(next, b'/' | b'>') {
            cursor = name_end;
            continue;
        }
        let open_end = name_end
            + template[name_end..]
                .find('>')
                .ok_or_else(|| invalid_declaration("region start tag is not closed"))?;
        let raw_attributes = &template[name_end..open_end];
        let self_closing = raw_attributes.trim_end().ends_with('/');
        let (name, layout) = parse_attributes(raw_attributes, self_closing)?;
        if !names.insert(name.clone()) {
            return Err(Error::Build(format!(
                "Template region '{name}' is declared more than once."
            )));
        }

        let end = open_end + 1;
        let index = declarations.len();
        declarations.push(Region {
            start,
            end,
            content_start: end,
            content_end: end,
            parent: parents.last().copied(),
            name,
            layout,
            html: None,
            state: None,
            script_file: None,
            inactive: false,
        });
        if !self_closing {
            parents.push(index);
        }
        cursor = end;
    }
    if let Some(index) = parents.last() {
        return Err(Error::Build(format!(
            "Template region '{}' has no closing tag.",
            declarations[*index].name
        )));
    }

    Ok(declarations)
}

fn parse_attributes(raw: &str, self_closing: bool) -> Result<(String, Option<String>)> {
    let mut remaining = raw.trim();
    if self_closing {
        remaining = remaining
            .strip_suffix('/')
            .map(str::trim_end)
            .unwrap_or(remaining);
    }
    let mut name = None;
    let mut layout = None;

    while !remaining.is_empty() {
        let name_end = remaining
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '=')
            .unwrap_or(remaining.len());
        if name_end == 0 {
            return Err(invalid_declaration("attributes are malformed"));
        }
        let attribute = &remaining[..name_end];
        remaining = remaining[name_end..].trim_start();
        let Some(after_equals) = remaining.strip_prefix('=') else {
            return Err(attribute_error(attribute, "requires a quoted value"));
        };
        remaining = after_equals.trim_start();
        let Some(quote @ ('"' | '\'')) = remaining.chars().next() else {
            return Err(attribute_error(attribute, "requires a quoted value"));
        };
        let value_start = quote.len_utf8();
        let Some(value_end) = remaining[value_start..].find(quote) else {
            return Err(attribute_error(attribute, "has an unclosed quoted value"));
        };
        let value = &remaining[value_start..value_start + value_end];
        if value.is_empty() {
            return Err(attribute_error(attribute, "requires a non-empty value"));
        }
        remaining = remaining[value_start + value_end + quote.len_utf8()..].trim_start();

        let slot = if attribute.eq_ignore_ascii_case("name") {
            &mut name
        } else if attribute.eq_ignore_ascii_case("layout") {
            &mut layout
        } else {
            return Err(invalid_declaration(&format!(
                "unsupported attribute '{attribute}'; use only 'name' and optional 'layout'"
            )));
        };
        if slot.replace(value.to_string()).is_some() {
            return Err(attribute_error(attribute, "is declared more than once"));
        }
    }

    let name = name
        .filter(|value| valid_region_name(value))
        .ok_or_else(|| {
            invalid_declaration("region name is missing or contains invalid characters")
        })?;
    Ok((name, layout))
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
fn attribute_error(attribute: &str, message: &str) -> Error {
    invalid_declaration(&format!("attribute '{attribute}' {message}"))
}

#[cold]
#[inline(never)]
fn invalid_declaration(message: &str) -> Error {
    Error::Build(format!("Invalid <{REGION_TAG}> declaration: {message}."))
}
