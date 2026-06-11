// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Zero-copy HTML scanner and mini DOM-like walker.
//!
//! This module intentionally does not build a DOM tree. It exposes borrowed
//! element/comment/text ranges so callers get a clean traversal API without
//! giving up the single-pass scanner performance.
//! It is not a browser-conformance parser; it is a lenient scanner for WebUI's
//! build-time template subset.

use std::ops::Range;

/// HTML attribute parsed from an opening tag.
#[derive(Debug, Clone)]
pub(crate) struct Attr<'a> {
    pub name: &'a str,
    pub value: Option<&'a str>,
    pub raw: &'a str,
    pub raw_range: Range<usize>,
}

/// Iterator over attributes in an opening tag.
#[derive(Debug, Clone)]
pub(crate) struct Attrs<'a> {
    input: &'a str,
    cursor: usize,
    attrs_end: usize,
}

/// HTML tag parsed from the start of a source slice.
#[derive(Debug)]
pub(crate) struct Tag<'a> {
    source: &'a str,
    pub name: &'a str,
    attrs_start: usize,
    attrs_end: usize,
    pub close: usize,
    pub self_closing: bool,
    pub closing: bool,
}

impl<'a> Tag<'a> {
    #[inline]
    pub(crate) fn attrs(&self) -> Attrs<'a> {
        Attrs {
            input: self.source,
            cursor: self.attrs_start,
            attrs_end: self.attrs_end,
        }
    }

    #[inline]
    pub(crate) fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs()
            .find(|attr| attr.name == name)
            .and_then(|attr| attr.value)
    }

    #[inline]
    pub(crate) fn has_attr(&self, name: &str) -> bool {
        self.attrs().any(|attr| attr.name == name)
    }
}

/// A parsed element event with source ranges for its opening tag, body, and
/// optional closing tag.
#[derive(Debug)]
pub(crate) struct Element<'a> {
    pub(crate) source: &'a str,
    pub(crate) start: usize,
    pub(crate) tag: Tag<'a>,
    pub(crate) content_start: usize,
    pub(crate) content_end: usize,
    pub(crate) close_end: usize,
}

impl<'a> Element<'a> {
    #[inline]
    pub(crate) fn source(&self) -> &'a str {
        self.source
    }

    #[inline]
    pub(crate) fn name(&self) -> &'a str {
        self.tag.name
    }

    #[inline]
    pub(crate) fn attrs(&self) -> Attrs<'a> {
        self.tag.attrs()
    }

    #[inline]
    pub(crate) fn attr(&self, name: &str) -> Option<&'a str> {
        self.tag.attr(name)
    }

    #[inline]
    pub(crate) fn has_attr(&self, name: &str) -> bool {
        self.tag.has_attr(name)
    }

    #[inline]
    pub(crate) fn opening(&self) -> &'a str {
        &self.source[self.start..self.content_start]
    }

    #[inline]
    pub(crate) fn self_closing(&self) -> bool {
        self.tag.self_closing
    }

    #[inline]
    pub(crate) fn inner(&self) -> Range<usize> {
        self.content_start..self.content_end
    }

    #[inline]
    pub(crate) fn content_end(&self) -> usize {
        self.content_end
    }

    #[inline]
    pub(crate) fn close_end(&self) -> usize {
        self.close_end
    }

    #[inline]
    pub(crate) fn is_void(&self) -> bool {
        is_void_element(self.tag.name)
    }

    #[inline]
    pub(crate) fn children(&self) -> Walker<'a> {
        Walker::new_range(self.source, self.content_start, self.content_end)
    }
}

/// Streaming HTML event.
#[derive(Debug)]
pub(crate) enum Event<'a> {
    Text(&'a str),
    Comment(Range<usize>),
    Declaration(Range<usize>),
    ClosingTag(Range<usize>),
    Element(Element<'a>),
}

/// Iterative scanner over a source range.
#[derive(Debug)]
pub(crate) struct Walker<'a> {
    source: &'a str,
    cursor: usize,
    end: usize,
}

impl<'a> Walker<'a> {
    #[cfg(test)]
    pub(crate) fn new(source: &'a str) -> Self {
        Self::new_range(source, 0, source.len())
    }

    #[inline]
    pub(crate) fn new_range(source: &'a str, start: usize, end: usize) -> Self {
        Self {
            source,
            cursor: start,
            end,
        }
    }
}

