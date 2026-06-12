// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! CSS scanner for WebUI components.

use crate::{comment_policy, LegalComments, ParserError, Result};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

/// Parser for CSS files.
pub struct CssParser;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CssComment {
    pub start_byte: usize,
    pub end_byte: usize,
    pub preserve: bool,
}

impl fmt::Debug for CssParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CssParser").finish()
    }
}

impl CssParser {
    /// Create a new CSS parser.
    pub fn new() -> Self {
        Self
    }

    /// Extract CSS custom property token names used via `var()` in the given CSS.
    pub fn extract_tokens(&mut self, css_content: &str) -> Result<HashSet<String>> {
        let (mut tokens, definitions, _comments) =
            scan_css(css_content, LegalComments::Inline, false)?;
        tokens.retain(|t| !definitions.contains(t));
        Ok(tokens)
    }

    /// Extract CSS custom property definitions from the given CSS.
    pub fn extract_definitions(&mut self, css_content: &str) -> Result<HashSet<String>> {
        let (_tokens, definitions, _comments) =
            scan_css(css_content, LegalComments::Inline, false)?;
        Ok(definitions)
    }

    /// Extract both token usages and definitions in a single scan.
    pub fn extract_tokens_and_definitions(
        &mut self,
        css_content: &str,
    ) -> Result<(HashSet<String>, HashSet<String>)> {
        let (mut tokens, definitions, _comments) =
            scan_css(css_content, LegalComments::Inline, false)?;
        tokens.retain(|t| !definitions.contains(t));
        Ok((tokens, definitions))
    }

    /// Extract tokens, definitions, and CSS with removable comments stripped in one scan.
    pub(crate) fn extract_tokens_definitions_and_strip_comments<'a>(
        &mut self,
        css_content: &'a str,
        legal_comments: LegalComments,
    ) -> Result<(HashSet<String>, HashSet<String>, Cow<'a, str>)> {
        let (mut tokens, definitions, comments) = scan_css(css_content, legal_comments, true)?;
        tokens.retain(|t| !definitions.contains(t));
        let mut comment_ranges = removable_ranges(&comments);
        let stripped = comment_policy::strip_ranges(css_content, comment_ranges.as_mut_slice());
        Ok((tokens, definitions, stripped))
    }

    /// Extract tokens, definitions, and CSS comments in one scan.
    pub(crate) fn extract_tokens_definitions_and_comments(
        &mut self,
        css_content: &str,
        legal_comments: LegalComments,
    ) -> Result<(HashSet<String>, HashSet<String>, Vec<CssComment>)> {
        let (mut tokens, definitions, mut comments) = scan_css(css_content, legal_comments, true)?;
        tokens.retain(|t| !definitions.contains(t));
        comments.sort_unstable_by_key(|comment| comment.start_byte);
        Ok((tokens, definitions, comments))
    }
}

