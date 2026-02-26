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

/// Strategy for how component CSS is delivered in rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CssStrategy {
    /// Emit `<link rel="stylesheet" href="./component.css">` tags (default).
    #[default]
    External,
    /// Embed CSS content inline in `<style>` tags within the shadow DOM template.
    Inline,
}

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

    /// How component CSS is delivered in output.
    css_strategy: CssStrategy,
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
            css_strategy: CssStrategy::default(),
            parser,
        }
    }

    /// Set the CSS strategy for component stylesheet delivery.
    pub fn set_css_strategy(&mut self, strategy: CssStrategy) -> &mut Self {
        self.css_strategy = strategy;
        self
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
            self.process_tag_attributes(tag_node, source, fragments, false)?;

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
    /// For component elements (`is_component = true`), all attributes become
    /// fragments and the first non-skipped attribute is marked with `attr_start`.
    /// For regular elements, only dynamic attributes become fragments while
    /// static attributes are accumulated into the raw buffer.
    fn process_tag_attributes(
        &mut self,
        tag_node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
        is_component: bool,
    ) -> Result<()> {
        let mut first_dynamic_emitted = false;
        let mut cursor = tag_node.walk();

        for child in tag_node.named_children(&mut cursor) {
            if child.kind() != "attribute" {
                continue;
            }

            let attr_name = self.get_attr_name(child, source)?;
            let attr_value = self.get_attr_value(child, source);

            if let Some(bool_name) = attr_name.strip_prefix('?') {
                // Boolean attribute: ?disabled={{isDisabled}}
                if is_component {
                    if let Some(val) = &attr_value {
                        if let Some(signal_name) = Self::extract_single_handlebars(val) {
                            let condition = ConditionExpr::identifier(&signal_name);
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute_boolean(bool_name, condition),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                        }
                    }
                } else {
                    self.process_boolean_attribute(bool_name, attr_value.as_deref(), fragments)?;
                }
            } else if attr_name.starts_with(':') {
                // Complex attribute: :config="{{settings}}"
                if is_component {
                    if let Some(val) = &attr_value {
                        if let Some(signal_name) = Self::extract_single_handlebars(val) {
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute_complex(&attr_name, signal_name),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                        }
                    }
                } else {
                    self.process_complex_attribute(&attr_name, attr_value.as_deref(), fragments)?;
                }
            } else if is_component && Self::is_skipped_attribute(&attr_name) {
                // Skipped component attribute (class, style, role, data-*, aria-*)
                if let Some(val) = &attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name,
                                    value: signal_name,
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                    }
                }
            } else if let Some(ref val) = attr_value {
                if Self::contains_handlebars(val) {
                    if is_component {
                        if let Some(signal_name) = Self::extract_single_handlebars(val) {
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute(&attr_name, signal_name),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                        } else {
                            let template_id = self.id_counter.next_id("attr");
                            let parsed = self.handlebars_parser.parse(val)?;
                            self.fragment_records
                                .insert(template_id.clone(), FragmentList { fragments: parsed });
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute_template(&attr_name, template_id),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                        }
                    } else {
                        self.process_dynamic_attribute(&attr_name, val, fragments)?;
                    }
                } else if is_component {
                    // Static attribute on component → rawValue fragment
                    let frag = Self::maybe_mark_attr_start(
                        WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name,
                                    value: val.clone(),
                                    raw_value: true,
                                    ..Default::default()
                                },
                            )),
                        },
                        &mut first_dynamic_emitted,
                    );
                    self.add_fragment(frag, fragments);
                } else {
                    // Static attribute on regular element → raw text
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

    /// Set `attr_start = true` on the first non-skipped attribute fragment for
    /// a component element.
    fn maybe_mark_attr_start(
        mut frag: WebUIFragment,
        first_dynamic_emitted: &mut bool,
    ) -> WebUIFragment {
        if !*first_dynamic_emitted {
            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) = frag.fragment {
                a.attr_start = true;
            }
            *first_dynamic_emitted = true;
        }
        frag
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
            self.process_tag_attributes(tag_node, source, fragments, true)?;
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
            let css_injection = match self.css_strategy {
                CssStrategy::External => {
                    if css_content.is_some() {
                        Some(format!(
                            "<link rel=\"stylesheet\" href=\"./{}.css\">",
                            tag_name
                        ))
                    } else {
                        None
                    }
                }
                CssStrategy::Inline => css_content
                    .as_ref()
                    .map(|css| format!("<style>{}</style>", css.trim())),
            };
            let processed =
                self.process_component_template(&html_content, css_injection.as_deref());
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

    /// Process component template HTML: wrap in shadow DOM template if needed,
    /// inject CSS snippet (link or inline style), and strip runtime-only attributes.
    fn process_component_template(&mut self, html: &str, css_snippet: Option<&str>) -> String {
        let trimmed = html.trim();
        let has_template = trimmed.starts_with("<template");

        if has_template {
            // Strip @/:/?-prefixed attributes from the template tag
            let stripped = self.strip_runtime_attrs_from_template(trimmed);
            if let Some(snippet) = css_snippet {
                // Inject CSS snippet after the first > in the template tag
                if let Some(pos) = stripped.find('>') {
                    let mut result = String::with_capacity(stripped.len() + snippet.len() + 16);
                    result.push_str(&stripped[..=pos]);
                    result.push_str(snippet);
                    result.push_str(&stripped[pos + 1..]);
                    return result;
                }
            }
            stripped
        } else if let Some(snippet) = css_snippet {
            format!("<template shadowrootmode=\"open\">{snippet}{trimmed}</template>")
        } else {
            format!("<template shadowrootmode=\"open\">{trimmed}</template>")
        }
    }

    /// Strip attributes starting with @, :, or ? from the opening template tag.
    /// Uses tree-sitter to parse the tag and reconstruct it without runtime attrs.
    fn strip_runtime_attrs_from_template(&mut self, html: &str) -> String {
        let tree = match self.parser.parse(html, None) {
            Some(t) => t,
            None => return html.to_string(),
        };

        // Find the first start_tag in the tree
        let root = tree.root_node();
        let start_tag = Self::find_first_node(root, "start_tag");
        let Some(tag) = start_tag else {
            return html.to_string();
        };

        // Collect byte ranges of runtime attributes to remove
        let mut removals: Vec<(usize, usize)> = Vec::new();
        let mut cursor = tag.walk();
        for child in tag.named_children(&mut cursor) {
            if child.kind() == "attribute" {
                let name_node = child.child(0);
                if let Some(name) = name_node {
                    let attr_name = &html[name.start_byte()..name.end_byte()];
                    if attr_name.starts_with('@')
                        || attr_name.starts_with(':')
                        || attr_name.starts_with('?')
                    {
                        // Remove the attribute and any leading whitespace
                        let mut start = child.start_byte();
                        while start > 0 && html.as_bytes()[start - 1] == b' ' {
                            start -= 1;
                        }
                        removals.push((start, child.end_byte()));
                    }
                }
            }
        }

        if removals.is_empty() {
            return html.to_string();
        }

        // Rebuild the string, skipping removed ranges
        let mut result = String::with_capacity(html.len());
        let mut pos = 0;
        for (start, end) in &removals {
            result.push_str(&html[pos..*start]);
            pos = *end;
        }
        result.push_str(&html[pos..]);
        result
    }

    /// Find the first node of a given kind in the tree (iterative BFS).
    fn find_first_node<'a>(root: Node<'a>, kind: &str) -> Option<Node<'a>> {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == kind {
                return Some(node);
            }
            let mut cursor = node.walk();
            // Push children in reverse so first child is processed first
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
