// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compiler-owned CSS boundary for Light DOM components.

use crate::diagnostic::{codes, Diagnostic};
use crate::{comment_policy, ParserError, Result};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Rules,
    Style,
    Keyframes,
}

struct KeyframeName {
    authored: String,
    compiled: String,
}

struct KeyframeRule {
    name_start: usize,
    name_end: usize,
    block_start: usize,
}

struct AnimationValue {
    range: Range<usize>,
    is_name_property: bool,
}

struct Declaration {
    property: Range<usize>,
    value: Range<usize>,
}

#[derive(Default)]
struct AnimationShorthandState {
    timing: bool,
    iteration: bool,
    direction: bool,
    fill: bool,
    play: bool,
    timeline: bool,
}

enum AtRule {
    Grouping { block_start: usize },
    Keyframes(KeyframeRule),
}

/// Compile developer-authored component CSS for one Light DOM component.
pub(crate) fn compile(tag_name: &str, source: &str) -> Result<String> {
    if source.trim().is_empty() {
        return Ok(String::new());
    }

    let keyframes = collect_keyframes_and_validate(tag_name, source)?;
    let rewritten = rewrite_css(tag_name, source, &keyframes)?;
    let mut output = String::with_capacity(tag_name.len() + rewritten.len() + 58);
    output.push_str("@scope (");
    output.push_str(tag_name);
    output.push_str("[data-wl]) to (:scope [data-wl] > *) {\n");
    output.push_str(&rewritten);
    output.push_str("\n}");
    Ok(output)
}

