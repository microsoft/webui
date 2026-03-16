// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::Result;
use webui_protocol::WebUIFragment;

/// Parser for handlebars-style template syntax.
pub struct HandlebarsParser;

impl HandlebarsParser {
    /// Create a new handlebars parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse handlebars expressions from text, handling edge cases with
    /// triple/double brace disambiguation.
    ///
    /// Rules for consecutive opening braces (N) without matching `}}}`:
    /// - N >= 5: first (N-2) braces are raw, remaining `{{…}}` is a valid double
    /// - N == 3 or 4: entire sequence through `}}` is treated as raw text
    pub fn parse(&self, text: &str) -> Result<Vec<WebUIFragment>> {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut fragments = Vec::new();
        let mut raw_buf = String::new();
        let mut pos = 0;

        while pos < len {
            let remaining = &text[pos..];
            let Some(offset) = remaining.find("{{") else {
                raw_buf.push_str(&text[pos..]);
                break;
            };

            let start = pos + offset;
            if start > pos {
                raw_buf.push_str(&text[pos..start]);
            }

            // Count consecutive opening braces
            let mut brace_count = 0;
            let mut i = start;
            while i < len && bytes[i] == b'{' {
                brace_count += 1;
                i += 1;
            }

            if brace_count >= 3 {
                // Try triple brace: look for }}}
                if let Some(end_offset) = text[i..].find("}}}") {
                    let var_end = i + end_offset;
                    let var_name = text[i..var_end].trim();
                    if !var_name.is_empty() {
                        // Extra braces beyond 3 become raw prefix
                        for _ in 0..(brace_count - 3) {
                            raw_buf.push('{');
                        }
                        if !raw_buf.is_empty() {
                            fragments.push(WebUIFragment::raw(std::mem::take(&mut raw_buf)));
                        }
                        fragments.push(WebUIFragment::signal(var_name.to_string(), true));
                        pos = var_end + 3;
                        continue;
                    }
                }

                // }}} not found. If N >= 5 we can extract a valid {{…}} from the tail.
                if brace_count >= 5 {
                    if let Some(end_offset) = text[i..].find("}}") {
                        let var_end = i + end_offset;
                        let var_name = text[i..var_end].trim();
                        if !var_name.is_empty() {
                            for _ in 0..(brace_count - 2) {
                                raw_buf.push('{');
                            }
                            if !raw_buf.is_empty() {
                                fragments.push(WebUIFragment::raw(std::mem::take(&mut raw_buf)));
                            }
                            fragments.push(WebUIFragment::signal(var_name.to_string(), false));
                            pos = var_end + 2;
                            continue;
                        }
                    }
                }

                // Failed triple (N < 5): consume everything through }} as raw
                if let Some(end_offset) = text[i..].find("}}") {
                    let raw_end = i + end_offset + 2;
                    raw_buf.push_str(&text[start..raw_end]);
                    pos = raw_end;
                } else {
                    raw_buf.push_str(&text[start..]);
                    pos = len;
                }
                continue;
            }

            // Double brace: look for }}
            if let Some(end_offset) = text[i..].find("}}") {
                let var_end = i + end_offset;
                let var_name = text[i..var_end].trim();
                if !var_name.is_empty() {
                    if !raw_buf.is_empty() {
                        fragments.push(WebUIFragment::raw(std::mem::take(&mut raw_buf)));
                    }
                    fragments.push(WebUIFragment::signal(var_name.to_string(), false));
                    pos = var_end + 2;
                    continue;
                }
            }

            // No valid closing — rest is raw
            raw_buf.push_str(&text[start..]);
            pos = len;
        }

        if !raw_buf.is_empty() {
            fragments.push(WebUIFragment::raw(raw_buf));
        }

        Ok(fragments)
    }
}

impl Default for HandlebarsParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::web_ui_fragment::Fragment;

    #[test]
    fn test_parse_plain_text() {
        let parser = HandlebarsParser::new();
        let result = parser
            .parse("Hello, World!")
            .expect("Failed to parse plain text");

        assert_eq!(result.len(), 1);
        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, World!"),
            _ => panic!("Expected Raw fragment"),
        }
    }

    #[test]
    fn test_parse_double_brace() {
        let parser = HandlebarsParser::new();
        let result = parser
            .parse("Hello, {{name}}!")
            .expect("Failed to parse double brace syntax");

        assert_eq!(result.len(), 3);

        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, "),
            _ => panic!("Expected Raw fragment"),
        }

        match result[1].fragment.as_ref() {
            Some(Fragment::Signal(signal)) => {
                assert_eq!(signal.value, "name");
                assert!(!signal.raw);
            }
            _ => panic!("Expected Signal fragment"),
        }

        match result[2].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "!"),
            _ => panic!("Expected Raw fragment"),
        }
    }

    #[test]
    fn test_parse_triple_brace() {
        let parser = HandlebarsParser::new();
        let result = parser
            .parse("Content: {{{html_content}}}")
            .expect("Failed to parse triple brace syntax");

        assert_eq!(result.len(), 2);

        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Content: "),
            _ => panic!("Expected Raw fragment"),
        }

        match result[1].fragment.as_ref() {
            Some(Fragment::Signal(signal)) => {
                assert_eq!(signal.value, "html_content");
                assert!(signal.raw);
            }
            _ => panic!("Expected Signal fragment"),
        }
    }

    #[test]
    fn test_mixed_braces() {
        let parser = HandlebarsParser::new();
        let result = parser
            .parse("Hello, {{name}}! {{{html_content}}}")
            .expect("Failed to parse mixed brace syntax");

        assert_eq!(result.len(), 4);

        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, "),
            _ => panic!("Expected Raw fragment"),
        }

        match result[1].fragment.as_ref() {
            Some(Fragment::Signal(signal)) => {
                assert_eq!(signal.value, "name");
                assert!(!signal.raw);
            }
            _ => panic!("Expected Signal fragment"),
        }

        match result[2].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "! "),
            _ => panic!("Expected Raw fragment"),
        }

        match result[3].fragment.as_ref() {
            Some(Fragment::Signal(signal)) => {
                assert_eq!(signal.value, "html_content");
                assert!(signal.raw);
            }
            _ => panic!("Expected Signal fragment"),
        }
    }

    #[test]
    fn test_invalid_triple_open() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("{{{invalid}}").expect("parse failed");
        assert_eq!(result.len(), 1);
        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{invalid}}"),
            _ => panic!("Expected Raw fragment"),
        }
    }

    #[test]
    fn test_four_open_braces() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("{{{{invalid}}").expect("parse failed");
        assert_eq!(result.len(), 1);
        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{{invalid}}"),
            _ => panic!("Expected Raw fragment"),
        }
    }

    #[test]
    fn test_five_braces_with_valid_double() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("{{{{{invalid}}").expect("parse failed");
        assert_eq!(result.len(), 2);
        match result[0].fragment.as_ref() {
            Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{"),
            _ => panic!("Expected Raw fragment for prefix"),
        }
        match result[1].fragment.as_ref() {
            Some(Fragment::Signal(s)) => {
                assert_eq!(s.value, "invalid");
                assert!(!s.raw);
            }
            _ => panic!("Expected Signal fragment"),
        }
    }
}
