// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared comment classification and stripping helpers.

use crate::LegalComments;
use std::borrow::Cow;

pub(crate) fn is_css_comment_node(kind: &str) -> bool {
    matches!(kind, "comment" | "js_comment")
}

pub(crate) fn should_preserve_css_comment(comment: &str, legal_comments: LegalComments) -> bool {
    legal_comments == LegalComments::Inline && is_legal_css_comment(comment)
}

pub(crate) fn is_legal_css_comment(comment: &str) -> bool {
    let trimmed = comment.trim_start();
    trimmed.starts_with("//!")
        || trimmed.starts_with("/*!")
        || comment.contains("@license")
        || comment.contains("@preserve")
}

pub(crate) fn is_css_line_comment_start(input: &str, index: usize) -> bool {
    let bytes = input.as_bytes();
    if index + 1 >= bytes.len() || bytes[index] != b'/' || bytes[index + 1] != b'/' {
        return false;
    }
    index == 0 || bytes[index - 1].is_ascii_whitespace() || matches!(bytes[index - 1], b'{' | b';')
}

pub(crate) fn find_css_line_comment_end(input: &str, start: usize) -> usize {
    input[start..]
        .find('\n')
        .map_or(input.len(), |offset| start + offset)
}

pub(crate) fn strip_ranges<'a>(source: &'a str, ranges: &mut [(usize, usize)]) -> Cow<'a, str> {
    if ranges.is_empty() {
        return Cow::Borrowed(source);
    }

    ranges.sort_unstable_by_key(|(start, _)| *start);
    let mut stripped = String::with_capacity(source.len());
    let mut last_end = 0usize;

    for &(start, end) in ranges.iter() {
        if start < last_end {
            if end > last_end {
                last_end = end;
            }
            continue;
        }
        stripped.push_str(&source[last_end..start]);
        last_end = end;
    }

    stripped.push_str(&source[last_end..]);
    Cow::Owned(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ranges_removes_overlapping_tail() {
        let mut ranges = [(2, 5), (4, 8)];
        let stripped = strip_ranges("0123456789", &mut ranges);

        assert_eq!(stripped.as_ref(), "0189");
    }

    #[test]
    fn strip_ranges_handles_unsorted_overlaps() {
        let mut ranges = [(7, 9), (2, 5), (4, 8)];
        let stripped = strip_ranges("0123456789", &mut ranges);

        assert_eq!(stripped.as_ref(), "019");
    }
}