fn collect_keyframes_and_validate(tag_name: &str, source: &str) -> Result<Vec<KeyframeName>> {
    let bytes = source.as_bytes();
    let mut keyframes = Vec::new();
    let mut blocks = vec![BlockKind::Rules];
    let mut pending_block = None;
    let mut index = 0usize;
    let mut segment_start = true;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut nested_brace_depth = 0usize;

    while index < bytes.len() {
        if segment_start && current_block(&blocks) == BlockKind::Style {
            if let Some(declaration) = declaration_at(source, index) {
                index = declaration.value.end;
                segment_start = false;
                continue;
            }
        }

        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'@' if segment_start => {
                match parse_at_rule(tag_name, source, index)? {
                    AtRule::Keyframes(rule) => {
                        let authored_token = &source[rule.name_start..rule.name_end];
                        let authored = keyframe_name_value(authored_token);
                        if !keyframes
                            .iter()
                            .any(|keyframe: &KeyframeName| keyframe.authored == authored.as_str())
                        {
                            keyframes.push(KeyframeName {
                                authored,
                                compiled: compiled_keyframe_name(tag_name, authored_token),
                            });
                        }
                        pending_block = Some((rule.block_start, BlockKind::Keyframes));
                        index = rule.name_end;
                    }
                    AtRule::Grouping { block_start } => {
                        pending_block = Some((block_start, current_block(&blocks)));
                        index += 1;
                    }
                }
                segment_start = false;
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
                segment_start = false;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
                segment_start = false;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth > 0 || bracket_depth > 0 => {
                nested_brace_depth += 1;
                index += 1;
            }
            b'{' => {
                blocks.push(block_for_open(&mut pending_block, index));
                index += 1;
                segment_start = true;
            }
            b'}' if nested_brace_depth > 0 => {
                nested_brace_depth -= 1;
                index += 1;
            }
            b'}' => {
                if blocks.len() > 1 {
                    blocks.pop();
                }
                index += 1;
                segment_start = true;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => {
                index += 1;
                segment_start = true;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                index = next_char_boundary(source, index);
                segment_start = false;
            }
        }
    }

    Ok(keyframes)
}

fn rewrite_css(tag_name: &str, source: &str, keyframes: &[KeyframeName]) -> Result<String> {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len() + keyframes.len() * 12);
    let mut blocks = vec![BlockKind::Rules];
    let mut pending_block = None;
    let mut copy_start = 0usize;
    let mut index = 0usize;
    let mut segment_start = true;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut nested_brace_depth = 0usize;

    while index < bytes.len() {
        if segment_start && current_block(&blocks) == BlockKind::Style {
            if let Some(declaration) = declaration_at(source, index) {
                if is_animation_property(&source[declaration.property.clone()]) {
                    let property = &source[declaration.property.clone()];
                    let is_name_property = css_identifier_eq(property, "animation-name")
                        || css_identifier_eq(property, "-webkit-animation-name");
                    output.push_str(&source[copy_start..declaration.value.start]);
                    rewrite_animation_value(
                        tag_name,
                        source,
                        AnimationValue {
                            range: declaration.value.clone(),
                            is_name_property,
                        },
                        keyframes,
                        &mut output,
                    )?;
                    copy_start = declaration.value.end;
                }
                index = declaration.value.end;
                segment_start = false;
                continue;
            }
        }

        if bytes[index] == b'"' || bytes[index] == b'\'' {
            index = quoted_end(source, index);
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = block_comment_end(source, index);
            continue;
        }
        if bytes[index] == b'/' && comment_policy::is_css_line_comment_start(source, index) {
            index = comment_policy::find_css_line_comment_end(source, index + 2);
            continue;
        }

        if bytes[index] == b':' {
            if let Some(pseudo) = pseudo_name(source, index) {
                let raw_name = &source[pseudo.name.clone()];
                let decoded_name = raw_name
                    .contains('\\')
                    .then(|| keyframe_name_value(raw_name));
                let name = decoded_name.as_deref().unwrap_or(raw_name);
                if !pseudo.is_element && name.eq_ignore_ascii_case("host-context") {
                    return Err(unsupported_light_css_error(
                        tag_name,
                        source,
                        index,
                        "`:host-context()` cannot be isolated in Light DOM",
                        "move the contextual rule to entry CSS, or wrap the component in `<template shadowrootmode=\"open\">`",
                    ));
                }
                if pseudo.is_element && name.eq_ignore_ascii_case("slotted") {
                    return Err(unsupported_light_css_error(
                        tag_name,
                        source,
                        index,
                        "`::slotted()` requires Shadow DOM slot projection",
                        "wrap the component in `<template shadowrootmode=\"open\">` to use `::slotted()`",
                    ));
                }
                if !pseudo.is_element && name.eq_ignore_ascii_case("host") {
                    let end = rewrite_host_selector(
                        tag_name,
                        source,
                        index..pseudo.name.end,
                        &mut output,
                        copy_start,
                    )?;
                    copy_start = end;
                    index = end;
                    segment_start = false;
                    continue;
                }
            }
        }

        if bytes[index] == b'@' && segment_start {
            match parse_at_rule(tag_name, source, index)? {
                AtRule::Keyframes(rule) => {
                    let authored = &source[rule.name_start..rule.name_end];
                    if let Some(keyframe) = find_keyframe(keyframes, authored) {
                        output.push_str(&source[copy_start..rule.name_start]);
                        output.push_str(&keyframe.compiled);
                        copy_start = rule.name_end;
                    }
                    pending_block = Some((rule.block_start, BlockKind::Keyframes));
                    index = rule.name_end;
                }
                AtRule::Grouping { block_start } => {
                    pending_block = Some((block_start, current_block(&blocks)));
                    index += 1;
                }
            }
            segment_start = false;
            continue;
        }

        match bytes[index] {
            b'(' => {
                paren_depth += 1;
                index += 1;
                segment_start = false;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
                segment_start = false;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth > 0 || bracket_depth > 0 => {
                nested_brace_depth += 1;
                index += 1;
            }
            b'{' => {
                blocks.push(block_for_open(&mut pending_block, index));
                index += 1;
                segment_start = true;
            }
            b'}' if nested_brace_depth > 0 => {
                nested_brace_depth -= 1;
                index += 1;
            }
            b'}' => {
                if blocks.len() > 1 {
                    blocks.pop();
                }
                index += 1;
                segment_start = true;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => {
                index += 1;
                segment_start = true;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                index = next_char_boundary(source, index);
                segment_start = false;
            }
        }
    }

    output.push_str(&source[copy_start..]);
    Ok(output)
}

fn rewrite_host_selector(
    tag_name: &str,
    source: &str,
    pseudo: Range<usize>,
    output: &mut String,
    copy_start: usize,
) -> Result<usize> {
    output.push_str(&source[copy_start..pseudo.start]);
    let argument_start = pseudo.end;
    if source.as_bytes().get(argument_start) != Some(&b'(') {
        output.push_str(":scope");
        return Ok(argument_start);
    }

    let Some(close) = matching_paren_end(source, argument_start) else {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            pseudo.start,
            "unterminated `:host()` selector",
            "close the `:host(...)` selector before building",
        ));
    };
    if !has_significant_content(source, argument_start + 1..close - 1) {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            pseudo.start,
            "empty `:host()` selector",
            "use `:host` without parentheses, or provide a compound selector",
        ));
    }
    validate_host_compound(tag_name, source, argument_start + 1..close - 1)?;
    validate_shadow_only_pseudos(tag_name, source, argument_start + 1..close - 1)?;
    output.push_str(":scope:is(");
    output.push_str(&source[argument_start + 1..close - 1]);
    output.push(')');
    Ok(close)
}