impl<'a> Iterator for Walker<'a> {
    type Item = Event<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.end {
            let start = self.cursor;
            let remaining = &self.source[start..self.end];

            if remaining.starts_with("<!--") {
                let close = find_comment_close(remaining).unwrap_or(remaining.len());
                self.cursor += close;
                return Some(Event::Comment(start..start + close));
            }

            if remaining.starts_with("<!") {
                let close = find_declaration_close(remaining).unwrap_or(remaining.len());
                self.cursor += close;
                return Some(Event::Declaration(start..start + close));
            }

            if remaining.starts_with('<') {
                if let Some(tag) = parse_tag(remaining) {
                    if tag.closing {
                        self.cursor += tag.close + 1;
                        return Some(Event::ClosingTag(start..start + tag.close + 1));
                    }

                    let content_start = start + tag.close + 1;
                    let (content_end, close_end) = if tag.self_closing || is_void_element(tag.name)
                    {
                        (content_start, content_start)
                    } else if let Some((close_start, close_end)) =
                        find_matching_end(remaining, tag.name, tag.close + 1)
                    {
                        (start + close_start, start + close_end)
                    } else {
                        (self.end, self.end)
                    };

                    self.cursor = close_end.max(content_start);
                    return Some(Event::Element(Element {
                        source: self.source,
                        start,
                        tag,
                        content_start,
                        content_end,
                        close_end,
                    }));
                }
            }

            let next = remaining.find('<').unwrap_or(remaining.len());
            if next == 0 {
                let ch = remaining.chars().next()?;
                self.cursor += ch.len_utf8();
                continue;
            }

            self.cursor += next;
            return Some(Event::Text(&remaining[..next]));
        }

        None
    }
}

/// Return the byte index of the `>` that closes an HTML tag, ignoring quoted
/// attribute values. Returns `None` if the tag is unterminated.
#[inline]
pub(crate) fn find_tag_close(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut quote: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else {
            match c {
                b'>' => return Some(i),
                b'"' | b'\'' => quote = c,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Parse the first tag in `input`.
#[inline]
pub(crate) fn parse_tag(input: &str) -> Option<Tag<'_>> {
    if !input.starts_with('<') || input.starts_with("<!--") || input.starts_with("<!") {
        return None;
    }

    let close = find_tag_close(input)?;
    let bytes = input.as_bytes();
    let mut cursor = 1usize;
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }

    while cursor < close && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    let name_start = cursor;
    while cursor < close
        && !bytes[cursor].is_ascii_whitespace()
        && bytes[cursor] != b'/'
        && bytes[cursor] != b'>'
    {
        cursor += 1;
    }
    if name_start == cursor {
        return None;
    }
    let name = &input[name_start..cursor];

    let self_closing = !closing && tag_is_self_closing(input, close);
    let attrs_end = if self_closing {
        input[..close].trim_end().len().saturating_sub(1)
    } else {
        close
    };
    let (attrs_start, attrs_end) = if closing {
        (cursor, cursor)
    } else {
        (cursor, attrs_end)
    };

    Some(Tag {
        source: input,
        name,
        attrs_start,
        attrs_end,
        close,
        self_closing,
        closing,
    })
}

/// Return the name of the first opening or self-closing tag in `input`.
#[inline]
pub(crate) fn opening_tag_name(input: &str) -> Option<&str> {
    let tag = parse_tag(input)?;
    (!tag.closing).then_some(tag.name)
}

/// Return the content and closing-tag byte ranges for a `<style>` element that
/// starts at the beginning of `input`.
#[inline]
pub(crate) fn style_element_bounds(input: &str) -> Option<(usize, usize, usize)> {
    let tag = parse_tag(input)?;
    if tag.closing || !tag.name.eq_ignore_ascii_case("style") {
        return None;
    }

    let content_start = tag.close + 1;
    let (close_start, close_end) = find_matching_end(input, tag.name, content_start)?;
    Some((content_start, close_start, close_end))
}

/// Find the matching closing tag for an element body that starts at
/// `content_start`. Returns `(close_start, close_end)`.
#[inline]
pub(crate) fn find_matching_end(
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

        if tag.name.eq_ignore_ascii_case(tag_name) {
            if tag.closing {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((index, index + tag.close + 1));
                }
            } else if !tag.self_closing && !is_void_element(tag.name) {
                depth += 1;
            }
        }

        index += tag.close + 1;
    }

    None
}

