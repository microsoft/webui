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
    web_ui_fragment, web_ui_fragment::Fragment, ConditionExpr, FragmentList, WebUIFragment,
    WebUIFragmentAttribute, WebUIFragmentRecords,
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
                    "body" => return self.process_body_element(node, source, fragments),
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
            // Preserve <!DOCTYPE ...> as raw content. Tree-sitter parses it as a
            // "doctype" node whose children are tokens (< ! DOCTYPE etc.), so we
            // grab the original source text verbatim to ensure full-page HTML
            // templates round-trip correctly.
            "doctype" => {
                let content = &source[node.start_byte()..node.end_byte()];
                self.add_raw_fragment(content);
            }
            // For other node types (like head, body), traverse their children
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

    /// Check if an element node has an end_tag child.
    fn has_end_tag(&self, node: Node) -> bool {
        let mut cursor = node.walk();
        let result = node
            .named_children(&mut cursor)
            .any(|child| child.kind() == "end_tag");
        result
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
        let has_end = self.has_end_tag(node);

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
            } else if !has_end {
                // Void element (no end tag from parser) — just close the opening tag
                self.add_raw_fragment(">");
            } else {
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

    /// Process a `<body>` element, injecting body_start/body_end signals.
    fn process_body_element(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        self.add_raw_fragment("<body>");
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::signal("body_start", true));
        for child in node.named_children(&mut node.walk()) {
            let kind = child.kind();
            if kind != "start_tag" && kind != "end_tag" {
                self.process_child_node(child, source, fragments)?;
            }
        }
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::signal("body_end", true));
        self.add_raw_fragment("</body>");
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

        // Use custom template attribute if provided, otherwise auto-generate
        let fragment_id = self
            .get_element_attribute(node, "template", source)?
            .unwrap_or_else(|| self.id_counter.next_id("for"));
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

        // Skip the for fragment entirely if the body is empty
        if for_fragment.is_empty() {
            return Ok(());
        }

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

    /// Skipped attribute names for components.
    const SKIPPED_ATTRIBUTES: &[&str] = &["class", "style", "role"];
    /// Skipped attribute prefixes for components.
    const SKIPPED_ATTRIBUTE_PREFIXES: &[&str] = &["data-", "aria-"];

    fn is_skipped_attribute(name: &str) -> bool {
        Self::SKIPPED_ATTRIBUTES.contains(&name)
            || Self::SKIPPED_ATTRIBUTE_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
    }

    fn process_component_directive(
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

        // Emit "<tagname"
        self.add_raw_fragment(&format!("<{}", tag_name));

        // Process attributes — component-aware
        if let Some(tag_node) = tag_node {
            self.process_component_tag_attributes(tag_node, source, fragments)?;
        }

        if is_self_closing {
            let tag_text = tag_node
                .map(|n| &source[n.start_byte()..n.end_byte()])
                .unwrap_or("");
            if tag_text.ends_with("/>") {
                self.add_raw_fragment("/>");
            } else {
                self.add_raw_fragment(">");
            }
        } else {
            self.add_raw_fragment(">");
        }

        // Flush before component fragment
        self.flush_raw_buffer(fragments);

        // Get component data
        let (html_content, css_content) = {
            let component = self.component_registry.get(tag_name).ok_or_else(|| {
                ParserError::Directive(format!("Component not found: {}", tag_name))
            })?;
            (
                component.html_content.clone(),
                component.css_content.clone(),
            )
        };

        // Parse and register component template if not already done
        if !self.fragment_records.contains_key(tag_name) {
            let has_css = css_content.is_some();
            let css_path = if has_css {
                Some(format!("{}.css", tag_name))
            } else {
                None
            };
            let processed = self.process_component_template(&html_content, css_path.as_deref());
            self.parse(tag_name, &processed)?;
        }

        // Emit component fragment
        fragments.push(WebUIFragment::component(tag_name.to_string()));

        // Process slot content (skip start_tag/end_tag/self_closing_tag)
        if !is_self_closing {
            for child in node.named_children(&mut node.walk()) {
                let kind = child.kind();
                if kind != "start_tag" && kind != "end_tag" && kind != "self_closing_tag" {
                    self.process_child_node(child, source, fragments)?;
                }
            }
        }

        // Emit closing tag
        if !is_self_closing {
            self.add_raw_fragment(&format!("</{}>", tag_name));
        }

        Ok(())
    }

    fn process_component_tag_attributes(
        &mut self,
        tag_node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        // First pass: collect all attributes
        let mut attrs: Vec<(String, Option<String>)> = Vec::new();

        let mut cursor = tag_node.walk();
        for child in tag_node.named_children(&mut cursor) {
            if child.kind() != "attribute" {
                continue;
            }
            let name = self.get_attr_name(child, source)?;
            let value = self.get_attr_value(child, source);
            attrs.push((name, value));
        }

        // Second pass: emit attributes
        let mut first_dynamic_emitted = false;

        for (attr_name, attr_value) in &attrs {
            if let Some(bool_name) = attr_name.strip_prefix('?') {
                // Boolean attribute
                if let Some(val) = attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let condition = ConditionExpr::identifier(&signal_name);
                        let mut frag = WebUIFragment::attribute_boolean(bool_name, condition);
                        if !first_dynamic_emitted {
                            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) =
                                frag.fragment
                            {
                                a.attr_start = true;
                            }
                            first_dynamic_emitted = true;
                        }
                        self.add_fragment(frag, fragments);
                    }
                }
            } else if attr_name.starts_with(':') {
                // Complex attribute
                if let Some(val) = attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let mut frag = WebUIFragment::attribute_complex(attr_name, signal_name);
                        if !first_dynamic_emitted {
                            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) =
                                frag.fragment
                            {
                                a.attr_start = true;
                            }
                            first_dynamic_emitted = true;
                        }
                        self.add_fragment(frag, fragments);
                    }
                }
            } else if Self::is_skipped_attribute(attr_name) {
                // Skipped attribute — always emit as attribute with attrSkip
                if let Some(val) = attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name.clone(),
                                    value: signal_name,
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                    }
                }
            } else if let Some(val) = attr_value {
                if Self::contains_handlebars(val) {
                    // Dynamic regular attribute
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let mut frag = WebUIFragment::attribute(attr_name, signal_name);
                        if !first_dynamic_emitted {
                            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) =
                                frag.fragment
                            {
                                a.attr_start = true;
                            }
                            first_dynamic_emitted = true;
                        }
                        self.add_fragment(frag, fragments);
                    } else {
                        // Mixed attribute — template
                        let template_id = self.id_counter.next_id("attr");
                        let parsed = self.handlebars_parser.parse(val)?;
                        self.fragment_records
                            .insert(template_id.clone(), FragmentList { fragments: parsed });
                        let mut frag = WebUIFragment::attribute_template(attr_name, template_id);
                        if !first_dynamic_emitted {
                            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) =
                                frag.fragment
                            {
                                a.attr_start = true;
                            }
                            first_dynamic_emitted = true;
                        }
                        self.add_fragment(frag, fragments);
                    }
                } else {
                    // Static attribute on component — always rawValue
                    let mut frag = WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: attr_name.clone(),
                                value: val.clone(),
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    };
                    if !first_dynamic_emitted {
                        if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) = frag.fragment
                        {
                            a.attr_start = true;
                        }
                        first_dynamic_emitted = true;
                    }
                    self.add_fragment(frag, fragments);
                }
            } else {
                // Attribute without value
                self.add_raw_fragment(&format!(" {}", attr_name));
            }
        }

        Ok(())
    }

    /// Process component template HTML: wrap in shadow DOM template if needed,
    /// inject stylesheet link, and strip runtime-only attributes.
    fn process_component_template(&self, html: &str, styles: Option<&str>) -> String {
        let trimmed = html.trim();
        let has_template = trimmed.starts_with("<template");

        if has_template {
            // Strip @/:/?-prefixed attributes from the template tag
            let stripped = Self::strip_runtime_attrs_from_template(trimmed);
            if let Some(style_path) = styles {
                // Inject link after the first > in the template tag
                if let Some(pos) = stripped.find('>') {
                    let mut result = String::with_capacity(stripped.len() + style_path.len() + 50);
                    result.push_str(&stripped[..=pos]);
                    result.push_str(&format!(
                        "<link rel=\"stylesheet\" href=\"./{}\">",
                        style_path
                    ));
                    result.push_str(&stripped[pos + 1..]);
                    return result;
                }
            }
            stripped
        } else if let Some(style_path) = styles {
            format!(
                "<template shadowrootmode=\"open\"><link rel=\"stylesheet\" href=\"./{style_path}\">{trimmed}</template>"
            )
        } else {
            format!("<template shadowrootmode=\"open\">{trimmed}</template>")
        }
    }

    /// Strip attributes starting with @, :, or ? from the opening template tag.
    fn strip_runtime_attrs_from_template(html: &str) -> String {
        let Some(close_pos) = html.find('>') else {
            return html.to_string();
        };

        let tag_portion = &html[..close_pos];

        let Some(template_end) = tag_portion.find("template") else {
            return html.to_string();
        };
        let attr_start = template_end + "template".len();

        let before_attrs = &html[..attr_start];
        let attr_section = &html[attr_start..close_pos];
        let after_tag = &html[close_pos..];

        let mut result = String::with_capacity(html.len());
        result.push_str(before_attrs.trim_end());

        let mut i = 0;
        let bytes = attr_section.as_bytes();
        while i < bytes.len() {
            // Skip whitespace
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }

            // Start of attribute name
            let attr_start_pos = i;
            while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let attr_name = &attr_section[attr_start_pos..i];

            let is_runtime = attr_name.starts_with('@')
                || attr_name.starts_with(':')
                || attr_name.starts_with('?');

            // Read the value if present
            let mut attr_end = i;
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let quote = bytes[i];
                    i += 1;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                } else {
                    // Unquoted value (including {foo})
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                }
                attr_end = i;
            }

            if !is_runtime {
                result.push(' ');
                result.push_str(&attr_section[attr_start_pos..attr_end]);
            }
        }

        result.push_str(after_tag);
        result
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
        parser
            .component_registry
            .register_component(
                "my-component",
                "<div>My Component</div>",
                Some("div { color: blue; }"),
            )
            .expect("Failed to register component");

        let result = parser.parse("test.html", "<my-component></my-component>");
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = &records["test.html"].fragments;

        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<my-component>")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "my-component")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</my-component>")
        );

        // Component template stream should be wrapped with shadow DOM template + style link
        let comp = &records["my-component"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<template shadowrootmode=\"open\">") && raw.value.contains("<div>My Component</div>"))
        );
    }

    #[test]
    fn test_component_directive_with_slots() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(
                "my-component",
                "<div>My Component</div>",
                Some("div { color: blue; }"),
            )
            .expect("Failed to register component");

        let result = parser.parse(
            "test.html",
            "Hello<my-component><p>World</p></my-component>",
        );
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = &records["test.html"].fragments;

        // Entry: raw(Hello<my-component>) + component + raw(<p>World</p></my-component>)
        assert!(fragments.len() >= 3);
        // First fragment should contain "Hello" and "<my-component>"
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("Hello") && raw.value.contains("<my-component>"))
        );
        // Should have component fragment
        assert!(fragments.iter().any(|f| matches!(
            f.fragment.as_ref(),
            Some(Fragment::Component(c)) if c.fragment_id == "my-component"
        )));
        // Should end with closing tag
        let last = fragments.last().unwrap();
        assert!(
            matches!(last.fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</my-component>"))
        );
    }

    // ── Component template wrapping tests ────────────────────────────

    #[test]
    fn test_component_no_double_wrap_template() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(
                "custom-element",
                r#"<template foo="bar"><slot></slot></template>"#,
                None,
            )
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 3);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-element>")
        );
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-element")
        );
        assert!(
            matches!(entry[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "Hello</custom-element>")
        );

        let comp = &records["custom-element"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value == r#"<template foo="bar"><slot></slot></template>"#)
        );
    }

    #[test]
    fn test_component_styled_no_double_wrap() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(
                "custom-element",
                r#"<template foo="bar"><slot></slot></template>"#,
                Some("div { color: red; }"),
            )
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let comp = &records["custom-element"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value == r#"<template foo="bar"><link rel="stylesheet" href="./custom-element.css"><slot></slot></template>"#)
        );
    }

    #[test]
    fn test_component_strip_runtime_attrs() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(
                "custom-element",
                r#"<template @click={foo} :bar="baz" ?bool="true"><slot></slot></template>"#,
                None,
            )
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let comp = &records["custom-element"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value == "<template><slot></slot></template>")
        );
    }

    #[test]
    fn test_component_with_slots_and_attrs() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-element", "<slot></slot>", None)
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element appearance="subtle">Hello World</custom-element>"#,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 5);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-element")
        );
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if
                a.name == "appearance" && a.value == "subtle" && a.attr_start && a.raw_value)
        );
        assert!(matches!(entry[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">"));
        assert!(
            matches!(entry[3].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-element")
        );
        assert!(
            matches!(entry[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "Hello World</custom-element>")
        );
    }

    #[test]
    fn test_component_legacy_no_styles() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-element", "<div>Custom Element</div>", None)
            .expect("register");
        let result = parser.parse("index.html", "<custom-element></custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 3);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-element>")
        );
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-element")
        );
        assert!(
            matches!(entry[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</custom-element>")
        );

        let comp = &records["custom-element"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value == "<template shadowrootmode=\"open\"><div>Custom Element</div></template>")
        );
    }

    #[test]
    fn test_component_self_closing() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-widget", "<div>Widget Content</div>", None)
            .expect("register");
        let result = parser.parse("index.html", r#"<custom-widget config="{{settings}}" />"#);
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 4);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-widget")
        );
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if
                a.name == "config" && a.value == "settings" && a.attr_start)
        );
        assert!(
            matches!(entry[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>")
        );
        assert!(
            matches!(entry[3].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-widget")
        );
    }

    #[test]
    fn test_component_nested_self_closing_in_slot() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-icon", "<svg><slot></slot></svg>", None)
            .expect("register");
        let result = parser.parse(
            "index.html",
            r##"<custom-icon><use href="#icon-{{iconName}}" /></custom-icon>"##,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 5);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-icon>")
        );
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-icon")
        );
        assert!(
            matches!(entry[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<use")
        );
        assert!(
            matches!(entry[3].fragment.as_ref(), Some(Fragment::Attribute(a)) if a.name == "href" && a.template == "attr-1")
        );
        assert!(
            matches!(entry[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/></custom-icon>")
        );

        let comp = &records["custom-icon"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value == "<template shadowrootmode=\"open\"><svg><slot></slot></svg></template>")
        );
    }

    #[test]
    fn test_component_leading_boolean_attr_start() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-element", "<slot></slot>", None)
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element ?disabled="{{isDisabled}}" title="Hello"></custom-element>"#,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 6);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<custom-element")
        );
        // First dynamic attr: boolean with attrStart
        assert!(
            matches!(entry[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if
                a.name == "disabled" && a.attr_start &&
                matches!(a.condition_tree.as_ref().and_then(|c| c.expr.as_ref()),
                    Some(condition_expr::Expr::Identifier(id)) if id.value == "isDisabled"))
        );
        // Static attr after dynamic: rawValue
        assert!(
            matches!(entry[2].fragment.as_ref(), Some(Fragment::Attribute(a)) if
                a.name == "title" && a.value == "Hello" && a.raw_value)
        );
        assert!(matches!(entry[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == ">"));
        assert!(
            matches!(entry[4].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-element")
        );
        assert!(
            matches!(entry[5].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</custom-element>")
        );
    }

    #[test]
    fn test_component_meta_link_tags() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<head><meta charset="utf-8" /><link rel="stylesheet" href="{{cssFile}}" /></head>"#,
        );
        assert!(fragments.len() >= 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<head><meta charset=\"utf-8\"") && raw.value.contains("<link"))
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if a.name == "href" && a.value == "cssFile")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("/></head>"))
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

    // ── Body signal tests ─────────────────────────────────────────────

    #[test]
    fn test_body_signals() {
        let (fragments, _) = parse_and_get_fragments("<body><app-shell></app-shell></body>");
        assert_eq!(fragments.len(), 5);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<body>")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(signal)) if signal.value == "body_start" && signal.raw)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<app-shell></app-shell>")
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(signal)) if signal.value == "body_end" && signal.raw)
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</body>")
        );
    }

    // ── Empty for handling tests ──────────────────────────────────────

    #[test]
    fn test_empty_for_produces_nothing() {
        let (fragments, records) =
            parse_and_get_fragments(r#"<div><for each="item in items"></for></div>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<div></div>")
        );
        assert!(!records.contains_key("for-1"));
    }

    // ── Self-closing / void element tests ─────────────────────────────

    #[test]
    fn test_self_closing_svg_path() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#)
        );
    }

    #[test]
    fn test_html5_void_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#,
        );
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#)
        );
    }

    #[test]
    fn test_self_closing_with_dynamic_attributes() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="{{imageUrl}}" alt="{{imageAlt}}" />"#);
        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<img")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "src" && attr.value == "imageUrl")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "alt" && attr.value == "imageAlt")
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>")
        );
    }

    #[test]
    fn test_self_closing_with_boolean_attributes() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<input type="checkbox" ?checked="{{isSelected}}" ?disabled="{{isDisabled}}" />"#,
        );
        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<input type=\"checkbox\"")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "checked" && matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()), Some(condition_expr::Expr::Identifier(id)) if id.value == "isSelected"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "disabled" && matches!(attr.condition_tree.as_ref().and_then(|c| c.expr.as_ref()), Some(condition_expr::Expr::Identifier(id)) if id.value == "isDisabled"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>")
        );
    }

    #[test]
    fn test_multiple_self_closing_in_sequence() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="1.jpg" /><br /><img src="2.jpg" />"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<img src="1.jpg"/><br/><img src="2.jpg"/>"#)
        );
    }

    #[test]
    fn test_self_closing_with_mixed_content() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div>Text before<img src="{{url}}" />Text after</div>"#);
        assert_eq!(fragments.len(), 3);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<div>Text before<img")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "src" && attr.value == "url")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>Text after</div>")
        );
    }

    #[test]
    fn test_self_closing_svg_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<svg><circle cx="{{x}}" cy="{{y}}" r="5" /><rect width="10" height="10" /></svg>"#,
        );
        assert_eq!(fragments.len(), 4);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<svg><circle")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "cx" && attr.value == "x")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "cy" && attr.value == "y")
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#" r="5"/><rect width="10" height="10"/></svg>"#)
        );
    }

    #[test]
    fn test_self_closing_inside_for_loop() {
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items"><img src="{{item.url}}" /></for>"#,
        );
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.item == "item" && fl.collection == "items" && fl.fragment_id == "for-1")
        );
        let for_stream = records.get("for-1").expect("Missing for-1");
        assert_eq!(for_stream.fragments.len(), 3);
        assert!(
            matches!(for_stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<img")
        );
        assert!(
            matches!(for_stream.fragments[1].fragment.as_ref(), Some(Fragment::Attribute(attr)) if attr.name == "src" && attr.value == "item.url")
        );
        assert!(
            matches!(for_stream.fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "/>")
        );
    }

    #[test]
    fn test_self_closing_whitespace_variations() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="test.jpg"/><input type="text" /><br/>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<img src="test.jpg"/><input type="text"/><br/>"#)
        );
    }

    #[test]
    fn test_deeply_nested_self_closing() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><section><article><img src="deep.jpg" /><br /></article></section></div>"#,
        );
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<div><section><article><img src="deep.jpg"/><br/></article></section></div>"#)
        );
    }

    #[test]
    fn test_self_closing_vs_empty_regular_tags() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div></div><img src="test.jpg" /><span></span>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == r#"<div></div><img src="test.jpg"/><span></span>"#)
        );
    }

    // ── Feature 1: Custom template attribute on <for> ────────────────────

    #[test]
    fn test_for_custom_template_attribute() {
        // Port of: 'should process transient node for with template'
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items" template="static"><span>Item</span></for>"#,
        );
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::ForLoop(fl)) if
            fl.item == "item" && fl.collection == "items" && fl.fragment_id == "static")
        );
        let stream = records.get("static").expect("Missing 'static' stream");
        assert_eq!(stream.fragments.len(), 1);
        assert!(
            matches!(stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<span>Item</span>")
        );
    }

    #[test]
    fn test_for_recursive_template() {
        // Port of: 'should process recursive transient nodes'
        let mut parser = HtmlParser::new();
        let html = r#"<for template="static" each="outerItem in outerItems"><div><span>{{outerItem.name}}</span><for template="static" each="innerItem in innerItems" /></div></for>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let entry = &records["index.html"].fragments;
        assert_eq!(entry.len(), 1);
        assert!(
            matches!(entry[0].fragment.as_ref(), Some(Fragment::ForLoop(fl)) if
            fl.item == "outerItem" && fl.collection == "outerItems" && fl.fragment_id == "static")
        );
        let static_stream = records.get("static").expect("Missing 'static' stream");
        assert_eq!(static_stream.fragments.len(), 5);
        assert!(
            matches!(static_stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<div><span>")
        );
        assert!(
            matches!(static_stream.fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "outerItem.name")
        );
        assert!(
            matches!(static_stream.fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</span>")
        );
        assert!(
            matches!(static_stream.fragments[3].fragment.as_ref(), Some(Fragment::ForLoop(fl)) if
            fl.item == "innerItem" && fl.collection == "innerItems" && fl.fragment_id == "static")
        );
        assert!(
            matches!(static_stream.fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "</div>")
        );
    }

    // ── Feature 2: <if> / <for> with multiple children ──────────────────

    #[test]
    fn test_if_multiple_children() {
        // Port of: 'should handle <if> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<if condition="valid"><p>hello</p><p>world</p></if>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::IfCond(ic)) if ic.fragment_id == "if-1")
        );
        let if_stream = records.get("if-1").expect("Missing if-1");
        assert_eq!(if_stream.fragments.len(), 1);
        assert!(
            matches!(if_stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<p>hello</p><p>world</p>")
        );
    }

    #[test]
    fn test_for_multiple_children() {
        // Port of: 'should handle <for> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<for each="item in items"><p>hello</p><p>world</p></for>"#);
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.fragment_id == "for-1")
        );
        let for_stream = records.get("for-1").expect("Missing for-1");
        assert_eq!(for_stream.fragments.len(), 1);
        assert!(
            matches!(for_stream.fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<p>hello</p><p>world</p>")
        );
    }

    // ── Feature 3: Handlebars at beginning/end of text ──────────────────

    #[test]
    fn test_handlebars_at_beginning() {
        // Port of: 'should process handlebars from text at beginning'
        let (fragments, _) = parse_and_get_fragments("{{first}}");
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "first")
        );
    }

    #[test]
    fn test_handlebars_at_beginning_and_raw() {
        // Port of: 'should process handlebars from text at beginning and raw'
        let (fragments, _) = parse_and_get_fragments("{{first}}test");
        assert_eq!(fragments.len(), 2);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "first")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "test")
        );
    }

    #[test]
    fn test_handlebars_raw_and_end() {
        // Port of: 'should process handlebars from text at raw and end'
        let (fragments, _) = parse_and_get_fragments("test{{first}}");
        assert_eq!(fragments.len(), 2);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "test")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "first")
        );
    }

    // ── Feature 4: Handlebars edge cases ────────────────────────────────

    #[test]
    fn test_handlebars_invalid_triple_open() {
        // Port of: 'should not process handlebars when invalid'
        let (fragments, _) = parse_and_get_fragments("{{{invalid}}");
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "{{{invalid}}")
        );
    }

    #[test]
    fn test_handlebars_four_open_braces() {
        // Port of: 'should not process handlebars when invalid since triple exists'
        let (fragments, _) = parse_and_get_fragments("{{{{invalid}}");
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "{{{{invalid}}")
        );
    }

    #[test]
    fn test_handlebars_five_open_with_valid_double() {
        // Port of: 'should not process handlebars when invalid but with valid triple'
        let (fragments, _) = parse_and_get_fragments("{{{{{invalid}}");
        assert_eq!(fragments.len(), 2);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "{{{")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "invalid")
        );
    }

    #[test]
    fn test_entities_preserved() {
        // Port of: 'should process entities correctly'
        let (fragments, _) = parse_and_get_fragments("<p>Hello&#125;World</p>");
        assert_eq!(fragments.len(), 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<p>Hello&#125;World</p>")
        );
    }

    // ── Feature 5: DOCTYPE handling ─────────────────────────────────────

    #[test]
    fn test_doctype_preserved() {
        // DOCTYPE should be preserved as raw content
        let (fragments, _) = parse_and_get_fragments("<!DOCTYPE html><html><head></head></html>");
        assert!(fragments.len() >= 1);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<!DOCTYPE html>"))
        );
    }
}