fn rewrite_animation_value(
    tag_name: &str,
    source: &str,
    value: AnimationValue,
    keyframes: &[KeyframeName],
    output: &mut String,
) -> Result<()> {
    let bytes = source.as_bytes();
    let mut copy_start = value.range.start;
    let mut index = value.range.start;
    let mut paren_depth = 0usize;
    let mut shorthand = AnimationShorthandState::default();

    while index < value.range.end {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = block_comment_end(source, index).min(value.range.end);
            continue;
        }
        if bytes[index] == b'/' && comment_policy::is_css_line_comment_start(source, index) {
            index =
                comment_policy::find_css_line_comment_end(source, index + 2).min(value.range.end);
            continue;
        }
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            let token_end = quoted_end(source, index).min(value.range.end);
            if paren_depth == 0 {
                if let Some(keyframe) = find_keyframe(keyframes, &source[index..token_end]) {
                    output.push_str(&source[copy_start..index]);
                    output.push_str(&keyframe.compiled);
                    copy_start = token_end;
                }
            }
            index = token_end;
            continue;
        }
        if bytes[index] == b'(' {
            paren_depth += 1;
            index += 1;
            continue;
        }
        if bytes[index] == b')' {
            paren_depth = paren_depth.saturating_sub(1);
            index += 1;
            continue;
        }
        if bytes[index] == b',' && paren_depth == 0 {
            shorthand = AnimationShorthandState::default();
            index += 1;
            continue;
        }
        if is_identifier_token_start(bytes, index, value.range.start) {
            let token_end = ident_end(bytes, index, value.range.end);
            let token = &source[index..token_end];
            let decoded_token = token.contains('\\').then(|| keyframe_name_value(token));
            let semantic_token = decoded_token.as_deref().unwrap_or(token);
            let is_function = bytes.get(token_end) == Some(&b'(');
            if is_function && !keyframes.is_empty() && is_dynamic_css_function(semantic_token) {
                return Err(dynamic_keyframe_error(tag_name, source, index));
            }
            let may_be_name = paren_depth == 0
                && (value.is_name_property
                    || shorthand.identifier_may_be_name(semantic_token, is_function));
            if may_be_name && !is_function {
                if let Some(keyframe) = find_keyframe(keyframes, token) {
                    output.push_str(&source[copy_start..index]);
                    output.push_str(&keyframe.compiled);
                    copy_start = token_end;
                }
            }
            index = token_end;
            continue;
        }
        index = next_char_boundary(source, index);
    }

    output.push_str(&source[copy_start..value.range.end]);
    Ok(())
}

fn declaration_at(source: &str, start: usize) -> Option<Declaration> {
    let property_start = skip_whitespace_and_comments(source, start);
    let bytes = source.as_bytes();
    if !bytes
        .get(property_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return None;
    }
    let property_end = ident_end(bytes, property_start, bytes.len());
    let colon = skip_whitespace_and_comments(source, property_end);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    let value_start = colon + 1;
    Some(Declaration {
        property: property_start..property_end,
        value: value_start..declaration_value_end(source, value_start),
    })
}

fn declaration_value_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth == 0 && bracket_depth == 0 => {
                brace_depth += 1;
                index += 1;
            }
            b'}' if paren_depth == 0 && bracket_depth == 0 && brace_depth > 0 => {
                brace_depth -= 1;
                index += 1;
            }
            b';' | b'}' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                break;
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    index
}

