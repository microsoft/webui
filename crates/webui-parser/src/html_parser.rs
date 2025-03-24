//! Directive parser for WebUI template directives.
//!
//! This module handles parsing WebUI-specific directives like <for>, <if>, etc.

use std::collections::HashMap;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIteratorMut};
use webui_protocol::{
    WebUIStream, WebUIStreamRaw, WebUIStreamFor, WebUIStreamIf,
    WebUIStreamRecords,
};
use crate::condition_parser::ConditionParser;
use crate::{ParserError, Result, CssParser, HandlebarsParser};

use tree_sitter_html::LANGUAGE;

/// Counter for generating unique stream IDs.
struct StreamIdCounter {
    /// Map of counter types to their current values.
    counters: HashMap<String, usize>,
}

impl StreamIdCounter {
    /// Create a new stream ID counter.
    fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }
    
    /// Generate a unique stream ID.
    fn next_id(&mut self, prefix: &str) -> String {
        let count = self.counters.entry(prefix.to_string()).or_insert(0);
        *count += 1;
        format!("{}-{}", prefix, count)
    }
}

/// Parser for WebUI directives.
pub struct HtmlParser {
    /// CSS parser.
    css_parser: CssParser,
    
    /// Tree-sitter parser for HTML.
    parser: Parser,
    
    /// Stream ID counter.
    id_counter: StreamIdCounter,
    
    /// Condition parser for parsing conditions in directives.
    condition_parser: ConditionParser,

    /// Handlebars parser for parsing handlebars expressions.
    handlebars_parser: HandlebarsParser,

    /// Buffer for accumulating raw content
    raw_buffer: String,

    /// Map of stream IDs to their streams
    stream_records: WebUIStreamRecords,
}

impl HtmlParser {
    /// Create a new directive parser.
    pub fn new(stream_records: WebUIStreamRecords) -> Self {
        let mut parser = Parser::new();
        parser.set_language(&LANGUAGE.into()).expect("Error loading HTML grammar");
        
        Self {
            css_parser: CssParser::new(),
            parser,
            id_counter: StreamIdCounter::new(),
            condition_parser: ConditionParser::new(),
            handlebars_parser: HandlebarsParser::new(),
            raw_buffer: String::new(),
            stream_records,
        }
    }
    
    pub fn into_stream_records(mut self) -> WebUIStreamRecords {
        std::mem::take(&mut self.stream_records)
    }

    /// Parse HTML content to generate WebUI streams.
    pub fn parse(&mut self, stream_id: &str, html_content: &str) -> Result<()> {
        // Reset sub-streams for new parse
        self.raw_buffer.clear();
        
        // Parse HTML
        let tree = self.parser.parse(html_content, None)
            .ok_or_else(|| ParserError::Html("Failed to parse HTML".to_string()))?;
        
        let mut entry_stream: Vec<WebUIStream> = Vec::new();

        // Start processing the HTML node.
        self.process_html_node(tree.root_node(), html_content, &mut entry_stream)?;
        
        self.flush_raw_buffer(&mut entry_stream);

        // Insert the entry record.
        self.stream_records.insert(stream_id.to_string(), entry_stream);

        // Return all streams including generated sub-streams
        Ok(())
    }
    
    /// Add raw content to the buffer
    fn add_raw_stream(&mut self, content: &str) {
        if !content.is_empty() {
            self.raw_buffer.push_str(content);
        }
    }
    
    /// Add a for stream, flushing raw buffer first
    fn add_for_stream(&mut self, item: String, collection: String, stream_id: String, streams: &mut Vec<WebUIStream>) {
        self.flush_raw_buffer(streams);
        streams.push(WebUIStream::For(WebUIStreamFor {
            item,
            collection,
            stream_id,
        }));
    }
    
    /// Add an if stream, flushing raw buffer first
    fn add_if_stream(&mut self, condition: webui_protocol::ConditionExpr, stream_id: String, streams: &mut Vec<WebUIStream>) {
        self.flush_raw_buffer(streams);
        streams.push(WebUIStream::If(WebUIStreamIf {
            condition,
            stream_id,
        }));
    }
    
