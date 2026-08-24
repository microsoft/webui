// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::html_parser::{
    find_comment_close, find_raw_text_end, find_tag_close, is_raw_text_element, is_void_element,
    parse_tag,
};
use std::ops::Range;

/// Opening-tag scan result without allocating a collection of offsets.
pub(super) struct NamedTagMatches {
    pub(super) first: Option<usize>,
    pub(super) count: usize,
}

/// Scan named opening tags, skipping comments and raw-text bodies.
///
/// Unterminated tags use a name-only fallback so callers can diagnose them.
pub(super) fn scan_named_open_tags(
    source: &str,
    range: Range<usize>,
    target: &str,
) -> NamedTagMatches {
    let mut first = None;
    let mut count = 0usize;
    let mut cursor = range.start;

    while cursor < range.end {
        let Some(relative) = source[cursor..range.end].find('<') else {
            break;
        };
        let start = cursor + relative;
        let remaining = &source[start..range.end];
        if remaining.starts_with("<!--") {
            cursor = find_comment_close(remaining).map_or(range.end, |length| start + length);
            continue;
        }

        let Some(tag) = parse_tag(remaining) else {
            if read_opening_tag_name(source, start, range.end)
                .is_some_and(|name| name.eq_ignore_ascii_case(target))
            {
                first.get_or_insert(start);
                count += 1;
            }
            cursor = find_tag_end(source, start, range.end).unwrap_or(start + 1);
            continue;
        };

        if !tag.closing {
            if tag.name.eq_ignore_ascii_case(target) {
                first.get_or_insert(start);
                count += 1;
            }
            if is_raw_text_element(tag.name) {
                cursor = start + find_raw_text_end(remaining, tag.name, tag.close + 1);
                continue;
            }
        }
        cursor = start + tag.close + 1;
    }

    NamedTagMatches { first, count }
}

/// Find a matching close while treating raw-text bodies as opaque.
///
/// Returns `(close_start, close_end)` and otherwise follows the generic
/// matcher's depth rules.
pub(super) fn find_matching_end_skip_raw_text(
    input: &str,
    tag_name: &str,
    content_start: usize,
) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut index = content_start;

    while index < input.len() {
        let relative = input[index..].find('<')?;
        index += relative;

        if input[index..].starts_with("<!--") {
            index += find_comment_close(&input[index..]).unwrap_or(input.len() - index);
            continue;
        }

        let Some(tag) = parse_tag(&input[index..]) else {
            index += 1;
            continue;
        };

        if tag.closing {
            if tag.name.eq_ignore_ascii_case(tag_name) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((index, index + tag.close + 1));
                }
            }
            index += tag.close + 1;
            continue;
        }

        if tag.name.eq_ignore_ascii_case(tag_name)
            && !tag.self_closing
            && !is_void_element(tag.name)
        {
            depth += 1;
        }

        if is_raw_text_element(tag.name) {
            index += find_raw_text_end(&input[index..], tag.name, tag.close + 1);
            continue;
        }

        index += tag.close + 1;
    }

    None
}

/// Find the end after `>` for one tag without scanning beyond `range_end`.
#[inline]
pub(super) fn find_tag_end(source: &str, start: usize, range_end: usize) -> Option<usize> {
    find_tag_close(&source[start..range_end]).map(|close| start + close + 1)
}

/// Read an opening tag name without requiring a terminating `>`.
pub(super) fn read_opening_tag_name(source: &str, start: usize, range_end: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut cursor = start + 1;
    if matches!(bytes.get(cursor), Some(b'/') | Some(b'!') | Some(b'?')) {
        return None;
    }
    while cursor < range_end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    let name_start = cursor;
    while cursor < range_end
        && !bytes[cursor].is_ascii_whitespace()
        && bytes[cursor] != b'>'
        && bytes[cursor] != b'/'
    {
        cursor += 1;
    }
    (cursor != name_start).then_some(&source[name_start..cursor])
}