fn parse_at_rule(tag_name: &str, source: &str, start: usize) -> Result<AtRule> {
    let (name, keyword_end) = at_keyword(source, start);
    if is_keyframes_keyword(name) {
        return parse_keyframe_rule(tag_name, source, keyword_end).map(AtRule::Keyframes);
    }
    if !is_scoped_grouping_keyword(name) {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            start,
            "unsupported global at-rule in Light component CSS",
            "move global CSS to the entry stylesheet, or use a block @media, @supports, @container, @layer, or @scope rule",
        ));
    }
    match at_rule_terminator(source, keyword_end) {
        Some((block_start, b'{')) => Ok(AtRule::Grouping { block_start }),
        _ => Err(unsupported_light_css_error(
            tag_name,
            source,
            start,
            "unscopable at-rule statement in Light component CSS",
            "use the block form of this grouping rule, or move the statement to the entry stylesheet",
        )),
    }
}

fn parse_keyframe_rule(tag_name: &str, source: &str, keyword_end: usize) -> Result<KeyframeRule> {
    let name_start = skip_whitespace_and_comments(source, keyword_end);
    if matches!(source.as_bytes().get(name_start), Some(b'"' | b'\'')) {
        let name_end = quoted_end(source, name_start);
        let block_start = skip_whitespace_and_comments(source, name_end);
        if source.as_bytes().get(block_start) != Some(&b'{') {
            return Err(unsupported_light_css_error(
                tag_name,
                source,
                name_start,
                "unsupported keyframe name syntax in Light DOM",
                "use a static CSS identifier or string for the keyframe name",
            ));
        }
        return Ok(KeyframeRule {
            name_start,
            name_end,
            block_start,
        });
    }
    if !source
        .as_bytes()
        .get(name_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            keyword_end,
            "keyframes require a static identifier name in Light DOM",
            "use an identifier such as `@keyframes fade`, then reference that name statically",
        ));
    }
    let name_end = ident_end(source.as_bytes(), name_start, source.len());
    if name_start == name_end {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            keyword_end,
            "keyframes require a static identifier name in Light DOM",
            "use an identifier such as `@keyframes fade`, then reference that name statically",
        ));
    }
    let block_start = skip_whitespace_and_comments(source, name_end);
    if source.as_bytes().get(block_start) != Some(&b'{') {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            name_start,
            "unsupported keyframe name syntax in Light DOM",
            "use an unquoted CSS identifier for the keyframe name",
        ));
    }
    Ok(KeyframeRule {
        name_start,
        name_end,
        block_start,
    })
}

fn at_keyword(source: &str, start: usize) -> (&str, usize) {
    let end = ident_end(source.as_bytes(), start + 1, source.len());
    (&source[start + 1..end], end)
}

fn is_keyframes_keyword(name: &str) -> bool {
    css_identifier_eq(name, "keyframes") || css_identifier_eq(name, "-webkit-keyframes")
}

fn is_scoped_grouping_keyword(name: &str) -> bool {
    css_identifier_eq(name, "media")
        || css_identifier_eq(name, "supports")
        || css_identifier_eq(name, "container")
        || css_identifier_eq(name, "layer")
        || css_identifier_eq(name, "scope")
        || css_identifier_eq(name, "starting-style")
}

fn find_keyframe<'a>(keyframes: &'a [KeyframeName], name: &str) -> Option<&'a KeyframeName> {
    if !matches!(name.as_bytes().first(), Some(b'"' | b'\'')) && !name.contains('\\') {
        return keyframes.iter().find(|keyframe| keyframe.authored == name);
    }
    let decoded = keyframe_name_value(name);
    keyframes
        .iter()
        .find(|keyframe| keyframe.authored == decoded)
}

fn keyframe_name_value(token: &str) -> String {
    let quoted = matches!(token.as_bytes().first(), Some(b'"' | b'\''));
    if !quoted && !token.contains('\\') {
        return token.to_string();
    }
    let bytes = token.as_bytes();
    let mut value = String::with_capacity(token.len().saturating_sub(2 * usize::from(quoted)));
    let mut index = usize::from(quoted);
    let end = token.len().saturating_sub(usize::from(quoted));
    while index < end {
        if bytes[index] != b'\\' {
            let next = next_char_boundary(token, index).min(end);
            value.push_str(&token[index..next]);
            index = next;
            continue;
        }
        index += 1;
        if index >= end {
            break;
        }
        if bytes[index].is_ascii_hexdigit() {
            let mut codepoint = 0u32;
            let mut digits = 0usize;
            while index < end && digits < 6 && bytes[index].is_ascii_hexdigit() {
                codepoint = codepoint * 16 + u32::from(hex_value(bytes[index]));
                index += 1;
                digits += 1;
            }
            if index < end && bytes[index].is_ascii_whitespace() {
                if bytes[index] == b'\r' && index + 1 < end && bytes.get(index + 1) == Some(&b'\n')
                {
                    index += 1;
                }
                index += 1;
            }
            value.push(if codepoint == 0 {
                char::REPLACEMENT_CHARACTER
            } else {
                char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER)
            });
        } else if matches!(bytes[index], b'\n' | b'\r' | b'\x0c') {
            if bytes[index] == b'\r' && index + 1 < end && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            index += 1;
        } else {
            let next = next_char_boundary(token, index).min(end);
            value.push_str(&token[index..next]);
            index = next;
        }
    }
    value
}

