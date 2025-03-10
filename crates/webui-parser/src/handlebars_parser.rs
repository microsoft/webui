use crate::Result;
use webui_protocol::{WebUIStream, WebUIStreamRaw, WebUIStreamSignal};

/// Parser for handlebars-style template syntax.
pub struct HandlebarsParser;

impl HandlebarsParser {
    /// Create a new handlebars parser.
    pub fn new() -> Self {
        Self
    }

    /// Simplified parse method (no nested brace support)
    pub fn parse(&self, text: &str) -> Result<Vec<WebUIStream>> {
        let mut streams = Vec::new();
        let mut pos = 0;
        while let Some(start) = text[pos..].find("{{") {
            let start_idx = pos + start;
            if start_idx > pos {
                // Add preceding raw text.
                streams.push(WebUIStream::Raw(WebUIStreamRaw {
                    value: text[pos..start_idx].to_string(),
                }));
            }
            // Determine if it's triple or double brace.
            let (is_raw, open_delim, close_delim) = if text[start_idx..].starts_with("{{{") {
                (true, "{{{", "}}}")
            } else {
                (false, "{{", "}}")
            };
            // Look for the closing delimiter.
            if let Some(end) = text[start_idx + open_delim.len()..].find(close_delim) {
                let var_start = start_idx + open_delim.len();
                let var_end = var_start + end;
                let var_name = text[var_start..var_end].trim().to_string();
                streams.push(WebUIStream::Signal(WebUIStreamSignal {
                    value: var_name,
                    raw: is_raw,
                }));
                pos = var_end + close_delim.len();
            } else {
                // No closing delimiter: treat the rest as raw text.
                streams.push(WebUIStream::Raw(WebUIStreamRaw {
                    value: text[start_idx..].to_string(),
                }));
                pos = text.len();
            }
        }
        if pos < text.len() {
            streams.push(WebUIStream::Raw(WebUIStreamRaw {
                value: text[pos..].to_string(),
            }));
        }
        Ok(streams)
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

    #[test]
    fn test_parse_plain_text() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("Hello, World!").unwrap();

        assert_eq!(result.len(), 1);
        match &result[0] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "Hello, World!"),
            _ => panic!("Expected Raw stream"),
        }
    }

    #[test]
    fn test_parse_double_brace() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("Hello, {{name}}!").unwrap();

        assert_eq!(result.len(), 3);

        match &result[0] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "Hello, "),
            _ => panic!("Expected Raw stream"),
        }

        match &result[1] {
            WebUIStream::Signal(signal) => {
                assert_eq!(signal.value, "name");
                assert!(!signal.raw);
            }
            _ => panic!("Expected Signal stream"),
        }

        match &result[2] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "!"),
            _ => panic!("Expected Raw stream"),
        }
    }

    #[test]
    fn test_parse_triple_brace() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("Content: {{{html_content}}}").unwrap();

        assert_eq!(result.len(), 2);

        match &result[0] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "Content: "),
            _ => panic!("Expected Raw stream"),
        }

        match &result[1] {
            WebUIStream::Signal(signal) => {
                assert_eq!(signal.value, "html_content");
                assert!(signal.raw);
            }
            _ => panic!("Expected Signal stream"),
        }
    }

    #[test]
    fn test_mixed_braces() {
        let parser = HandlebarsParser::new();
        let result = parser.parse("Hello, {{name}}! {{{html_content}}}").unwrap();

        assert_eq!(result.len(), 4);

        match &result[0] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "Hello, "),
            _ => panic!("Expected Raw stream"),
        }

        match &result[1] {
            WebUIStream::Signal(signal) => {
                assert_eq!(signal.value, "name");
                assert!(!signal.raw);
            }
            _ => panic!("Expected Signal stream"),
        }

        match &result[2] {
            WebUIStream::Raw(raw) => assert_eq!(raw.value, "! "),
            _ => panic!("Expected Raw stream"),
        }

        match &result[3] {
            WebUIStream::Signal(signal) => {
                assert_eq!(signal.value, "html_content");
                assert!(signal.raw);
            }
            _ => panic!("Expected Signal stream"),
        }
    }
}
