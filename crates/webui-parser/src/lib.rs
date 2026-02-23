//! Directive parser for WebUI template directives.
//!
//! This module handles parsing WebUI-specific directives like <for>, <if>, etc.
mod component_registry;
mod condition_parser;
mod css_parser;
mod error;
mod handlebars_parser;

pub use component_registry::{Component, ComponentRegistry};
pub use condition_parser::ConditionParser;
pub use css_parser::CssParser;
pub use error::{ParserError, Result};
pub use handlebars_parser::HandlebarsParser;

use std::collections::HashMap;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIteratorMut};
use tree_sitter_html::LANGUAGE;
use webui_protocol::{
    web_ui_fragment::Fragment, ConditionExpr, FragmentList, WebUIFragment, WebUIFragmentRecords,
};

/// Counter for generating unique fragment IDs.
struct FragmentIdCounter {
    /// Map of counter types to their current values.
    counters: HashMap<String, usize>,
}

impl FragmentIdCounter {
    /// Create a new fragment ID counter.
    fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    /// Generate a unique fragment ID.
    fn next_id(&mut self, prefix: &str) -> String {
        let count = self.counters.entry(prefix.to_string()).or_insert(0);
        *count += 1;
        format!("{}-{}", prefix, count)
    }
}

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Parser for WebUI directives.
pub struct HtmlParser {
    /// CSS parser.
    css_parser: CssParser,

    /// Tree-sitter parser for HTML.
    parser: Parser,

    /// Fragment ID counter.
    id_counter: FragmentIdCounter,

    /// Condition parser for parsing conditions in directives.
    condition_parser: ConditionParser,

    /// Handlebars parser for parsing handlebars expressions.
    handlebars_parser: HandlebarsParser,

    /// Component registry for WebUI components.
    component_registry: ComponentRegistry,

    /// Map of fragment IDs to their fragments
    fragment_records: WebUIFragmentRecords,

    /// Buffer for accumulating raw content
    raw_buffer: String,
}

impl HtmlParser {
    /// Create a new directive parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE.into())
            .expect("Error loading HTML grammar");