fn css_identifier_eq(identifier: &str, expected: &str) -> bool {
    if identifier.contains('\\') {
        keyframe_name_value(identifier).eq_ignore_ascii_case(expected)
    } else {
        identifier.eq_ignore_ascii_case(expected)
    }
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

fn compiled_keyframe_name(tag_name: &str, authored: &str) -> String {
    let quote = authored
        .as_bytes()
        .first()
        .copied()
        .filter(|byte| matches!(*byte, b'"' | b'\''));
    let mut compiled = String::with_capacity(tag_name.len() + authored.len() + 5);
    if let Some(quote) = quote {
        compiled.push(char::from(quote));
    }
    compiled.push_str("wui-");
    compiled.push_str(tag_name);
    compiled.push('-');
    if let Some(quote) = quote {
        compiled.push_str(&authored[1..authored.len() - 1]);
        compiled.push(char::from(quote));
    } else {
        compiled.push_str(authored);
    }
    compiled
}

fn current_block(blocks: &[BlockKind]) -> BlockKind {
    blocks.last().copied().unwrap_or(BlockKind::Rules)
}

fn block_for_open(pending: &mut Option<(usize, BlockKind)>, index: usize) -> BlockKind {
    match *pending {
        Some((block_start, block)) if block_start == index => {
            *pending = None;
            block
        }
        _ => BlockKind::Style,
    }
}

fn is_animation_property(property: &str) -> bool {
    css_identifier_eq(property, "animation")
        || css_identifier_eq(property, "animation-name")
        || css_identifier_eq(property, "-webkit-animation")
        || css_identifier_eq(property, "-webkit-animation-name")
}

fn is_dynamic_css_function(name: &str) -> bool {
    name.eq_ignore_ascii_case("var")
        || name.eq_ignore_ascii_case("attr")
        || name.eq_ignore_ascii_case("env")
}

impl AnimationShorthandState {
    fn identifier_may_be_name(&mut self, name: &str, is_function: bool) -> bool {
        if is_function {
            if ["cubic-bezier", "linear", "steps"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function))
            {
                self.timing = true;
            } else if ["scroll", "view"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function))
            {
                self.timeline = true;
            }
            return false;
        }

        if ["inherit", "initial", "revert", "revert-layer", "unset"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            return false;
        }

        let slot = if [
            "ease",
            "ease-in",
            "ease-in-out",
            "ease-out",
            "linear",
            "step-end",
            "step-start",
        ]
        .iter()
        .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.timing)
        } else if name.eq_ignore_ascii_case("infinite") {
            Some(&mut self.iteration)
        } else if ["alternate", "alternate-reverse", "normal", "reverse"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.direction)
        } else if ["backwards", "both", "forwards", "none"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.fill)
        } else if ["paused", "running"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.play)
        } else if name.eq_ignore_ascii_case("auto") {
            Some(&mut self.timeline)
        } else {
            return true;
        };

        if let Some(consumed) = slot {
            let was_consumed = *consumed;
            *consumed = true;
            was_consumed
        } else {
            true
        }
    }
}

struct PseudoName {
    name: Range<usize>,
    is_element: bool,
}

fn pseudo_name(source: &str, start: usize) -> Option<PseudoName> {
    let bytes = source.as_bytes();
    let is_element = bytes.get(start + 1) == Some(&b':');
    let name_start = start + 1 + usize::from(is_element);
    if !bytes
        .get(name_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return None;
    }

    let name_end = ident_end(bytes, name_start, bytes.len());
    Some(PseudoName {
        name: name_start..name_end,
        is_element,
    })
}