    /// Add a non-raw stream, flushing the raw buffer first if needed
    fn add_stream(&mut self, stream: WebUIStream, streams: &mut Vec<WebUIStream>) {
        self.flush_raw_buffer(streams);
        streams.push(stream);
    }
    
    /// Flush the raw buffer into streams if not empty
    fn flush_raw_buffer(&mut self, streams: &mut Vec<WebUIStream>) {
        if !self.raw_buffer.is_empty() {
            streams.push(WebUIStream::Raw(WebUIStreamRaw {
                value: std::mem::take(&mut self.raw_buffer),
            }));
        }
    }

    /// Process an HTML node to generate WebUI streams.
    fn process_html_node(
        &mut self,
        node: Node,
        source: &str,
        streams: &mut Vec<WebUIStream>
    ) -> Result<()> {
        let mut cursor = node.walk();
        
        if node.kind() == "document" || node.kind() == "fragment" || node.kind() == "element" {
            // Process children
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    
                    // Process child node
                    self.process_child_node(child, source, streams)?;
                    
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            }
        } else {
            // Add text content as raw stream
            let content = &source[node.start_byte()..node.end_byte()];
            if !content.trim().is_empty() {
                self.add_raw_stream(content);
            }
        }
        
        Ok(())
    }
    
    /// Process a child HTML node.
    fn process_child_node(
        &mut self,
        node: Node,
        source: &str,
        streams: &mut Vec<WebUIStream>
    ) -> Result<()> {
        match node.kind() {
            "element" => {
                // Get the tag name
                let tag_name = self.get_element_tag_name(node, source)?;
                
                // Handle WebUI directives
                match tag_name.as_str() {
                    "for" => return self.process_for_directive(node, source, streams),
                    "if" => return self.process_if_directive(node, source, streams),
                    _ => {
                        // For regular HTML elements, extract the raw start/end tags
                        let start_byte = node.start_byte();
                        let end_byte = node.end_byte();
                        let full_content = &source[start_byte..end_byte];
                        
                        // Find indices of opening and closing brackets to extract tags
                        if let Some(close_bracket_pos) = full_content.find('>') {
                            // Add opening tag as raw content
                            let opening_tag = &full_content[0..=close_bracket_pos];
                            self.add_raw_stream(opening_tag);
                            
                            // Process children
                            for child in node.named_children(&mut node.walk()) {
                                if child.kind() != "start_tag" && child.kind() != "end_tag" {
                                    self.process_child_node(child, source, streams)?;
                                }
                            }
                            
                            // Find closing tag and add it
                            if let Some(last_open_pos) = full_content.rfind('<') {
                                let closing_tag = &full_content[last_open_pos..];
                                self.add_raw_stream(closing_tag);
                                self.flush_raw_buffer(streams);
                            }
                        }
                        
                        return Ok(());
                    }
                }
            },
            "style_element" => {
                // Process inline CSS
                for child in node.named_children(&mut node.walk()) {
                    if child.kind() == "raw_text" {
                        let style_content = &source[child.start_byte()..child.end_byte()];
                        let processed_css = self.css_parser.parse_inline_css(style_content)?;
                        
                        // Add the style tag with processed CSS
                        let style_tag = format!("<style>{}</style>", processed_css);
                        self.add_raw_stream(&style_tag);
                    }
                }
            },
            "text"  => {
                let content = &source[node.start_byte()..node.end_byte()];
                if !content.trim().is_empty() {
                    let handlebars_result = self.handlebars_parser.parse(content);
                    match handlebars_result {
                        Ok(parsed_streams) => {
                            for stream in parsed_streams {
                                match stream {
                                    WebUIStream::Raw(raw) => self.add_raw_stream(&raw.value),
                                    _ => self.add_stream(stream, streams),
                                }
                            }
                        },
                        Err(error) => return Err(error),
                    }
                }
            },
            // For other node types (like doctype, head, body), traverse their children
            _  => {
                self.process_html_node(node, source, streams)?;
            }
        }
        
        Ok(())
    }
    
    /// Get the tag name of an element.
    fn get_element_tag_name(&self, node: Node, source: &str) -> Result<String> {
        // Create a new cursor for this function
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "start_tag" {
                // Create another cursor for the inner loop
                let mut tag_cursor = child.walk();
                for tag_name_node in child.named_children(&mut tag_cursor) {
                    if tag_name_node.kind() == "tag_name" {
                        let tag_name = source[tag_name_node.start_byte()..tag_name_node.end_byte()].to_string();
                        return Ok(tag_name);
                    }
                }
            }
        }
        
        Err(ParserError::Html("Failed to extract tag name".to_string()))
    }
    
    fn get_element_attribute(&self, node: Node, attr_name: &str, source: &str) -> Result<Option<String>> {
        let query_str = r#"
            (element
              (start_tag
                (attribute
                  (attribute_name) @name
                  [(quoted_attribute_value (attribute_value) @value)
                   (attribute_value) @value]))
            )
        "#;
        
        let query = Query::new(&LANGUAGE.into(), query_str).map_err(|e| {
            ParserError::Html(format!("Failed to create attribute query: {:?}", e))
        })?;
        
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, node, source.as_bytes());
        while let Some(m) = matches.next_mut() {
            let mut found_name = false;
            let mut found_value: Option<String> = None;

            for capture in m.captures.iter() {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];
                
                if capture_name == "name" {
                    let name_text = node.utf8_text(source.as_bytes())
                        .map_err(|_| ParserError::Html("Invalid UTF-8 for attribute name".to_string()))?;
                    if name_text == attr_name {
                        found_name = true;
                    }
                } else if capture_name == "value" {
                    let value_text = node.utf8_text(source.as_bytes())
                        .map_err(|_| ParserError::Html("Invalid UTF-8 for attribute value".to_string()))?;
                    found_value = Some(value_text.to_string());
                }
            }
            
            if found_name {
                return Ok(found_value);
            }
        }
        
        Ok(None)
    }
    
    /// Process a <for> directive.
    fn process_for_directive(
        &mut self,
        node: Node,
        source: &str,
        streams: &mut Vec<WebUIStream>
    ) -> Result<()> {
        self.flush_raw_buffer(streams);

        // Extract each attribute
        let each = self.get_element_attribute(node, "each", source)?
            .ok_or_else(|| ParserError::Directive("Missing 'each' attribute on <for>".to_string()))?;
        
        // Split the each by whitespace.
        let parts: Vec<&str> = each.split_whitespace().collect();
        if parts.len() != 3 || parts[1] != "in" {
            return Err(ParserError::Directive(format!("Invalid for each: {}", each)));
        }

        // Check that the first and third part contain only allowed characters.
        let allowed = |s: &str| {
            s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        };
        if !allowed(parts[0]) || !allowed(parts[2]) {
            return Err(ParserError::Directive(format!("Invalid identifier in for each: {}", each)));
        }

        let item = parts[0];
        let collection = parts[2];
        
        // Generate a unique stream ID for the for loop content
        let stream_id = self.id_counter.next_id("for");
        let mut for_stream: Vec<WebUIStream> = Vec::new();

        // Process the for loop body
        for child in node.named_children(&mut node.walk()) {
            if child.kind() != "start_tag" && child.kind() != "end_tag" {
                self.process_child_node(child, source, &mut for_stream)?;
            }
        }
        
        // Store the record.
        self.stream_records.insert(stream_id.clone(), for_stream);
        
        // Add the for directive stream
        self.add_for_stream(
            item.to_string(),
            collection.to_string(),
            stream_id,
            streams
        );
        
        Ok(())
    }
    
    /// Process an <if> directive.
    fn process_if_directive(
        &mut self,
        node: Node,
        source: &str,
        streams: &mut Vec<WebUIStream>
    ) -> Result<()> {
        self.flush_raw_buffer(streams);

        // Extract condition attribute
        let condition_str = self.get_element_attribute(node, "condition", source)?
            .ok_or_else(|| ParserError::Directive("Missing 'condition' attribute on <if>".to_string()))?;
        
        // Parse condition into a ConditionExpr
        let condition_result = self.condition_parser.parse(&condition_str);
        let condition = match condition_result {
            Ok(cond) => cond,
            Err(_) => return Err(ParserError::Directive(format!(
                "Invalid condition expression: {}", condition_str
            ))),
        };
        
        // Generate a unique stream ID for the if content
        let stream_id = self.id_counter.next_id("if");
        let mut if_stream: Vec<WebUIStream> = Vec::new();

        // Process the if body
        for child in node.named_children(&mut node.walk()) {
            if child.kind() != "start_tag" && child.kind() != "end_tag" {
                self.process_child_node(child, source, &mut if_stream)?;
            }
        }
        
        // Store the record.
        self.stream_records.insert(stream_id.clone(), if_stream);
    
        // Add the if directive stream
        self.add_if_stream(condition, stream_id, streams);
    
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use webui_protocol::ConditionExpr;

    use super::*;
    
    #[test]
    fn test_parse_signal() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = "Hello, {{name}}!";
        let result = parser.parse("test.html", html);
        
        assert!(result.is_ok());
        let stream_records = parser.into_stream_records();
        let streams = stream_records.get("test.html").unwrap();
        assert_eq!(streams.len(), 3);
        
        // Verify each stream
        assert!(matches!(streams.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "Hello, "));
        assert!(matches!(streams.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "name" && !signal.raw
        ));
        assert!(matches!(streams.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "!"));
    }
    
    #[test]
    fn test_parse_raw_signal() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = "Hello, {{{html_content}}}!";
        let result = parser.parse("test.html", html);
        
        assert!(result.is_ok());
        let stream_records = parser.into_stream_records();
        let streams = stream_records.get("test.html").unwrap();
        assert_eq!(streams.len(), 3);

        // Verify each stream
        assert!(matches!(streams.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "Hello, "));
        assert!(matches!(streams.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "html_content" && signal.raw
        ));
        assert!(matches!(streams.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "!"));
    }
    
    #[test]
    fn test_parse_for_directive() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = r#"<for each="item in items"><div class="item">{{item.name}}</div></for>"#;
        
        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let stream_records = parser.into_stream_records();
        let streams = stream_records.get("test.html").unwrap();

        // Verify each stream
        assert_eq!(streams.len(), 1);

        assert!(matches!(streams.get(0), Some(WebUIStream::For(for_loop)) if 
            for_loop.item == "item" &&
            for_loop.collection == "items" &&
            for_loop.stream_id == "for-1"
        ));
        
        // Verify the sub-stream contains our item content
        let for_stream = stream_records.get("for-1").unwrap();
        assert_eq!(for_stream.len(), 3);
        assert!(matches!(for_stream.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "<div class=\"item\">"));
        assert!(matches!(for_stream.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "item.name" && !signal.raw
        ));
        assert!(matches!(for_stream.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "</div>"));
    }

    #[test]
    fn test_parse_if_directive() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = r#"<if condition="isLoggedIn"><div>Welcome back, {{username}}!</div></if>"#;
        
        let result = parser.parse("test.html", html);
        
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let stream_records = parser.into_stream_records();
        let streams = stream_records.get("test.html").unwrap();
        assert_eq!(streams.len(), 1);

        assert!(matches!(streams.get(0), Some(WebUIStream::If(if_cond)) if 
            matches!(&if_cond.condition, ConditionExpr::Identifier { value } if value == "isLoggedIn") &&
            if_cond.stream_id == "if-1"
        ));
        
        // Verify the sub-stream contains our content
        let if_stream = stream_records.get("if-1").unwrap();
        assert_eq!(if_stream.len(), 3);
        assert!(matches!(if_stream.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "<div>Welcome back, "));
        assert!(matches!(if_stream.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "username" && !signal.raw
        ));
        assert!(matches!(if_stream.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "!</div>"));
    }

    #[test]
    fn test_nested_directives() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = r#"<for each="category in categories">
            <if condition="category.hasItems">
                <for each="item in category.items">
                   {{item.title}}
                </for>
            </if>
        </for>"#;
        
        let result = parser.parse("test.html", html);
        
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let stream_records = parser.into_stream_records();
        
        let streams = stream_records.get("test.html").unwrap();
        assert_eq!(streams.len(), 1);
        assert!(matches!(streams.get(0), Some(WebUIStream::For(for_loop)) if 
            for_loop.item == "category" &&
            for_loop.collection == "categories" &&
            for_loop.stream_id == "for-1"
        ));
        
        let for_stream = stream_records.get("for-1").unwrap();
        assert_eq!(for_stream.len(), 1);
        assert!(matches!(for_stream.get(0), Some(WebUIStream::If(if_cond)) if 
            matches!(&if_cond.condition, ConditionExpr::Identifier { value } if value == "category.hasItems") &&
            if_cond.stream_id == "if-1"
        ));

        let if_stream = stream_records.get("if-1").unwrap();
        assert_eq!(if_stream.len(), 1);
        assert!(matches!(if_stream.get(0), Some(WebUIStream::For(for_loop)) if 
            for_loop.item == "item" &&
            for_loop.collection == "category.items" &&
            for_loop.stream_id == "for-2"
        ));

        let nested_for_stream = stream_records.get("for-2").unwrap();
        assert_eq!(nested_for_stream.len(), 1);
        assert!(matches!(nested_for_stream.get(0), Some(WebUIStream::Signal(signal)) if 
            signal.value == "item.title" && !signal.raw
        ));
    }

    #[test]
    fn test_complex_directives() {
        let mut parser = HtmlParser::new(WebUIStreamRecords::new());
        let html = r#"<for each="category in categories">
            <div class="category">
                <h2>{{category.name}}</h2>
                <if condition="category.hasItems">
                    <ul>
                        <for each="item in category.items">
                            <li>{{item.title}}</li>
                        </for>
                    </ul>
                </if>
            </div>
        </for>"#;

        let result = parser.parse("test.html", html);
        
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let stream_records = parser.into_stream_records();
        let streams = stream_records.get("test.html").unwrap();
        assert_eq!(streams.len(), 1);
        
        assert!(matches!(streams.get(0), Some(WebUIStream::For(for_loop)) if 
            for_loop.item == "category" &&
            for_loop.collection == "categories" &&
            for_loop.stream_id == "for-1"
        ));

        // Verify for streams contains the category.name signal
        let for_streams: &Vec<WebUIStream> = stream_records.get("for-1").unwrap();
        assert_eq!(for_streams.len(), 5);
        assert!(matches!(for_streams.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "<div class=\"category\"><h2>"));
        assert!(matches!(for_streams.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "category.name" && !signal.raw
        ));
        assert!(matches!(for_streams.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "</h2>"));
        assert!(matches!(for_streams.get(3), Some(WebUIStream::If(if_cond)) if 
            matches!(&if_cond.condition, ConditionExpr::Identifier { value } if value == "category.hasItems") &&
            if_cond.stream_id == "if-1"
        ));
        assert!(matches!(for_streams.get(4), Some(WebUIStream::Raw(raw)) if raw.value == "</div>"));

        // Verify nested if condition.
        let if_streams: &Vec<WebUIStream> = stream_records.get("if-1").unwrap();
        assert_eq!(if_streams.len(), 3);
        assert!(matches!(if_streams.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "<ul>"));
        assert!(matches!(if_streams.get(1), Some(WebUIStream::For(for_loop)) if 
            for_loop.item == "item" &&
            for_loop.collection == "category.items" &&
            for_loop.stream_id == "for-2"
        ));
        assert!(matches!(if_streams.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "</ul>"));

        // Verify nested for each.
        let nested_for_streams: &Vec<WebUIStream> = stream_records.get("for-2").unwrap();
        assert_eq!(nested_for_streams.len(), 3);
        assert!(matches!(nested_for_streams.get(0), Some(WebUIStream::Raw(raw)) if raw.value == "<li>"));
        assert!(matches!(nested_for_streams.get(1), Some(WebUIStream::Signal(signal)) if 
            signal.value == "item.title" && !signal.raw
        ));
        assert!(matches!(nested_for_streams.get(2), Some(WebUIStream::Raw(raw)) if raw.value == "</li>"));
    }
}