        Self {
            component_registry: ComponentRegistry::new(),
            css_parser: CssParser::new(),
            id_counter: FragmentIdCounter::new(),
            condition_parser: ConditionParser::new(),
            handlebars_parser: HandlebarsParser::new(),
            raw_buffer: String::new(),
            fragment_records: WebUIFragmentRecords::new(),
            parser,
        }
    }

    /// Get a mutable reference to the component registry.
    pub fn component_registry_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.component_registry
    }

    pub fn into_fragment_records(mut self) -> WebUIFragmentRecords {
        std::mem::take(&mut self.fragment_records)
    }

    /// Parse HTML content to generate WebUI fragments.
    pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<()> {
        // Reset sub-fragments for new parse
        self.raw_buffer.clear();

        // Parse HTML
        let tree = self
            .parser
            .parse(html_content, None)
            .ok_or_else(|| ParserError::Html("Failed to parse HTML".to_string()))?;

        let mut entry_fragment: Vec<WebUIFragment> = Vec::new();

        // Start processing the HTML node.
        self.process_html_node(tree.root_node(), html_content, &mut entry_fragment)?;

        self.flush_raw_buffer(&mut entry_fragment);

        // Insert the entry record.
        self.fragment_records.insert(
            fragment_id.to_string(),
            FragmentList {
                fragments: entry_fragment,
            },
        );

        // Return all fragments including generated sub-fragments
        Ok(())
    }

    /// Add raw content to the buffer
    fn add_raw_fragment(&mut self, content: &str) {
        if !content.is_empty() {
            self.raw_buffer.push_str(content);
        }
    }

    /// Add a for fragment, flushing raw buffer first
    fn add_for_fragment(
        &mut self,
        item: String,
        collection: String,
        fragment_id: String,
        fragments: &mut Vec<WebUIFragment>,
    ) {
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::for_loop(item, collection, fragment_id));
    }

    /// Add an if fragment, flushing raw buffer first
    fn add_if_fragment(
        &mut self,
        condition: ConditionExpr,
        fragment_id: String,
        fragments: &mut Vec<WebUIFragment>,
    ) {
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::if_cond(condition, fragment_id));
    }

    /// Add a component fragment, flushing raw buffer first
    fn add_component_fragment(&mut self, fragment_id: String, fragments: &mut Vec<WebUIFragment>) {
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::component(fragment_id));
    }

    /// Add a non-raw fragment, flushing the raw buffer first if needed
    fn add_fragment(&mut self, fragment: WebUIFragment, fragments: &mut Vec<WebUIFragment>) {
        self.flush_raw_buffer(fragments);
        fragments.push(fragment);
    }

    /// Flush the raw buffer into fragments if not empty
    fn flush_raw_buffer(&mut self, fragments: &mut Vec<WebUIFragment>) {
        if !self.raw_buffer.is_empty() {
            fragments.push(WebUIFragment::raw(std::mem::take(&mut self.raw_buffer)));
        }
    }

    /// Process an HTML node to generate WebUI fragments.
    fn process_html_node(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        let mut cursor = node.walk();

        if node.kind() == "document" || node.kind() == "fragment" || node.kind() == "element" {
            // Process children
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();

                    // Process child node
                    self.process_child_node(child, source, fragments)?;

                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            }
        } else {
            // Add text content as raw fragment
            let content = &source[node.start_byte()..node.end_byte()];
            if !content.trim().is_empty() {
                self.add_raw_fragment(content);
            }
        }

        Ok(())
    }

    /// Process a child HTML node.
    fn process_child_node(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        match node.kind() {
            "element" => {
                // Get the tag name
                let tag_name = self.get_element_tag_name(node, source)?;

                // Handle WebUI directives
                match tag_name.as_str() {
                    "for" => return self.process_for_directive(node, source, fragments),
                    "if" => return self.process_if_directive(node, source, fragments),
                    _ => {
                        if self.component_registry.contains(tag_name.as_str()) {
                            return self.process_component_directive(
                                node,
                                source,
                                fragments,
                                tag_name.as_str(),
                            );
                        }

                        // Process regular HTML element with attribute-aware parsing
                        self.process_regular_element(node, source, fragments, &tag_name)?;

                        return Ok(());
                    }
                }
            }
            "style_element" => {
                // Process inline CSS
                for child in node.named_children(&mut node.walk()) {
                    if child.kind() == "raw_text" {
                        let style_content = &source[child.start_byte()..child.end_byte()];
                        let processed_css = self.css_parser.parse_inline_css(style_content)?;

                        // Add the style tag with processed CSS
                        let style_tag = format!("<style>{}</style>", processed_css);
                        self.add_raw_fragment(&style_tag);
                    }
                }
            }
            "text" => {
                let content = &source[node.start_byte()..node.end_byte()];
                if !content.trim().is_empty() {
                    let handlebars_result = self.handlebars_parser.parse(content);
                    match handlebars_result {
                        Ok(parsed_fragments) => {
                            for fragment in parsed_fragments {
                                if matches!(fragment.fragment.as_ref(), Some(Fragment::Raw(_))) {
                                    if let Some(Fragment::Raw(raw)) = fragment.fragment.as_ref() {
                                        self.add_raw_fragment(&raw.value);
                                    }
                                } else {
                                    self.add_fragment(fragment, fragments);
                                }
                            }
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            // For other node types (like doctype, head, body), traverse their children
            _ => {
                self.process_html_node(node, source, fragments)?;
            }
        }

        Ok(())
    }

    /// Get the tag name of an element.
    fn get_element_tag_name(&self, node: Node, source: &str) -> Result<String> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "start_tag" || child.kind() == "self_closing_tag" {
                let mut tag_cursor = child.walk();
                for tag_name_node in child.named_children(&mut tag_cursor) {
                    if tag_name_node.kind() == "tag_name" {
                        let tag_name = source[tag_name_node.start_byte()..tag_name_node.end_byte()]
                            .to_string();
                        return Ok(tag_name);
                    }
                }
            }
        }

        Err(ParserError::Html("Failed to extract tag name".to_string()))
    }

    fn get_element_attribute(
        &self,
        node: Node,
        attr_name: &str,
        source: &str,
    ) -> Result<Option<String>> {
        let query_str = r#"
            (element
              [
                (start_tag
                  (attribute
                    (attribute_name) @name
                    [(quoted_attribute_value (attribute_value) @value)
                     (attribute_value) @value]))
                (self_closing_tag
                  (attribute
                    (attribute_name) @name
                    [(quoted_attribute_value (attribute_value) @value)
                     (attribute_value) @value]))
              ]
            )
        "#;

        let query = Query::new(&LANGUAGE.into(), query_str)
            .map_err(|e| ParserError::Html(format!("Failed to create attribute query: {:?}", e)))?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, node, source.as_bytes());
        while let Some(m) = matches.next_mut() {
            let mut found_name = false;
            let mut found_value: Option<String> = None;

            for capture in m.captures.iter() {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                if capture_name == "name" {
                    let name_text = node.utf8_text(source.as_bytes()).map_err(|_| {
                        ParserError::Html("Invalid UTF-8 for attribute name".to_string())
                    })?;
                    if name_text == attr_name {
                        found_name = true;
                    }
                } else if capture_name == "value" {
                    let value_text = node.utf8_text(source.as_bytes()).map_err(|_| {
                        ParserError::Html("Invalid UTF-8 for attribute value".to_string())
                    })?;
                    found_value = Some(value_text.to_string());
                }
            }

            if found_name {
                return Ok(found_value);
            }
        }

        Ok(None)
    }

    /// Find the start_tag or self_closing_tag child of an element node.
    fn find_tag_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        let result = node
            .named_children(&mut cursor)
            .find(|child| child.kind() == "start_tag" || child.kind() == "self_closing_tag");
        result
    }

    /// Check if an attribute value is a pure handlebars expression (e.g., "{{name}}" or
    /// "{{name}}" with quotes). Returns the inner signal name if so.
    fn extract_single_handlebars(value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.starts_with("{{") && trimmed.ends_with("}}") && !trimmed.starts_with("{{{") {
            let inner = trimmed[2..trimmed.len() - 2].trim();
            // Verify there's no other {{ in the middle (i.e., it's truly a single expression)
            if !inner.contains("{{") && !inner.is_empty() {
                return Some(inner.to_string());
            }
        }
        None
    }

    /// Check if an attribute value contains any handlebars expressions.
    fn contains_handlebars(value: &str) -> bool {
        value.contains("{{")
    }

    /// Process a regular HTML element with attribute-aware parsing.
    fn process_regular_element(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
        tag_name: &str,
    ) -> Result<()> {
        let tag_node = self.find_tag_node(node);
        let is_self_closing = tag_node
            .map(|n| n.kind() == "self_closing_tag")
            .unwrap_or(false);

        if let Some(tag_node) = tag_node {
            // Emit "<tagname" as the start of raw content
            self.add_raw_fragment(&format!("<{}", tag_name));

            // Process attributes on the tag
            self.process_tag_attributes(tag_node, source, fragments)?;

            if is_self_closing {
                // Find the /> at the end of the self-closing tag
                let tag_text = &source[tag_node.start_byte()..tag_node.end_byte()];
                if tag_text.ends_with("/>") {
                    self.add_raw_fragment("/>");
                } else {
                    self.add_raw_fragment(">");
                }
            } else {
                // Emit ">"
                self.add_raw_fragment(">");

                // Process children (skip start_tag and end_tag nodes)
                for child in node.named_children(&mut node.walk()) {
                    let kind = child.kind();
                    if kind != "start_tag" && kind != "end_tag" && kind != "self_closing_tag" {
                        self.process_child_node(child, source, fragments)?;
                    }
                }

                // Add closing tag
                self.add_raw_fragment(&format!("</{}>", tag_name));
            }
        }

        Ok(())
    }

    /// Process all attributes on a start_tag or self_closing_tag node.
    /// Emits attribute fragments for dynamic attributes and accumulates static
    /// attributes into the raw buffer.
    fn process_tag_attributes(
        &mut self,
        tag_node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        let mut cursor = tag_node.walk();
        for child in tag_node.named_children(&mut cursor) {
            if child.kind() != "attribute" {
                continue;
            }

            let attr_name = self.get_attr_name(child, source)?;
            let attr_value = self.get_attr_value(child, source);

            if let Some(bool_name) = attr_name.strip_prefix('?') {
                // Boolean attribute: ?disabled={{isDisabled}}
                self.process_boolean_attribute(bool_name, attr_value.as_deref(), fragments)?;
            } else if attr_name.starts_with(':') {
                // Complex attribute: :config="{{settings}}"
                self.process_complex_attribute(&attr_name, attr_value.as_deref(), fragments)?;
            } else if let Some(ref val) = attr_value {
                if Self::contains_handlebars(val) {
                    // Dynamic attribute with handlebars
                    self.process_dynamic_attribute(&attr_name, val, fragments)?;
                } else {
                    // Static attribute — emit as raw
                    let attr_text = &source[child.start_byte()..child.end_byte()];
                    self.add_raw_fragment(&format!(" {}", attr_text));
                }
            } else {
                // Attribute without value (e.g., "disabled") — emit as raw
                self.add_raw_fragment(&format!(" {}", attr_name));
            }
        }
        Ok(())
    }

    /// Extract the attribute name from an attribute node.
    fn get_attr_name(&self, attr_node: Node, source: &str) -> Result<String> {
        let mut cursor = attr_node.walk();
        for child in attr_node.children(&mut cursor) {
            if child.kind() == "attribute_name" {
                return Ok(source[child.start_byte()..child.end_byte()].to_string());
            }
        }
        Err(ParserError::Html(
            "Attribute node missing attribute_name".to_string(),
        ))
    }

    /// Extract the attribute value from an attribute node (handles both quoted and
    /// unquoted forms). Returns None if there is no value.
    fn get_attr_value(&self, attr_node: Node, source: &str) -> Option<String> {
        let mut cursor = attr_node.walk();
        for child in attr_node.children(&mut cursor) {
            match child.kind() {
                "quoted_attribute_value" => {
                    // Find the inner attribute_value node
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "attribute_value" {
                            return Some(source[inner.start_byte()..inner.end_byte()].to_string());
                        }
                    }
                    // Quoted but empty value — return empty string
                    return Some(String::new());
                }
                "attribute_value" => {
                    return Some(source[child.start_byte()..child.end_byte()].to_string());
                }
                _ => {}
            }
        }
        None
    }

    /// Process a boolean attribute (?prefix). Silently drops if value is not a
    /// pure handlebars expression.
    fn process_boolean_attribute(
        &mut self,
        name: &str,
        value: Option<&str>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        if let Some(val) = value {
            if let Some(signal_name) = Self::extract_single_handlebars(val) {
                // Valid boolean attribute — emit as attribute fragment with conditionTree
                let condition = ConditionExpr::identifier(&signal_name);
                self.add_fragment(WebUIFragment::attribute_boolean(name, condition), fragments);
                return Ok(());
            }
        }
        // Invalid boolean attribute — silently drop (no output at all)
        Ok(())
    }

    /// Process a complex attribute (:prefix).
    fn process_complex_attribute(
        &mut self,
        name: &str,
        value: Option<&str>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        if let Some(val) = value {
            if let Some(signal_name) = Self::extract_single_handlebars(val) {
                self.add_fragment(
                    WebUIFragment::attribute_complex(name, signal_name),
                    fragments,
                );
                return Ok(());
            }
        }
        // No valid handlebars — emit as raw
        self.add_raw_fragment(&format!(" {}=\"{}\"", name, value.unwrap_or("")));
        Ok(())
    }

    /// Process a dynamic attribute (regular name with handlebars in value).
    fn process_dynamic_attribute(
        &mut self,
        name: &str,
        value: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        if let Some(signal_name) = Self::extract_single_handlebars(value) {
            // Pure handlebars — simple attribute fragment
            self.add_fragment(WebUIFragment::attribute(name, signal_name), fragments);
        } else {
            // Mixed static + dynamic — create a template sub-stream
            let template_id = self.id_counter.next_id("attr");
            let parsed = self.handlebars_parser.parse(value)?;

            self.fragment_records
                .insert(template_id.clone(), FragmentList { fragments: parsed });

            self.add_fragment(
                WebUIFragment::attribute_template(name, template_id),
                fragments,
            );
        }
        Ok(())
    }

    /// Process a <for> directive.
    fn process_for_directive(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        // Extract each attribute
        let each = self
            .get_element_attribute(node, "each", source)?
            .ok_or_else(|| {
                ParserError::Directive("Missing 'each' attribute on <for>".to_string())
            })?;

        // Split the each by whitespace.
        let parts: Vec<&str> = each.split_whitespace().collect();
        if parts.len() != 3 || parts[1] != "in" {
            return Err(ParserError::Directive(format!(
                "Invalid for each: {}",
                each
            )));
        }

        // Check that the first and third part contain only allowed characters.
        let allowed = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        };
        if !allowed(parts[0]) || !allowed(parts[2]) {
            return Err(ParserError::Directive(format!(
                "Invalid identifier in for each: {}",
                each
            )));
        }

        let item = parts[0];
        let collection = parts[2];

        // Generate a unique fragment ID for the for loop content
        let fragment_id = self.id_counter.next_id("for");
        let mut for_fragment: Vec<WebUIFragment> = Vec::new();

        // Create a temporary buffer for for loop content
        let mut temp_buffer = String::new();
        std::mem::swap(&mut self.raw_buffer, &mut temp_buffer);

        // Process the for loop body
        for child in node.named_children(&mut node.walk()) {
            if child.kind() != "start_tag" && child.kind() != "end_tag" {
                self.process_child_node(child, source, &mut for_fragment)?;
            }
        }

        // Ensure any remaining content is flushed to the for loop's fragment
        self.flush_raw_buffer(&mut for_fragment);

        // Restore the original buffer
        std::mem::swap(&mut self.raw_buffer, &mut temp_buffer);

        // Store the record
        self.fragment_records.insert(
            fragment_id.clone(),
            FragmentList {
                fragments: for_fragment,
            },
        );

        // Add the for directive fragment to the parent fragment
        self.add_for_fragment(
            item.to_string(),
            collection.to_string(),
            fragment_id,
            fragments,
        );

        Ok(())
    }

    /// Process an <if> directive.
    fn process_if_directive(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        // Extract condition attribute
        let condition_str = self
            .get_element_attribute(node, "condition", source)?
            .ok_or_else(|| {
                ParserError::Directive("Missing 'condition' attribute on <if>".to_string())
            })?;

        // Parse condition into a ConditionExpr
        let condition_result = self.condition_parser.parse(&condition_str);
        let condition = match condition_result {
            Ok(cond) => cond,
            Err(_) => {
                return Err(ParserError::Directive(format!(
                    "Invalid condition expression: {}",
                    condition_str
                )))
            }
        };

        // Generate a unique fragment ID for the if content
        let fragment_id = self.id_counter.next_id("if");

        // Flush any existing content in the parent fragment before switching context
        self.flush_raw_buffer(fragments);

        // Create a separate fragment for the if condition content
        let mut if_fragment: Vec<WebUIFragment> = Vec::new();

        // Save the current raw buffer and create a new one for the if condition
        let parent_buffer = std::mem::take(&mut self.raw_buffer);

        // Process the if body - capture all content including closing tags
        for child in node.named_children(&mut node.walk()) {
            if child.kind() != "start_tag" && child.kind() != "end_tag" {
                self.process_child_node(child, source, &mut if_fragment)?;
            }
        }

        // Make sure all content in the if buffer is flushed to the if fragment
        self.flush_raw_buffer(&mut if_fragment);

        // Store the if fragment in the records
        self.fragment_records.insert(
            fragment_id.clone(),
            FragmentList {
                fragments: if_fragment,
            },
        );

        // Restore the parent buffer - only after we've processed all if content
        self.raw_buffer = parent_buffer;

        // Add the if directive to the parent fragment
        self.add_if_fragment(condition, fragment_id, fragments);

        Ok(())
    }

    // Simplify the process_component_directive method
    fn process_component_directive(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
        tag_name: &str,
    ) -> Result<()> {
        // Build opening tag with attributes
        let mut opening_tag = format!("<{}", tag_name);
        for child in node.named_children(&mut node.walk()) {
            if child.kind() == "attribute" {
                let attr_text = &source[child.start_byte()..child.end_byte()];
                opening_tag.push_str(attr_text);
            }
        }
        opening_tag.push('>');

        // Add shadow DOM template opening
        let start_content = format!("{}<template shadowrootmode=\"open\">", opening_tag);
        self.add_raw_fragment(&start_content);

        // Explicitly flush the buffer to create a Raw fragment with the opening content
        self.flush_raw_buffer(fragments);

        // Get the component data we need
        let html_content;
        {
            let component = self.component_registry.get(tag_name).ok_or_else(|| {
                ParserError::Directive(format!("Component not found: {}", tag_name))
            })?;
            html_content = component.html_content.clone();
        }

        // Check if we need to parse the component template
        if !self.fragment_records.contains_key(tag_name) {
            // Parse component HTML content and add to fragment records
            let _ = self.parse(tag_name, &html_content);
        }

        // Add component fragment directly to output fragments - buffer is already flushed
        self.add_component_fragment(tag_name.to_string(), fragments);

        // Start building the closing part with template end
        self.add_raw_fragment("</template>");

        // Process slot content
        for child in node.named_children(&mut node.walk()) {
            if child.kind() != "start_tag" && child.kind() != "end_tag" {
                if child.kind() == "element" {
                    // For element slots, extract the full source text
                    let slot_content = &source[child.start_byte()..child.end_byte()];
                    self.add_raw_fragment(slot_content);
                } else {
                    // For text nodes and others, use normal processing
                    self.process_child_node(child, source, fragments)?;
                }
            }
        }

        // Add closing component tag
        self.add_raw_fragment(&format!("</{}>", tag_name));

        // Don't flush here to let it combine with any subsequent raw content

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use webui_protocol::condition_expr;

    use super::*;

    #[test]
    fn test_parse_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{name}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 3);

        // Verify each fragment
        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "Hello, ")
        );
        assert!(
            matches!(fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "name" && !signal.raw
            )
        );
        assert!(
            matches!(fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "!")
        );
    }

    #[test]
    fn test_parse_raw_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{{html_content}}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 3);

        // Verify each fragment
        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "Hello, ")
        );
        assert!(
            matches!(fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "html_content" && signal.raw
            )
        );
        assert!(
            matches!(fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "!")
        );
    }

    #[test]
    fn test_parse_for_directive() {
        let mut parser = HtmlParser::new();
        let html = r#"<for each="item in items"><div class="item">{{item.name}}</div></for>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();
        println!("Fragment records: {:#?}", fragment_records);
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;

        // Verify each fragment
        assert_eq!(fragments.len(), 1);

        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::ForLoop(for_loop)) if
                for_loop.item == "item" &&
                for_loop.collection == "items" &&
                for_loop.fragment_id == "for-1"
            )
        );

        // Verify the sub-fragment contains our item content
        let for_fragment_list = fragment_records
            .get("for-1")
            .expect("Failed to get for-1 fragment");
        let for_fragment = &for_fragment_list.fragments;
        assert_eq!(for_fragment.len(), 3);
        assert!(
            matches!(for_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<div class=\"item\">")
        );
        assert!(
            matches!(for_fragment.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "item.name" && !signal.raw
            )
        );
        assert!(
            matches!(for_fragment.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</div>")
        );
    }

    #[test]
    fn test_parse_if_directive() {
        let mut parser = HtmlParser::new();
        let html = r#"<if condition="isLoggedIn"><div>Welcome back, {{username}}!</div></if>"#;

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();
        println!("Fragment records: {:#?}", fragment_records);
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 1);

        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::IfCond(if_cond)) if
                matches!(if_cond.condition.as_ref().and_then(|c| c.expr.as_ref()), Some(condition_expr::Expr::Identifier(id)) if id.value == "isLoggedIn") &&
                if_cond.fragment_id == "if-1"
            )
        );

        // Verify the sub-fragment contains our content
        let if_fragment_list = fragment_records
            .get("if-1")
            .expect("Failed to get if-1 fragment");
        let if_fragment = &if_fragment_list.fragments;
        assert_eq!(if_fragment.len(), 3);
        assert!(
            matches!(if_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<div>Welcome back, ")
        );
        assert!(
            matches!(if_fragment.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "username" && !signal.raw
            )
        );
        assert!(
            matches!(if_fragment.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "!</div>")
        );
    }

    #[test]
    fn test_component_directive() {
        let mut parser = HtmlParser::new();
        let html = r#"<my-component></my-component>"#;

        // Register the component
        assert!(
            parser
                .component_registry
                .register_component(
                    "my-component",
                    "<div>My Component</div>",
                    Some("div { color: blue; }")
                )
                .is_ok(),
            "Failed to register component"
        );

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();
        println!("Fragment records: {:#?}", fragment_records);
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 3);

        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<my-component><template shadowrootmode=\"open\">")
        );
        assert!(
            matches!(fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Component(component)) if
                component.fragment_id == "my-component"
            )
        );
        assert!(
            matches!(fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</template></my-component>")
        );

        // Verify the sub-fragment contains our component content
        let component_fragment_list = fragment_records
            .get("my-component")
            .expect("Failed to get my-component fragment");
        let component_fragment = &component_fragment_list.fragments;
        assert_eq!(component_fragment.len(), 1);
        assert!(
            matches!(component_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if
                raw.value == "<div>My Component</div>"
            )
        );
    }

    #[test]
    fn test_component_directive_with_slots() {
        let mut parser = HtmlParser::new();
        let html = r#"Hello<my-component><p>World</p></my-component>"#;

        // Register the component
        assert!(
            parser
                .component_registry
                .register_component(
                    "my-component",
                    "<div>My Component</div>",
                    Some("div { color: blue; }")
                )
                .is_ok(),
            "Failed to register component"
        );

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();
        println!("Fragment records: {:#?}", fragment_records);
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 3);

        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "Hello<my-component><template shadowrootmode=\"open\">")
        );
        assert!(
            matches!(fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Component(component)) if
                component.fragment_id == "my-component"
            )
        );
        assert!(
            matches!(fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</template><p>World</p></my-component>")
        );

        // Verify the sub-fragment contains our component content
        let component_fragment_list = fragment_records
            .get("my-component")
            .expect("Failed to get my-component fragment");
        let component_fragment = &component_fragment_list.fragments;
        assert_eq!(component_fragment.len(), 1);
        assert!(
            matches!(component_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if
                raw.value == "<div>My Component</div>"
            )
        );
    }

    #[test]
    fn test_nested_directives() {
        let mut parser = HtmlParser::new();
        let html = r#"<for each="category in categories">
            <if condition="category.hasItems">
                <for each="item in category.items">
                   {{item.title}}
                </for>
            </if>
        </for>"#;

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::ForLoop(for_loop)) if
                for_loop.item == "category" &&
                for_loop.collection == "categories" &&
                for_loop.fragment_id == "for-1"
            )
        );

        let for_fragment_list = fragment_records
            .get("for-1")
            .expect("Failed to get for-1 fragment");
        let for_fragment = &for_fragment_list.fragments;
        assert_eq!(for_fragment.len(), 1);
        assert!(
            matches!(for_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::IfCond(if_cond)) if
                matches!(if_cond.condition.as_ref().and_then(|c| c.expr.as_ref()), Some(condition_expr::Expr::Identifier(id)) if id.value == "category.hasItems") &&
                if_cond.fragment_id == "if-1"
            )
        );

        let if_fragment_list = fragment_records
            .get("if-1")
            .expect("Failed to get if-1 fragment");
        let if_fragment = &if_fragment_list.fragments;
        assert_eq!(if_fragment.len(), 1);
        assert!(
            matches!(if_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::ForLoop(for_loop)) if
                for_loop.item == "item" &&
                for_loop.collection == "category.items" &&
                for_loop.fragment_id == "for-2"
            )
        );

        let nested_for_fragment_list = fragment_records
            .get("for-2")
            .expect("Failed to get for-2 fragment");
        let nested_for_fragment = &nested_for_fragment_list.fragments;
        assert_eq!(nested_for_fragment.len(), 1);
        assert!(
            matches!(nested_for_fragment.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "item.title" && !signal.raw
            )
        );
    }

    #[test]
    fn test_complex_directives() {
        let mut parser = HtmlParser::new();
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
        let fragment_records = parser.into_fragment_records();
        let fragment_list = fragment_records
            .get("test.html")
            .expect("Failed to get test.html fragment");
        let fragments = &fragment_list.fragments;
        assert_eq!(fragments.len(), 1);

        assert!(
            matches!(fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::ForLoop(for_loop)) if
                for_loop.item == "category" &&
                for_loop.collection == "categories" &&
                for_loop.fragment_id == "for-1"
            )
        );

        // Verify for fragments contains the category.name signal
        let for_fragments: &Vec<WebUIFragment> = &fragment_records
            .get("for-1")
            .expect("Failed to get for-1 fragment")
            .fragments;
        assert_eq!(for_fragments.len(), 5);
        assert!(
            matches!(for_fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<div class=\"category\"><h2>")
        );
        assert!(
            matches!(for_fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "category.name" && !signal.raw
            )
        );
        assert!(
            matches!(for_fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</h2>")
        );
        assert!(
            matches!(for_fragments.get(3).and_then(|f| f.fragment.as_ref()), Some(Fragment::IfCond(if_cond)) if
                matches!(if_cond.condition.as_ref().and_then(|c| c.expr.as_ref()), Some(condition_expr::Expr::Identifier(id)) if id.value == "category.hasItems") &&
                if_cond.fragment_id == "if-1"
            )
        );
        assert!(
            matches!(for_fragments.get(4).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</div>")
        );

        // Verify nested if condition.
        let if_fragments: &Vec<WebUIFragment> = &fragment_records
            .get("if-1")
            .expect("Failed to get if-1 fragment")
            .fragments;
        assert_eq!(if_fragments.len(), 3);
        assert!(
            matches!(if_fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<ul>")
        );
        assert!(
            matches!(if_fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::ForLoop(for_loop)) if
                for_loop.item == "item" &&
                for_loop.collection == "category.items" &&
                for_loop.fragment_id == "for-2"
            )
        );
        assert!(
            matches!(if_fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</ul>")
        );

        // Verify nested for each.
        let nested_for_fragments: &Vec<WebUIFragment> = &fragment_records
            .get("for-2")
            .expect("Failed to get for-2 fragment")
            .fragments;
        assert_eq!(nested_for_fragments.len(), 3);
        assert!(
            matches!(nested_for_fragments.first().and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "<li>")
        );
        assert!(
            matches!(nested_for_fragments.get(1).and_then(|f| f.fragment.as_ref()), Some(Fragment::Signal(signal)) if
                signal.value == "item.title" && !signal.raw
            )
        );
        assert!(
            matches!(nested_for_fragments.get(2).and_then(|f| f.fragment.as_ref()), Some(Fragment::Raw(raw)) if raw.value == "</li>")
        );
    }

    // ── Attribute fragment tests ─────────────────────────────────────────

    /// Helper to parse HTML and return the fragments for the entry stream.
    fn parse_and_get_fragments(html: &str) -> (Vec<WebUIFragment>, WebUIFragmentRecords) {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = records
            .get("index.html")
            .expect("Failed to get index.html fragment")
            .fragments
            .clone();
        (fragments, records)
    }

    #[test]
    fn test_attribute_handlebars_in_href() {
        // Port of: 'should process handlebars from attributes as signals'
        // <a href="{{url}}">{{name}}</a>
        let (fragments, _) = parse_and_get_fragments(r#"<a href="{{url}}">{{name}}</a>"#);

        assert_eq!(fragments.len(), 5);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<a")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "href" && attr.value == "url" && attr.template.is_empty() && !attr.complex)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">")
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(signal)) if signal.value == "name")
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</a>")
        );
    }

    #[test]
    fn test_attribute_boolean_with_handlebars() {
        // Port of: 'should process boolean attribute with handlebars expression'
        // <button ?disabled={{isDisabled}}>Click</button>
        let (fragments, _) =
            parse_and_get_fragments("<button ?disabled={{isDisabled}}>Click</button>");

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<button")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "disabled" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isDisabled"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">Click</button>")
        );
    }

    #[test]
    fn test_attribute_multiple_boolean() {
        // Port of: 'should process multiple boolean attributes'
        // <input ?checked={{isChecked}} ?disabled={{isDisabled}} />
        let (fragments, _) =
            parse_and_get_fragments("<input ?checked={{isChecked}} ?disabled={{isDisabled}} />");

        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "checked" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isChecked"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "disabled" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isDisabled"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>")
        );
    }

    #[test]
    fn test_attribute_boolean_and_regular_together() {
        // Port of: 'should process a boolean attribute and a regular attribute together'
        // <input ?checked="{{isChecked}}" type="checkbox">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input ?checked="{{isChecked}}" type="checkbox">Hi</input>"#,
        );

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "checked" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isChecked"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == " type=\"checkbox\">Hi</input>")
        );
    }

    #[test]
    fn test_attribute_boolean_sandwiched() {
        // Port of: 'should process a boolean attribute sandwiched between regular attributes'
        // <input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>"#,
        );

        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "version" && attr.value == "edition")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "checked" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isChecked"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == " type=\"checkbox\">Hi</input>")
        );
    }

    #[test]
    fn test_attribute_boolean_ending() {
        // Port of: 'should process html ending with boolean attribute correctly'
        // <input version={{edition}} ?checked="{{isChecked}}">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input version={{edition}} ?checked="{{isChecked}}">Hi</input>"#,
        );

        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "version" && attr.value == "edition")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "checked" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isChecked"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">Hi</input>")
        );
    }

    #[test]
    fn test_attribute_boolean_dotted_path() {
        // Port of: 'should process boolean attribute with dotted path'
        // <div ?checked={{layout.isPinned}}>Content</div>
        let (fragments, _) =
            parse_and_get_fragments("<div ?checked={{layout.isPinned}}>Content</div>");

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<div")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "checked" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "layout.isPinned"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">Content</div>")
        );
    }

    #[test]
    fn test_attribute_colon_prefixed_complex() {
        // Port of: 'should process colon-prefixed attribute with handlebars'
        // <my-component :config="{{settings}}"></my-component>
        let (fragments, _) =
            parse_and_get_fragments(r#"<my-component :config="{{settings}}"></my-component>"#);

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<my-component")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == ":config" && attr.value == "settings" && attr.complex)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "></my-component>")
        );
    }

    #[test]
    fn test_attribute_multiple_colon_prefixed() {
        // Port of: 'should process multiple colon-prefixed complex attributes'
        // <my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
        let (fragments, _) = parse_and_get_fragments(
            r#"<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>"#,
        );

        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<my-component")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == ":prop1" && attr.value == "val1" && attr.complex)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == ":prop2" && attr.value == "val2" && attr.complex)
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "></my-component>")
        );
    }

    #[test]
    fn test_attribute_mixed_normal_boolean_colon() {
        // Port of: 'should process mixed normal, boolean, and colon-prefixed attributes'
        // <my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>
        let (fragments, _) = parse_and_get_fragments(
            r#"<my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>"#,
        );

        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<my-component id=\"comp\"")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == ":config" && attr.value == "settings" && attr.complex)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "enabled" &&
                matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isEnabled"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "></my-component>")
        );
    }

    #[test]
    fn test_attribute_reject_boolean_without_handlebars() {
        // Port of: 'should reject boolean attribute without handlebars'
        // <input ?checked="name"></input>
        let (fragments, _) = parse_and_get_fragments(r#"<input ?checked="name"></input>"#);

        // Boolean attribute is silently dropped
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input></input>")
        );
    }

    #[test]
    fn test_attribute_reject_boolean_with_partial_handlebars() {
        // Port of: 'should reject boolean attribute with partial handlebars'
        // <input ?checked="Hello {{name}}"></input>
        let (fragments, _) =
            parse_and_get_fragments(r#"<input ?checked="Hello {{name}}"></input>"#);

        // Boolean attribute is silently dropped
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input></input>")
        );
    }

    #[test]
    fn test_attribute_reject_boolean_with_plain_value() {
        // Port of: 'should reject boolean attribute with plain value'
        // <button ?disabled="true">Click</button>
        let (fragments, _) = parse_and_get_fragments(r#"<button ?disabled="true">Click</button>"#);

        // Boolean attribute is silently dropped
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<button>Click</button>")
        );
    }

    #[test]
    fn test_attribute_mixed_static_dynamic() {
        // Port of: 'should process mixed attributes correctly'
        // <input value="hello {{world}}">Hi</input>
        let (fragments, records) =
            parse_and_get_fragments(r#"<input value="hello {{world}}">Hi</input>"#);

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if
                attr.name == "value" && attr.template == "attr-1")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">Hi</input>")
        );

        // Verify the template sub-stream
        let attr_stream = records.get("attr-1").expect("Missing attr-1 sub-stream");
        assert_eq!(attr_stream.fragments.len(), 2);
        assert!(
            matches!(attr_stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "hello ")
        );
        assert!(
            matches!(attr_stream.fragments[1].fragment.as_ref(), Some(Fragment::Signal(signal)) if signal.value == "world")
        );
    }
}