fn validate_shadow_only_pseudos(tag_name: &str, source: &str, range: Range<usize>) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = range.start;
    while index < range.end {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index).min(range.end),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index).min(range.end);
            }
            b':' => {
                if let Some(pseudo) = pseudo_name(source, index) {
                    let raw_name = &source[pseudo.name.clone()];
                    let decoded_name = raw_name
                        .contains('\\')
                        .then(|| keyframe_name_value(raw_name));
                    let name = decoded_name.as_deref().unwrap_or(raw_name);
                    if !pseudo.is_element && name.eq_ignore_ascii_case("host-context") {
                        return Err(unsupported_light_css_error(
                            tag_name,
                            source,
                            index,
                            "`:host-context()` cannot be isolated in Light DOM",
                            "move the contextual rule to entry CSS, or wrap the component in `<template shadowrootmode=\"open\">`",
                        ));
                    }
                    if pseudo.is_element && name.eq_ignore_ascii_case("slotted") {
                        return Err(unsupported_light_css_error(
                            tag_name,
                            source,
                            index,
                            "`::slotted()` requires Shadow DOM slot projection",
                            "wrap the component in `<template shadowrootmode=\"open\">` to use `::slotted()`",
                        ));
                    }
                }
                index += 1;
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    Ok(())
}

fn validate_host_compound(tag_name: &str, source: &str, range: Range<usize>) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = range.start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut saw_token = false;
    let mut whitespace_after_token = false;
    while index < range.end {
        match bytes[index] {
            b'"' | b'\'' => {
                saw_token = true;
                whitespace_after_token = false;
                index = quoted_end(source, index).min(range.end);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index).min(range.end);
            }
            b'(' => {
                paren_depth += 1;
                saw_token = true;
                whitespace_after_token = false;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                saw_token = true;
                whitespace_after_token = false;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b',' if paren_depth == 0 && bracket_depth == 0 => {
                saw_token = false;
                whitespace_after_token = false;
                index += 1;
            }
            b'>' | b'+' | b'~' if paren_depth == 0 && bracket_depth == 0 => {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            b'|' if paren_depth == 0
                && bracket_depth == 0
                && bytes.get(index + 1) == Some(&b'|') =>
            {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            byte if byte.is_ascii_whitespace() && paren_depth == 0 && bracket_depth == 0 => {
                whitespace_after_token |= saw_token;
                index += 1;
            }
            _ if paren_depth == 0 && bracket_depth == 0 && whitespace_after_token => {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            _ => {
                saw_token = true;
                whitespace_after_token = false;
                index = next_char_boundary(source, index);
            }
        }
    }
    Ok(())
}

fn non_compound_host_error(tag_name: &str, source: &str, offset: usize) -> ParserError {
    unsupported_light_css_error(
        tag_name,
        source,
        offset,
        "`:host()` requires a compound selector",
        "remove descendant or child combinators from `:host(...)`, and place them after the closing parenthesis",
    )
}

fn ident_end(bytes: &[u8], mut index: usize, limit: usize) -> usize {
    while index < limit {
        if is_ident_byte(bytes[index]) {
            index += 1;
        } else if bytes[index] == b'\\' {
            index = css_escape_end(bytes, index, limit);
        } else {
            break;
        }
    }
    index
}

fn is_ident_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'-' | b'_' | b'\\') || byte >= 0x80
}

fn is_identifier_token_start(bytes: &[u8], index: usize, start: usize) -> bool {
    is_ident_start_byte(bytes[index])
        && (index == start || !is_ident_byte(bytes[index - 1]) && bytes[index - 1] != b'\\')
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80
}

fn css_escape_end(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut index = start + 1;
    if index >= limit {
        return index;
    }
    if bytes[index].is_ascii_hexdigit() {
        let mut digits = 0usize;
        while index < limit && digits < 6 && bytes[index].is_ascii_hexdigit() {
            index += 1;
            digits += 1;
        }
        if index < limit && bytes[index].is_ascii_whitespace() {
            if bytes[index] == b'\r' && index + 1 < limit && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            index += 1;
        }
        index
    } else {
        next_char_boundary_from_bytes(bytes, index, limit)
    }
}

fn skip_whitespace_and_comments(source: &str, mut index: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            index = block_comment_end(source, index);
        } else {
            return index;
        }
    }
}