fn scan_css(
    source: &str,
    legal_comments: LegalComments,
    collect_comments: bool,
) -> Result<(HashSet<String>, HashSet<String>, Vec<CssComment>)> {
    let bytes = source.as_bytes();
    let mut tokens = HashSet::new();
    let mut definitions = HashSet::new();
    let mut comments = Vec::new();
    let mut index = 0usize;
    let mut quote: u8 = 0;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    while index < bytes.len() {
        if quote != 0 {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == quote {
                quote = 0;
            }
            index += 1;
            continue;
        }

        match bytes[index] {
            b'"' | b'\'' => {
                quote = bytes[index];
                index += 1;
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                let Some(offset) = source[index + 2..].find("*/") else {
                    return Err(ParserError::Css(format!(
                        "Unterminated CSS block comment near byte {index}. Close the comment with `*/` before building."
                    )));
                };
                let end = index + 2 + offset + 2;
                if collect_comments {
                    let comment = &source[index..end];
                    comments.push(CssComment {
                        start_byte: index,
                        end_byte: end,
                        preserve: comment_policy::should_preserve_css_comment(
                            comment,
                            legal_comments,
                        ),
                    });
                }
                index = end;
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                let end = comment_policy::find_css_line_comment_end(source, index + 2);
                if collect_comments {
                    let comment = &source[index..end];
                    comments.push(CssComment {
                        start_byte: index,
                        end_byte: end,
                        preserve: comment_policy::should_preserve_css_comment(
                            comment,
                            legal_comments,
                        ),
                    });
                }
                index = end;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                if let Some((name, end)) = parse_custom_property_name(source, index) {
                    if is_custom_property_definition(source, end) {
                        definitions.insert(name.to_string());
                    }
                    index = end;
                } else {
                    index += 1;
                }
            }
            b'v' if source[index..].starts_with("var(") => {
                index = scan_var_call(source, index, &mut tokens)?;
            }
            b'{' => {
                brace_depth += 1;
                index += 1;
            }
            b'}' => {
                if brace_depth == 0 {
                    return Err(ParserError::Css(format!(
                        "Unexpected CSS closing brace near byte {index}. Remove the extra `}}` or add a matching opening `{{`."
                    )));
                }
                brace_depth -= 1;
                index += 1;
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                if paren_depth == 0 {
                    return Err(ParserError::Css(format!(
                        "Unexpected CSS closing parenthesis near byte {index}. Remove the extra `)` or add a matching opening `(`."
                    )));
                }
                paren_depth -= 1;
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                if bracket_depth == 0 {
                    return Err(ParserError::Css(format!(
                        "Unexpected CSS closing bracket near byte {index}. Remove the extra `]` or add a matching opening `[`."
                    )));
                }
                bracket_depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }

    if quote != 0 {
        return Err(ParserError::Css(format!(
            "Unterminated CSS string literal. Close the `{}` quote before building.",
            char::from(quote)
        )));
    }
    if brace_depth != 0 {
        return Err(ParserError::Css(
            "Unterminated CSS rule block. Add the missing `}` before building.".to_string(),
        ));
    }
    if paren_depth != 0 {
        return Err(ParserError::Css(
            "Unterminated CSS parenthesized expression. Add the missing `)` before building."
                .to_string(),
        ));
    }
    if bracket_depth != 0 {
        return Err(ParserError::Css(
            "Unterminated CSS bracket expression. Add the missing `]` before building.".to_string(),
        ));
    }

    Ok((tokens, definitions, comments))
}

fn scan_var_call(source: &str, start: usize, tokens: &mut HashSet<String>) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut index = start + 4;
    let mut depth = 1usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote: u8 = 0;
    let mut pending_tokens: Vec<&str> = Vec::new();

    while index < bytes.len() {
        if quote != 0 {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == quote {
                quote = 0;
            }
            index += 1;
            continue;
        }

        match bytes[index] {
            b'"' | b'\'' => {
                quote = bytes[index];
                index += 1;
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                let Some(offset) = source[index + 2..].find("*/") else {
                    return Err(ParserError::Css(format!(
                        "Unterminated CSS block comment inside var() near byte {index}. Close the comment with `*/` before building."
                    )));
                };
                index += 2 + offset + 2;
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
                if depth == 0 {
                    if brace_depth != 0 {
                        return Err(ParserError::Css(format!(
                            "Unterminated CSS brace expression inside var() near byte {start}. Add the missing `}}` before building."
                        )));
                    }
                    if bracket_depth != 0 {
                        return Err(ParserError::Css(format!(
                            "Unterminated CSS bracket expression inside var() near byte {start}. Add the missing `]` before building."
                        )));
                    }
                    for token in pending_tokens {
                        tokens.insert(token.to_string());
                    }
                    return Ok(index);
                }
            }
            b'{' => {
                brace_depth += 1;
                index += 1;
            }
            b'}' => {
                if brace_depth == 0 {
                    return Err(ParserError::Css(format!(
                        "Unterminated CSS var() call near byte {start}. Add the missing `)` before the closing `}}`."
                    )));
                }
                brace_depth -= 1;
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                if bracket_depth == 0 {
                    return Err(ParserError::Css(format!(
                        "Unexpected CSS closing bracket inside var() near byte {index}. Remove the extra `]` or add a matching opening `[`."
                    )));
                }
                bracket_depth -= 1;
                index += 1;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                if let Some((name, end)) = parse_custom_property_name(source, index) {
                    pending_tokens.push(name);
                    index = end;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    if quote != 0 {
        return Err(ParserError::Css(format!(
            "Unterminated CSS string literal inside var() near byte {start}. Close the `{}` quote before building.",
            char::from(quote)
        )));
    }

    Err(ParserError::Css(format!(
        "Unterminated CSS var() call near byte {start}. Add the missing `)` before building."
    )))
}

fn parse_custom_property_name(source: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start..start + 2) != Some(b"--") {
        return None;
    }

    let mut end = start + 2;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
    {
        end += 1;
    }

    (end > start + 2).then(|| (&source[start + 2..end], end))
}

fn is_custom_property_definition(source: &str, name_end: usize) -> bool {
    let bytes = source.as_bytes();
    let mut cursor = name_end;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b':') {
        return false;
    }

    let mut before = name_end.saturating_sub(1);
    while before > 0 && bytes[before - 1].is_ascii_whitespace() {
        before -= 1;
    }
    while before > 0
        && (bytes[before - 1].is_ascii_alphanumeric() || matches!(bytes[before - 1], b'_' | b'-'))
    {
        before -= 1;
    }
    while before > 0 && bytes[before - 1].is_ascii_whitespace() {
        before -= 1;
    }

    before == 0 || matches!(bytes[before - 1], b'{' | b';')
}

fn removable_ranges(comments: &[CssComment]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(comments.len());
    for comment in comments {
        if !comment.preserve {
            ranges.push((comment.start_byte, comment.end_byte));
        }
    }
    ranges
}

impl Default for CssParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_var() {
        let mut parser = CssParser::new();
        let tokens = parser
            .extract_tokens(".btn { color: var(--colorPrimary); }")
            .expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["colorPrimary".to_string()]));
    }

    #[test]
    fn test_extract_multiple_vars() {
        let css = r#"
            .btn {
                color: var(--textColor);
                background: var(--bgColor);
                border-radius: var(--radius);
            }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from([
                "textColor".to_string(),
                "bgColor".to_string(),
                "radius".to_string(),
            ])
        );
    }

    #[test]
    fn test_extract_nested_fallback() {
        let css = ".x { color: var(--primary, var(--fallback)); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from(["primary".to_string(), "fallback".to_string()])
        );
    }

    #[test]
    fn test_extract_deeply_nested_fallbacks() {
        let css = ".x { color: var(--a, var(--b, var(--c))); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn test_literal_fallback_ignored() {
        let css = ".x { font-size: var(--size, 16px); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["size".to_string()]));
    }

    #[test]
    fn test_var_fallback_block_comment_ignored() {
        let css = ".x { color: var(--primary, /* --debug-only */ red); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["primary".to_string()]));
    }

    #[test]
    fn test_var_fallback_line_comment_ignored() {
        let css = ".x { color: var(--primary,\n // --debug-only\n red); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["primary".to_string()]));
    }

    #[test]
    fn test_exclude_local_definitions() {
        let css = r#"
            :root { --bar: 12px; }
            .x { width: var(--bar); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert!(
            tokens.is_empty(),
            "Locally defined --bar should be excluded: {tokens:?}"
        );
    }

    #[test]
    fn test_exclude_definitions_keep_unrelated_usages() {
        let css = r#"
            :host { --local: 5px; }
            .x { color: var(--external); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["external".to_string()]));
    }

    #[test]
    fn test_empty_css() {
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens("").expect("extract_tokens failed");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_no_var_calls() {
        let css = "body { margin: 0; color: red; }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_deduplicate_same_var() {
        let css = r#"
            .a { color: var(--shared); }
            .b { background: var(--shared); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["shared".to_string()]));
    }

    #[test]
    fn test_unterminated_var_call_returns_error() {
        let css = ".bad { color: var(--dangling; } .ok { color: var(--valid); }";
        let mut parser = CssParser::new();
        let err = parser
            .extract_tokens(css)
            .expect_err("unterminated var() should fail");
        assert!(matches!(err, ParserError::Css(message) if
            message.contains("Unterminated CSS var() call")
        ));
    }

    #[test]
    fn test_unmatched_var_fallback_delimiter_returns_error() {
        let css = ".bad { color: var(--accent, [red); }";
        let mut parser = CssParser::new();
        let err = parser
            .extract_tokens(css)
            .expect_err("unmatched var() fallback delimiter should fail");
        assert!(matches!(err, ParserError::Css(message) if
            message.contains("Unterminated CSS bracket expression inside var()")
        ));
    }

    #[test]
    fn test_balanced_var_fallback_delimiters_extract_tokens() {
        let css = ".ok { color: var(--accent, color(display-p3 1 0 0 / 1)); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["accent".to_string()]));
    }

    #[test]
    fn test_unterminated_comment_returns_error() {
        let css = ".ok { color: var(--valid); } /* .bad { color: var(--ignored); }";
        let mut parser = CssParser::new();
        let err = parser
            .extract_tokens(css)
            .expect_err("unterminated comment should fail");
        assert!(matches!(err, ParserError::Css(message) if
            message.contains("Unterminated CSS block comment")
        ));
    }

    #[test]
    fn test_unterminated_string_returns_error() {
        let css = r#".bad { content: "dangling; }"#;
        let mut parser = CssParser::new();
        let err = parser
            .extract_tokens(css)
            .expect_err("unterminated string should fail");
        assert!(matches!(err, ParserError::Css(message) if
            message.contains("Unterminated CSS string literal")
        ));
    }

    #[test]
    fn test_comments_are_collected_and_stripped() {
        let mut parser = CssParser::new();
        let (_tokens, _defs, stripped) = parser
            .extract_tokens_definitions_and_strip_comments(
                "/* remove */.x{color:var(--a)}/*! keep */",
                LegalComments::Inline,
            )
            .expect("scan failed");
        assert_eq!(stripped, ".x{color:var(--a)}/*! keep */");
    }
}