/// Return the end byte after an HTML comment starting at the beginning of
/// `input`.
#[inline]
pub(crate) fn find_comment_close(input: &str) -> Option<usize> {
    input.find("-->").map(|close| close + 3)
}

/// Return the end byte after a doctype/declaration starting at the beginning of
/// `input`.
#[inline]
pub(crate) fn find_declaration_close(input: &str) -> Option<usize> {
    find_tag_close(input).map(|close| close + 1)
}

/// Return true for HTML void elements that do not require end tags.
///
/// Tag names are compared ASCII-case-insensitively, matching the HTML
/// specification and the rest of the scanner (e.g. [`find_matching_end`] and
/// [`style_element_bounds`]), so `<BR>` and `<img>` are both recognized as
/// void. Without this, an uppercase void tag would be treated as an open
/// element and swallow the remainder of the document as its body.
#[inline]
pub(crate) fn is_void_element(tag_name: &str) -> bool {
    let bytes = tag_name.as_bytes();
    // The longest void tag name ("source") is six bytes; anything longer or
    // empty cannot be a void element, so skip the case-folding work entirely.
    if bytes.is_empty() || bytes.len() > 6 {
        return false;
    }
    let mut buf = [0u8; 6];
    let name = &mut buf[..bytes.len()];
    name.copy_from_slice(bytes);
    name.make_ascii_lowercase();
    matches!(
        &*name,
        b"area"
            | b"base"
            | b"br"
            | b"col"
            | b"embed"
            | b"hr"
            | b"img"
            | b"input"
            | b"link"
            | b"meta"
            | b"param"
            | b"source"
            | b"track"
            | b"wbr"
    )
}

#[inline]
fn tag_is_self_closing(input: &str, close: usize) -> bool {
    input[..close].trim_end().ends_with('/')
}

impl<'a> Iterator for Attrs<'a> {
    type Item = Attr<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.input.as_bytes();