fn has_significant_content(source: &str, range: Range<usize>) -> bool {
    let bytes = source.as_bytes();
    let mut index = range.start;
    while index < range.end {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            index = block_comment_end(source, index).min(range.end);
        } else {
            return true;
        }
    }
    false
}

fn at_rule_terminator(source: &str, start: usize) -> Option<(usize, u8)> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            byte @ (b'{' | b';') if paren_depth == 0 && bracket_depth == 0 => {
                return Some((index, byte));
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    None
}

fn matching_paren_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = open + 1;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    None
}

fn quoted_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn next_char_boundary(source: &str, index: usize) -> usize {
    next_char_boundary_from_bytes(source.as_bytes(), index, source.len())
}

fn next_char_boundary_from_bytes(bytes: &[u8], index: usize, limit: usize) -> usize {
    if index >= limit || bytes[index].is_ascii() {
        return (index + 1).min(limit);
    }
    let width = if bytes[index] & 0b1111_1000 == 0b1111_0000 {
        4
    } else if bytes[index] & 0b1111_0000 == 0b1110_0000 {
        3
    } else {
        2
    };
    (index + width).min(limit)
}

fn block_comment_end(source: &str, start: usize) -> usize {
    source[start + 2..]
        .find("*/")
        .map_or(source.len(), |offset| start + offset + 4)
}

#[cold]
#[inline(never)]
fn unsupported_light_css_error(
    tag_name: &str,
    source: &str,
    offset: usize,
    title: &str,
    help: &str,
) -> ParserError {
    Diagnostic::error(title)
        .code(codes::UNSUPPORTED_LIGHT_CSS)
        .component(tag_name)
        .at_offset(source, offset)
        .snippet(super::source_line_snippet(source, offset))
        .help(help)
        .into()
}

