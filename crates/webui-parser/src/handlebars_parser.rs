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
#[path = "handlebars_parser_tests.rs"]
mod handlebars_parser_tests;