        loop {
            while self.cursor < self.attrs_end && bytes[self.cursor].is_ascii_whitespace() {
                self.cursor += 1;
            }
            if self.cursor >= self.attrs_end {
                return None;
            }

            let raw_start = self.cursor;
            while self.cursor < self.attrs_end
                && !bytes[self.cursor].is_ascii_whitespace()
                && bytes[self.cursor] != b'='
                && bytes[self.cursor] != b'/'
            {
                self.cursor += 1;
            }
            if raw_start == self.cursor {
                self.cursor += 1;
                continue;
            }

            let name = &self.input[raw_start..self.cursor];
            while self.cursor < self.attrs_end && bytes[self.cursor].is_ascii_whitespace() {
                self.cursor += 1;
            }

            let mut value = None;
            if self.cursor < self.attrs_end && bytes[self.cursor] == b'=' {
                self.cursor += 1;
                while self.cursor < self.attrs_end && bytes[self.cursor].is_ascii_whitespace() {
                    self.cursor += 1;
                }
                if self.cursor < self.attrs_end
                    && (bytes[self.cursor] == b'"' || bytes[self.cursor] == b'\'')
                {
                    let quote = bytes[self.cursor];
                    self.cursor += 1;
                    let value_start = self.cursor;
                    while self.cursor < self.attrs_end && bytes[self.cursor] != quote {
                        self.cursor += 1;
                    }
                    value = Some(&self.input[value_start..self.cursor.min(self.attrs_end)]);
                    if self.cursor < self.attrs_end {
                        self.cursor += 1;
                    }
                } else {
                    let value_start = self.cursor;
                    while self.cursor < self.attrs_end && !bytes[self.cursor].is_ascii_whitespace()
                    {
                        self.cursor += 1;
                    }
                    value = Some(&self.input[value_start..self.cursor]);
                }
            }

            return Some(Attr {
                name,
                value,
                raw: self.input[raw_start..self.cursor].trim(),
                raw_range: raw_start..self.cursor,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_tag_close_simple() {
        assert_eq!(find_tag_close("<br>"), Some(3));
        assert_eq!(find_tag_close("<br/>"), Some(4));
        assert_eq!(find_tag_close("<template>"), Some(9));
        assert_eq!(find_tag_close("</div>"), Some(5));
        assert_eq!(find_tag_close("</style>"), Some(7));
        assert_eq!(find_tag_close("</outlet></main>"), Some(8));
        assert_eq!(find_tag_close("<outlet /></main>"), Some(9));
        assert_eq!(find_tag_close("<div>{{title}}</div>"), Some(4));
        assert_eq!(
            find_tag_close(r#"<template shadowrootmode="open"><div></div></template>"#),
            Some(31)
        );
    }

    #[test]
    fn find_tag_close_skips_quoted_gt() {
        assert_eq!(find_tag_close(r#"<if condition="a > b">"#), Some(21));
        assert_eq!(find_tag_close(r#"<if condition="a >= b">"#), Some(22));
        assert_eq!(find_tag_close(r#"<a x='a>b'>"#), Some(10));
    }

    #[test]
    fn parse_tag_reads_attributes() {
        let tag = parse_tag(r#"<button class="a b" disabled data-x='1'>"#).unwrap();
        assert_eq!(tag.name, "button");
        assert_eq!(tag.close, 39);
        assert!(!tag.self_closing);
        let attrs: Vec<_> = tag.attrs().collect();
        assert_eq!(attrs.len(), 3);
        assert_eq!(attrs[0].name, "class");
        assert_eq!(attrs[0].value, Some("a b"));
        assert_eq!(attrs[0].raw_range, 8..19);
        assert_eq!(attrs[1].name, "disabled");
        assert_eq!(attrs[1].value, None);
        assert_eq!(attrs[2].name, "data-x");
        assert_eq!(attrs[2].value, Some("1"));
    }

    #[test]
    fn walker_yields_dom_like_events() {
        let mut walker = Walker::new("<div id=\"a\">hi</div><!--x-->");
        let Some(Event::Element(element)) = walker.next() else {
            panic!("expected element");
        };
        assert_eq!(element.name(), "div");
        assert_eq!(element.attr("id"), Some("a"));
        assert_eq!(element.inner(), 12..14);

        let mut children = element.children();
        assert!(matches!(children.next(), Some(Event::Text("hi"))));
        assert!(children.next().is_none());

        assert!(matches!(walker.next(), Some(Event::Comment(range)) if range == (20..28)));
        assert!(walker.next().is_none());
    }

    #[test]
    fn opening_tag_name_reads_tag_name() {
        assert_eq!(
            opening_tag_name(r#"<template data-x="a>b">"#),
            Some("template")
        );
        assert_eq!(opening_tag_name("<outlet />"), Some("outlet"));
        assert_eq!(opening_tag_name("</template>"), None);
    }

    #[test]
    fn style_element_bounds_returns_content_and_closing_tag() {
        let html = "<style>.a { color: red; }</style><div></div>";
        assert_eq!(style_element_bounds(html), Some((7, 25, 33)));
    }

    #[test]
    fn find_matching_end_handles_nested_same_tag() {
        let html = "<div><div>x</div></div><p></p>";
        assert_eq!(find_matching_end(html, "div", 5), Some((17, 23)));
    }

    #[test]
    fn find_matching_end_handles_nested_for_inside_if() {
        let html = r#"<for each="category in categories">
            <if condition="category.hasItems">
                <for each="item in category.items">
                   {{item.title}}
                </for>
            </if>
        </for>"#;
        let open = find_tag_close(html).unwrap() + 1;
        assert_eq!(find_matching_end(html, "for", open), Some((218, 224)));
    }

    #[test]
    fn is_void_element_matches_case_insensitively() {
        for name in [
            "br", "BR", "Img", "IMG", "input", "INPUT", "source", "SOURCE",
        ] {
            assert!(is_void_element(name), "{name} should be void");
        }
        for name in ["div", "style", "Section", "", "sourcex", "embedded"] {
            assert!(!is_void_element(name), "{name} should not be void");
        }
    }

    #[test]
    fn walker_treats_uppercase_void_element_as_void() {
        let mut walker = Walker::new("<BR><span>after</span>");
        let Some(Event::Element(br)) = walker.next() else {
            panic!("expected void element");
        };
        assert_eq!(br.name(), "BR");
        // The void element must not swallow its siblings as body content.
        assert_eq!(br.inner(), 4..4);

        let Some(Event::Element(span)) = walker.next() else {
            panic!("expected sibling element");
        };
        assert_eq!(span.name(), "span");
        assert!(walker.next().is_none());
    }
}