#[cold]
#[inline(never)]
fn dynamic_keyframe_error(tag_name: &str, source: &str, offset: usize) -> ParserError {
    Diagnostic::error("dynamic keyframe references cannot be namespaced for Light DOM")
        .code(codes::DYNAMIC_LIGHT_KEYFRAME)
        .component(tag_name)
        .at_offset(source, offset)
        .snippet(super::source_line_snippet(source, offset))
        .help("use a static animation name, or move the animation to entry CSS")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_rules_in_component_scope() {
        let css = compile("my-card", ".label { color: red; }").expect("compile");
        assert_eq!(
            css,
            "@scope (my-card[data-wl]) to (:scope [data-wl] > *) {\n.label { color: red; }\n}"
        );
    }

    #[test]
    fn lowers_composed_host_selectors() {
        let source = concat!(
            ":host { display:block }",
            ":host:has(.error) { color:red }",
            ":host([disabled]):hover { opacity:.5 }",
            ":host:not([hidden])::before { content:\"x\" }",
            r":h\6fst(.escaped) { color:blue }"
        );
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains(":scope { display:block }"));
        assert!(css.contains(":scope:has(.error) { color:red }"));
        assert!(css.contains(":scope:is([disabled]):hover { opacity:.5 }"));
        assert!(css.contains(":scope:not([hidden])::before { content:\"x\" }"));
        assert!(css.contains(":scope:is(.escaped) { color:blue }"));
    }

    #[test]
    fn leaves_host_text_in_strings_and_comments() {
        let source =
            ".x::before{content:\":host\";--selector: :host;background:url(:host)}/* :host */";
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("content:\":host\""));
        assert!(css.contains("--selector: :host"));
        assert!(css.contains("url(:host)"));
        assert!(css.contains("/* :host */"));
    }

    #[test]
    fn preserves_nested_grouping_rules() {
        let source =
            "@media (width > 10px) { @supports (display:grid) { :host { display:grid } } }";
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("@media"));
        assert!(css.contains("@supports"));
        assert!(css.contains(":scope { display:grid }"));
    }

    #[test]
    fn preserves_authored_scope_and_non_ascii_selectors() {
        let source = "@scope (.é) { @layer card { :host(.wide) > .标题 { color:red } } }";
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("@scope (.é)"));
        assert!(css.contains(":scope:is(.wide) > .标题"));
    }

    #[test]
    fn rejects_shadow_only_selectors() {
        for selector in [
            ":host-context(body)",
            "::slotted(*)",
            ":HOST-CONTEXT(main)",
            ":host(::slotted(*))",
        ] {
            let error = compile("my-card", &format!("{selector}{{color:red}}"))
                .expect_err("selector must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
            ));
        }
    }

    #[test]
    fn rejects_non_compound_host_argument() {
        let error = compile("my-card", ":host(.card > .label){color:red}").expect_err("must fail");
        assert!(matches!(
            error,
            ParserError::Template(ref diagnostic)
                if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
                    && diagnostic.help_text().is_some()
        ));
    }

    #[test]
    fn namespaces_keyframes_and_static_references() {
        let source = concat!(
            "@keyframes fade { from { opacity:0 } to { opacity:1 } }",
            ".x { animation: fade 1s ease; animation-name: fade; }"
        );
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("@keyframes wui-my-card-fade"));
        assert!(css.contains("animation: wui-my-card-fade 1s ease"));
        assert!(css.contains("animation-name: wui-my-card-fade"));
    }

    #[test]
    fn rewrites_comma_separated_and_vendor_animation_references() {
        let source = concat!(
            "@keyframes fade{to{opacity:1}}",
            "@-webkit-keyframes spin{to{rotate:1turn}}",
            ".x{-webkit-animation:spin 1s, fade 2s ease;",
            "animation-name:fade, spin;content:\"fade\";--animation:fade}"
        );
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("-webkit-animation:wui-my-card-spin 1s, wui-my-card-fade 2s ease"));
        assert!(css.contains("animation-name:wui-my-card-fade, wui-my-card-spin"));
        assert!(css.contains("content:\"fade\";--animation:fade"));
    }

    #[test]
    fn rewrites_quoted_keyframe_names_only_in_animation_syntax() {
        let source = concat!(
            "@keyframes \"fade in\"{to{opacity:1}}",
            ".x{animation-name:'fade in';content:\"fade in\"}"
        );
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("@keyframes \"wui-my-card-fade in\""));
        assert!(css.contains("animation-name:\"wui-my-card-fade in\""));
        assert!(css.contains("content:\"fade in\""));
    }

    #[test]
    fn matches_equivalent_escaped_keyframe_identifiers() {
        let source = r"@keyframes f\61 de{to{opacity:1}}.x{animation-name:fa\64 e}";
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains(r"@keyframes wui-my-card-f\61 de"));
        assert!(css.contains(r"animation-name:wui-my-card-f\61 de"));
    }

    #[test]
    fn does_not_rewrite_shorthand_keywords_or_dimension_units() {
        let source = concat!(
            "@keyframes linear{to{opacity:1}}",
            "@keyframes s{to{opacity:1}}",
            ".x{animation:other 1s linear;",
            "animation:1s linear linear;",
            "animation-name:linear,s}"
        );
        let css = compile("my-card", source).expect("compile");
        assert!(css.contains("animation:other 1s linear"));
        assert!(css.contains("animation:1s linear wui-my-card-linear"));
        assert!(css.contains("animation-name:wui-my-card-linear,wui-my-card-s"));
    }

    #[test]
    fn rejects_dynamic_keyframe_references() {
        for function in ["var(--animation)", "ATTR(data-animation)"] {
            let source = format!("@keyframes fade{{to{{opacity:1}}}}.x{{animation:{function}}}");
            let error = compile("my-card", &source).expect_err("dynamic reference must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::DYNAMIC_LIGHT_KEYFRAME)
                        && diagnostic.component_name() == Some("my-card")
                        && diagnostic.help_text().is_some()
                        && diagnostic.snippet_text().is_some()
                        && diagnostic.position_line_column().is_some()
            ));
        }
    }

    #[test]
    fn allows_dynamic_global_animation_without_local_keyframes() {
        let css = compile("my-card", ".x{animation:var(--animation)}").expect("compile");
        assert!(css.contains("animation:var(--animation)"));
    }

    #[test]
    fn rejects_global_at_rules() {
        for source in [
            "@font-face{font-family:x;src:url(x)}",
            "@layer reset;",
            "@import url(global.css);",
        ] {
            let error = compile("my-card", source).expect_err("must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
            ));
        }
    }

    #[test]
    fn preserves_source_bytes_inside_generated_scope() {
        let source = " \n:host { color:red }\n ";
        let css = compile("my-card", source).expect("compile");
        assert_eq!(
            css,
            "@scope (my-card[data-wl]) to (:scope [data-wl] > *) {\n \n:scope { color:red }\n \n}"
        );
    }
}
