use crate::Result;
use webui_protocol::WebUIFragment;

/// Parser for handlebars-style template syntax.
pub struct HandlebarsParser;

impl HandlebarsParser {
    /// Create a new handlebars parser.
    pub fn new() -> Self {
        Self
    }

    /// Simplified parse method (no nested brace support)
    pub fn parse(&self, text: &str) -> Result<Vec<WebUIFragment>> {
        let mut fragments = Vec::new();
        let mut pos = 0;
        while let Some(start) = text[pos..].find("{{") {
            let start_idx = pos + start;
            if start_idx > pos {
                // Add preceding raw text.
                fragments.push(WebUIFragment::raw(text[pos..start_idx].to_string()));
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
                fragments.push(WebUIFragment::signal(var_name, is_raw));
                pos = var_end + close_delim.len();
            } else {
                // No closing delimiter: treat the rest as raw text.
                fragments.push(WebUIFragment::raw(text[start_idx..].to_string()));
                pos = text.len();
            }
        }
        if pos < text.len() {
            fragments.push(WebUIFragment::raw(text[pos..].to_string()));
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
}
