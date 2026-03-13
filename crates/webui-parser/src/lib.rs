//! Directive parser for WebUI template directives.
//!
//! This module handles parsing WebUI-specific directives like <for>, <if>, etc.
mod component_registry;
mod condition_parser;
mod css_parser;
mod error;
mod handlebars_parser;
pub mod plugin;
mod route_parser;

pub use component_registry::{Component, ComponentRegistry};
pub use condition_parser::ConditionParser;
pub use css_parser::CssParser;
pub use error::{ParserError, Result};
pub use handlebars_parser::HandlebarsParser;

use crate::plugin::ParserPlugin;
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIteratorMut};
use tree_sitter_html::LANGUAGE;
use webui_protocol::{
    web_ui_fragment, web_ui_fragment::Fragment, ConditionExpr, FragmentList, RouteRecord,
    WebUIFragment, WebUIFragmentAttribute, WebUIFragmentRecords, WebUiFragmentRoute,
};

/// Strategy for how component CSS is delivered in rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum CssStrategy {
    /// Emit `<link rel="stylesheet" href="/component.css">` tags (default).
    #[default]
    Link,
    /// Embed CSS content in `<style>` tags within the shadow DOM template.
    Style,
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

    /// Optional parser plugin for framework-specific behavior.
    plugin: Option<Box<dyn ParserPlugin>>,

    /// Accumulated CSS custom property token names from all processed
    /// components and inline `<style>` tags.
    token_store: HashSet<String>,

    /// CSS custom property names **defined** in inline `<style>` tags
    /// (e.g., `:root { --color-primary: #0078d4; }`). These are excluded
    /// from the final token set since the app already provides their values.
    token_definitions: HashSet<String>,

    /// Collected route fragments for the top-level registry.
    route_fragments: Vec<WebUiFragmentRoute>,

    /// Route name uniqueness tracker.
    route_name_registry: route_parser::RouteNameRegistry,
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
            plugin: None,
            token_store: HashSet::new(),
            token_definitions: HashSet::new(),
            route_fragments: Vec::new(),
            route_name_registry: route_parser::RouteNameRegistry::new(),
            parser,
        }
    }

    /// Create a new parser with a plugin for framework-specific behavior.
    pub fn with_plugin(plugin: Box<dyn ParserPlugin>) -> Self {
        let mut p = Self::new();
        p.plugin = Some(plugin);
        p
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

    /// Take the parser plugin. Call before `into_fragment_records()` if you need
    /// access to plugin state (e.g., component templates).
    pub fn take_plugin(&mut self) -> Option<Box<dyn ParserPlugin>> {
        self.plugin.take()
    }

    /// Get a mutable reference to the parser plugin.
    pub fn plugin_mut(&mut self) -> Option<&mut dyn ParserPlugin> {
        self.plugin.as_deref_mut()
    }

    /// Take the accumulated CSS tokens as a sorted, deduplicated `Vec`.
    ///
    /// Tokens that are **defined** in inline `<style>` tags (e.g., in a
    /// `:root` block) are excluded — only externally-referenced tokens
    /// that the app does not already define are returned.
    ///
    /// This consumes the internal token store. Call after parsing is complete.
    #[must_use]
    pub fn take_tokens(&mut self) -> Vec<String> {
        let definitions = std::mem::take(&mut self.token_definitions);
        let mut tokens: Vec<String> = std::mem::take(&mut self.token_store)
            .into_iter()
            .filter(|t| !definitions.contains(t))
            .collect();
        tokens.sort();
        tokens
    }

    /// Take the collected route registry as a map keyed by route name (or fragment ID).
    ///
    /// Call after parsing is complete.
    #[must_use]
    pub fn take_routes(&mut self) -> HashMap<String, RouteRecord> {
        let routes = std::mem::take(&mut self.route_fragments);
        route_parser::collect_route_registry(&routes)
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
                    "route" => return self.process_route_directive(node, source, fragments),
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

                        // Single parse: extract both token usages and definitions
                        if let Ok((tokens, defs)) = self
                            .css_parser
                            .extract_tokens_and_definitions(style_content)
                        {
                            self.token_store.extend(tokens);
                            self.token_definitions.extend(defs);
                        }

                        // Add the style tag with processed CSS
                        let style_tag = format!("<style>{}</style>", processed_css);
                        self.add_raw_fragment(&style_tag);
                    }
                }
            }
            "text" | "raw_text" => {
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
            // HTML comment: check for handlebars bindings like <!--{{tokens}}-->
            "comment" => {
                let content = &source[node.start_byte()..node.end_byte()];
                self.process_comment(content, fragments)?;
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

    /// Check whether a boolean attribute (no value) exists on an element.
    /// Returns `true` for `<el attr>` or `<el attr="...">`.
    fn has_element_attribute(&self, node: Node, attr_name: &str, source: &str) -> Result<bool> {
        // First check for valued attributes
        if self
            .get_element_attribute(node, attr_name, source)?
            .is_some()
        {
            return Ok(true);
        }

        // Check for boolean (valueless) attributes via tree-sitter query
        let query_str = r#"
            (element
              [
                (start_tag
                  (attribute
                    (attribute_name) @name))
                (self_closing_tag
                  (attribute
                    (attribute_name) @name))
              ]
            )
        "#;

        let query = Query::new(&LANGUAGE.into(), query_str)
            .map_err(|e| ParserError::Html(format!("Failed to create attribute query: {:?}", e)))?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, node, source.as_bytes());
        while let Some(m) = matches.next_mut() {
            for capture in m.captures.iter() {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "name" {
                    let name_text = capture.node.utf8_text(source.as_bytes()).map_err(|_| {
                        ParserError::Html("Invalid UTF-8 for attribute name".to_string())
                    })?;
                    if name_text == attr_name {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
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
            let binding_count = self.process_tag_attributes(tag_node, source, fragments, false)?;
            if let Some(ref mut p) = self.plugin {
                if let Some(data) = p.on_element_parsed(binding_count) {
                    self.add_fragment(WebUIFragment::plugin(data), fragments);
                }
            }

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
    /// Returns the number of binding (dynamic) attributes found.
    fn process_tag_attributes(
        &mut self,
        tag_node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
        is_component: bool,
    ) -> Result<u32> {
        let mut first_dynamic_emitted = false;
        let mut binding_count: u32 = 0;
        let mut cursor = tag_node.walk();

        for child in tag_node.named_children(&mut cursor) {
            if child.kind() != "attribute" {
                continue;
            }

            let attr_name = self.get_attr_name(child, source)?;

            // Let plugin skip framework-specific attributes (but still count them
            // for binding attribute tracking — FAST-HTML creates factories for these)
            if let Some(ref p) = self.plugin {
                if p.should_skip_attribute(&attr_name) {
                    binding_count += 1;
                    continue;
                }
            }

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
                            binding_count += 1;
                        }
                    }
                } else {
                    self.process_boolean_attribute(bool_name, attr_value.as_deref(), fragments)?;
                    binding_count += 1;
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
                            binding_count += 1;
                        }
                    }
                } else {
                    self.process_complex_attribute(&attr_name, attr_value.as_deref(), fragments)?;
                    binding_count += 1;
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
                        binding_count += 1;
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
                            binding_count += 1;
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
                            binding_count += 1;
                        }
                    } else {
                        self.process_dynamic_attribute(&attr_name, val, fragments)?;
                        binding_count += 1;
                    }
                } else if is_component {
                    // Static attribute on component → rawValue fragment.
                    // Not counted as a binding attribute.
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
        Ok(binding_count)
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
            if let Some(expr_str) = Self::extract_single_handlebars(val) {
                // Parse as a full condition expression (supports predicates like page == 'dashboard')
                let condition = self
                    .condition_parser
                    .parse(&expr_str)
                    .unwrap_or_else(|_| ConditionExpr::identifier(&expr_str));
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
        // Let plugin inject content before body_end
        if let Some(ref mut p) = self.plugin {
            if let Some(body_end_content) = p.on_body_end() {
                fragments.push(WebUIFragment::raw(body_end_content));
            }
        }
        fragments.push(WebUIFragment::signal("body_end", true));
        self.add_raw_fragment("</body>");
        Ok(())
    }

    /// Process an HTML comment node.
    ///
    /// If the comment contains handlebars expressions (e.g., `<!--{{tokens}}-->`),
    /// parse them as signal fragments. Otherwise, preserve the comment as raw content.
    fn process_comment(&mut self, content: &str, fragments: &mut Vec<WebUIFragment>) -> Result<()> {
        // Strip <!-- and --> delimiters to get the inner text
        let inner = content
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            .unwrap_or("");

        let trimmed = inner.trim();

        // Quick check: if no handlebars syntax, preserve as raw comment
        if !trimmed.contains("{{") {
            self.add_raw_fragment(content);
            return Ok(());
        }

        // Parse the inner text through the handlebars parser
        let parsed = self.handlebars_parser.parse(trimmed)?;

        // Check if the result is *only* signal fragments (no raw text mixed in).
        // If all fragments are signals, emit them without comment delimiters.
        // If there's any raw text mixed in, preserve the whole comment as-is.
        let all_signals = parsed
            .iter()
            .all(|f| matches!(f.fragment.as_ref(), Some(Fragment::Signal(_))));

        if all_signals && !parsed.is_empty() {
            for fragment in parsed {
                self.add_fragment(fragment, fragments);
            }
        } else {
            // Mixed content or parse result has raw parts — keep as raw comment
            self.add_raw_fragment(content);
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

    /// Process a `<route>` directive.
    ///
    /// Emits a `Fragment::Route` protocol fragment. The handler renders
    /// `<webui-route>` elements with server-side route matching — matched
    /// routes get `active` + component content, non-matched get `display:none`.
    fn process_route_directive(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        // Extract route attributes
        let path = self
            .get_element_attribute(node, "path", source)?
            .unwrap_or_default();

        let component = self
            .get_element_attribute(node, "component", source)?
            .unwrap_or_default();

        let name = self
            .get_element_attribute(node, "name", source)?
            .unwrap_or_default();

        let exact = self.has_element_attribute(node, "exact", source)?;

        let attrs = route_parser::RouteAttributes {
            path: path.clone(),
            component: component.clone(),
            name: name.clone(),
            exact,
        };

        // Validate attributes (component is required)
        route_parser::validate_attributes(&attrs)?;

        // Register route name for uniqueness
        self.route_name_registry.register(&name)?;

        // Extract params from path template (validation only)
        route_parser::extract_params(&path)?;

        // Ensure the component's template is parsed and registered
        if !component.is_empty()
            && self.component_registry.contains(&component)
            && !self.fragment_records.contains_key(&component)
        {
            if let Some(ref mut p) = self.plugin {
                if let Some(comp) = self.component_registry.get(&component) {
                    p.on_parse_component(&component, comp)?;
                }
            }
            let (html_content, css_content, css_tokens) = {
                let comp = self.component_registry.get(&component).ok_or_else(|| {
                    crate::error::ParserError::Directive(format!(
                        "Component not found: {component}"
                    ))
                })?;
                (
                    comp.html_content.clone(),
                    comp.css_content.clone(),
                    comp.css_tokens.clone(),
                )
            };
            self.token_store.extend(css_tokens);
            let css_injection = match self.css_strategy {
                CssStrategy::Link => {
                    if css_content.is_some() {
                        Some(format!(
                            "<link rel=\"stylesheet\" href=\"/{component}.css\">"
                        ))
                    } else {
                        None
                    }
                }
                CssStrategy::Style => css_content
                    .as_ref()
                    .map(|css| format!("<style>{}</style>", css.trim())),
            };
            let processed =
                self.process_component_template(&html_content, css_injection.as_deref());
            let saved_buffer = std::mem::take(&mut self.raw_buffer);
            self.parse(&component, &processed)?;
            self.raw_buffer = saved_buffer;
        }

        // Flush any pending raw content before the route fragment
        self.flush_raw_buffer(fragments);

        // Build route metadata for the registry
        let route_fragment = route_parser::build_route_fragment(&attrs, component.clone());

        // Emit Fragment::Route — the handler renders it as <webui-route>
        fragments.push(WebUIFragment::route_from(route_fragment.clone()));

        // Track for registry
        self.route_fragments.push(route_fragment);

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
            let binding_count = self.process_tag_attributes(tag_node, source, fragments, true)?;
            if let Some(ref mut p) = self.plugin {
                if let Some(data) = p.on_element_parsed(binding_count) {
                    self.add_fragment(WebUIFragment::plugin(data), fragments);
                }
            }
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
        let (html_content, css_content, css_tokens) = {
            let component = self.component_registry.get(tag_name).ok_or_else(|| {
                ParserError::Directive(format!("Component not found: {}", tag_name))
            })?;
            (
                component.html_content.clone(),
                component.css_content.clone(),
                component.css_tokens.clone(),
            )
        };

        // Merge component CSS tokens into the global token store
        self.token_store.extend(css_tokens);

        // Parse and register component template if not already done
        if !self.fragment_records.contains_key(tag_name) {
            // Notify plugin about component (only on first encounter)
            if let Some(ref mut p) = self.plugin {
                if let Some(component) = self.component_registry.get(tag_name) {
                    p.on_parse_component(tag_name, component)?;
                }
            }

            let css_injection = match self.css_strategy {
                CssStrategy::Link => {
                    if css_content.is_some() {
                        Some(format!(
                            "<link rel=\"stylesheet\" href=\"/{}.css\">",
                            tag_name
                        ))
                    } else {
                        None
                    }
                }
                CssStrategy::Style => css_content
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
mod tests {
    use webui_test_utils::*;

    use super::*;

    #[test]
    fn test_parse_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{name}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [raw("Hello, "), signal("name"), raw("!"),]
        );
    }

    #[test]
    fn test_parse_raw_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{{html_content}}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [raw("Hello, "), signal_raw("html_content"), raw("!"),]
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

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("item", "items", "for-1"),]
        );

        // Verify the sub-fragment contains our item content
        assert_stream!(
            fragment_records,
            "for-1",
            [
                raw("<div class=\"item\">"),
                signal("item.name"),
                raw("</div>"),
            ]
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

        assert_stream!(fragment_records, "test.html", [if_cond("if-1"),]);

        // Verify the sub-fragment contains our content
        assert_stream!(
            fragment_records,
            "if-1",
            [
                raw("<div>Welcome back, "),
                signal("username"),
                raw("!</div>"),
            ]
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

        assert_stream!(
            records,
            "test.html",
            [
                raw("<my-component>"),
                component("my-component"),
                raw("</my-component>"),
            ]
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

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element>"),
                component("custom-element"),
                raw("Hello</custom-element>"),
            ]
        );

        assert_stream!(
            records,
            "custom-element",
            [raw(r#"<template foo="bar"><slot></slot></template>"#),]
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

        assert_stream!(
            records,
            "custom-element",
            [raw(
                r#"<template foo="bar"><link rel="stylesheet" href="/custom-element.css"><slot></slot></template>"#
            ),]
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

        assert_stream!(
            records,
            "custom-element",
            [raw("<template><slot></slot></template>"),]
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
        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                attr_raw_start("appearance", "subtle"),
                raw(">"),
                component("custom-element"),
                raw("Hello World</custom-element>"),
            ]
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

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element>"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
        );

        assert_stream!(
            records,
            "custom-element",
            [raw(
                "<template shadowrootmode=\"open\"><div>Custom Element</div></template>"
            ),]
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

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-widget"),
                attr_start("config", "settings"),
                raw("/>"),
                component("custom-widget"),
            ]
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

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-icon>"),
                component("custom-icon"),
                raw("<use"),
                attr_template("href", "attr-1"),
                raw("/></custom-icon>"),
            ]
        );

        assert_stream!(
            records,
            "custom-icon",
            [raw(
                "<template shadowrootmode=\"open\"><svg><slot></slot></svg></template>"
            ),]
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

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                // First dynamic attr: boolean with attrStart
                bool_attr_start("disabled", "isDisabled"),
                // Static attr after dynamic: rawValue
                attr_raw("title", "Hello"),
                raw(">"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
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

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("category", "categories", "for-1"),]
        );

        assert_stream!(fragment_records, "for-1", [if_cond("if-1"),]);

        assert_stream!(
            fragment_records,
            "if-1",
            [for_loop("item", "category.items", "for-2"),]
        );

        assert_stream!(fragment_records, "for-2", [signal("item.title"),]);
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

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("category", "categories", "for-1"),]
        );

        // Verify for fragments contains the category.name signal
        assert_stream!(
            fragment_records,
            "for-1",
            [
                raw("<div class=\"category\"><h2>"),
                signal("category.name"),
                raw("</h2>"),
                if_cond("if-1"),
                raw("</div>"),
            ]
        );

        // Verify nested if condition.
        assert_stream!(
            fragment_records,
            "if-1",
            [
                raw("<ul>"),
                for_loop("item", "category.items", "for-2"),
                raw("</ul>"),
            ]
        );

        // Verify nested for each.
        assert_stream!(
            fragment_records,
            "for-2",
            [raw("<li>"), signal("item.title"), raw("</li>"),]
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
        let (fragments, _) = parse_and_get_fragments(r#"<a href="{{url}}">{{name}}</a>"#);
        assert_fragments!(
            fragments,
            [
                raw("<a"),
                attr("href", "url"),
                raw(">"),
                signal("name"),
                raw("</a>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_with_handlebars() {
        // Port of: 'should process boolean attribute with handlebars expression'
        let (fragments, _) =
            parse_and_get_fragments("<button ?disabled={{isDisabled}}>Click</button>");
        assert_fragments!(
            fragments,
            [
                raw("<button"),
                bool_attr("disabled", "isDisabled"),
                raw(">Click</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_multiple_boolean() {
        // Port of: 'should process multiple boolean attributes'
        // <input ?checked={{isChecked}} ?disabled={{isDisabled}} />
        let (fragments, _) =
            parse_and_get_fragments("<input ?checked={{isChecked}} ?disabled={{isDisabled}} />");

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                bool_attr("checked", "isChecked"),
                bool_attr("disabled", "isDisabled"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_and_regular_together() {
        // Port of: 'should process a boolean attribute and a regular attribute together'
        // <input ?checked="{{isChecked}}" type="checkbox">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input ?checked="{{isChecked}}" type="checkbox">Hi</input>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                bool_attr("checked", "isChecked"),
                raw(" type=\"checkbox\">Hi</input>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_sandwiched() {
        // Port of: 'should process a boolean attribute sandwiched between regular attributes'
        // <input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input version={{edition}} ?checked="{{isChecked}}" type="checkbox">Hi</input>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                attr("version", "edition"),
                bool_attr("checked", "isChecked"),
                raw(" type=\"checkbox\">Hi</input>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_ending() {
        // Port of: 'should process html ending with boolean attribute correctly'
        // <input version={{edition}} ?checked="{{isChecked}}">Hi</input>
        let (fragments, _) = parse_and_get_fragments(
            r#"<input version={{edition}} ?checked="{{isChecked}}">Hi</input>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                attr("version", "edition"),
                bool_attr("checked", "isChecked"),
                raw(">Hi</input>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_dotted_path() {
        // Port of: 'should process boolean attribute with dotted path'
        // <div ?checked={{layout.isPinned}}>Content</div>
        let (fragments, _) =
            parse_and_get_fragments("<div ?checked={{layout.isPinned}}>Content</div>");

        assert_fragments!(
            fragments,
            [
                raw("<div"),
                bool_attr("checked", "layout.isPinned"),
                raw(">Content</div>"),
            ]
        );
    }

    #[test]
    fn test_attribute_colon_prefixed_complex() {
        // Port of: 'should process colon-prefixed attribute with handlebars'
        // <my-component :config="{{settings}}"></my-component>
        let (fragments, _) =
            parse_and_get_fragments(r#"<my-component :config="{{settings}}"></my-component>"#);

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex(":config", "settings"),
                raw("></my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_multiple_colon_prefixed() {
        // Port of: 'should process multiple colon-prefixed complex attributes'
        // <my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
        let (fragments, _) = parse_and_get_fragments(
            r#"<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex(":prop1", "val1"),
                attr_complex(":prop2", "val2"),
                raw("></my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_mixed_normal_boolean_colon() {
        // Port of: 'should process mixed normal, boolean, and colon-prefixed attributes'
        // <my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>
        let (fragments, _) = parse_and_get_fragments(
            r#"<my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component id=\"comp\""),
                attr_complex(":config", "settings"),
                bool_attr("enabled", "isEnabled"),
                raw("></my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_reject_boolean_without_handlebars() {
        // Port of: 'should reject boolean attribute without handlebars'
        // <input ?checked="name"></input>
        let (fragments, _) = parse_and_get_fragments(r#"<input ?checked="name"></input>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<input></input>"),]);
    }

    #[test]
    fn test_attribute_reject_boolean_with_partial_handlebars() {
        // Port of: 'should reject boolean attribute with partial handlebars'
        // <input ?checked="Hello {{name}}"></input>
        let (fragments, _) =
            parse_and_get_fragments(r#"<input ?checked="Hello {{name}}"></input>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<input></input>"),]);
    }

    #[test]
    fn test_attribute_reject_boolean_with_plain_value() {
        // Port of: 'should reject boolean attribute with plain value'
        // <button ?disabled="true">Click</button>
        let (fragments, _) = parse_and_get_fragments(r#"<button ?disabled="true">Click</button>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<button>Click</button>"),]);
    }

    #[test]
    fn test_attribute_boolean_predicate_equal() {
        // Boolean attribute with == predicate expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<div ?data-active="{{page == 'dashboard'}}">X</div>"#);
        assert_fragments!(
            fragments,
            [
                raw("<div"),
                bool_attr_predicate("data-active", "page", 3, "'dashboard'"),
                raw(">X</div>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_predicate_greater_than() {
        // Boolean attribute with > predicate expression
        let (fragments, _) = parse_and_get_fragments(r#"<span ?hidden="{{num > 9}}">X</span>"#);
        assert_fragments!(
            fragments,
            [
                raw("<span"),
                bool_attr_predicate("hidden", "num", 1, "9"),
                raw(">X</span>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_predicate_not_equal() {
        // Boolean attribute with != predicate expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<a ?data-active="{{status != 'inactive'}}">X</a>"#);
        assert_fragments!(
            fragments,
            [
                raw("<a"),
                bool_attr_predicate("data-active", "status", 4, "'inactive'"),
                raw(">X</a>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_negation() {
        // Boolean attribute with negated expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<button ?disabled="{{!isReady}}">X</button>"#);
        assert_fragments!(
            fragments,
            [
                raw("<button"),
                bool_attr_not("disabled", "isReady"),
                raw(">X</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_mixed_static_dynamic() {
        // Port of: 'should process mixed attributes correctly'
        // <input value="hello {{world}}">Hi</input>
        let (fragments, records) =
            parse_and_get_fragments(r#"<input value="hello {{world}}">Hi</input>"#);

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                attr_template("value", "attr-1"),
                raw(">Hi</input>"),
            ]
        );

        // Verify the template sub-stream
        assert_stream!(records, "attr-1", [raw("hello "), signal("world"),]);
    }

    // ── Body signal tests ─────────────────────────────────────────────

    #[test]
    fn test_body_signals() {
        let (fragments, _) = parse_and_get_fragments("<body><app-shell></app-shell></body>");
        assert_fragments!(
            fragments,
            [
                raw("<body>"),
                signal_raw("body_start"),
                raw("<app-shell></app-shell>"),
                signal_raw("body_end"),
                raw("</body>"),
            ]
        );
    }

    // ── Empty for handling tests ──────────────────────────────────────

    #[test]
    fn test_empty_for_produces_nothing() {
        let (fragments, records) =
            parse_and_get_fragments(r#"<div><for each="item in items"></for></div>"#);
        assert_fragments!(fragments, [raw("<div></div>"),]);
        assert!(!records.contains_key("for-1"));
    }

    // ── Self-closing / void element tests ─────────────────────────────

    #[test]
    fn test_self_closing_svg_path() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#);
        assert_fragments!(
            fragments,
            [raw(
                r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#
            ),]
        );
    }

    #[test]
    fn test_html5_void_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#,
        );
        assert_fragments!(
            fragments,
            [raw(
                r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#
            ),]
        );
    }

    #[test]
    fn test_self_closing_with_dynamic_attributes() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="{{imageUrl}}" alt="{{imageAlt}}" />"#);
        assert_fragments!(
            fragments,
            [
                raw("<img"),
                attr("src", "imageUrl"),
                attr("alt", "imageAlt"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_self_closing_with_boolean_attributes() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<input type="checkbox" ?checked="{{isSelected}}" ?disabled="{{isDisabled}}" />"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<input type=\"checkbox\""),
                bool_attr("checked", "isSelected"),
                bool_attr("disabled", "isDisabled"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_multiple_self_closing_in_sequence() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="1.jpg" /><br /><img src="2.jpg" />"#);
        assert_fragments!(
            fragments,
            [raw(r#"<img src="1.jpg"/><br/><img src="2.jpg"/>"#),]
        );
    }

    #[test]
    fn test_self_closing_with_mixed_content() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div>Text before<img src="{{url}}" />Text after</div>"#);
        assert_fragments!(
            fragments,
            [
                raw("<div>Text before<img"),
                attr("src", "url"),
                raw("/>Text after</div>"),
            ]
        );
    }

    #[test]
    fn test_self_closing_svg_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<svg><circle cx="{{x}}" cy="{{y}}" r="5" /><rect width="10" height="10" /></svg>"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<svg><circle"),
                attr("cx", "x"),
                attr("cy", "y"),
                raw(r#" r="5"/><rect width="10" height="10"/></svg>"#),
            ]
        );
    }

    #[test]
    fn test_self_closing_inside_for_loop() {
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items"><img src="{{item.url}}" /></for>"#,
        );
        assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
        assert_stream!(
            records,
            "for-1",
            [raw("<img"), attr("src", "item.url"), raw("/>"),]
        );
    }

    #[test]
    fn test_self_closing_whitespace_variations() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="test.jpg"/><input type="text" /><br/>"#);
        assert_fragments!(
            fragments,
            [raw(r#"<img src="test.jpg"/><input type="text"/><br/>"#),]
        );
    }

    #[test]
    fn test_deeply_nested_self_closing() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><section><article><img src="deep.jpg" /><br /></article></section></div>"#,
        );
        assert_fragments!(
            fragments,
            [raw(
                r#"<div><section><article><img src="deep.jpg"/><br/></article></section></div>"#
            ),]
        );
    }

    #[test]
    fn test_self_closing_vs_empty_regular_tags() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div></div><img src="test.jpg" /><span></span>"#);
        assert_fragments!(
            fragments,
            [raw(r#"<div></div><img src="test.jpg"/><span></span>"#),]
        );
    }

    // ── Feature 1: Custom template attribute on <for> ────────────────────

    #[test]
    fn test_for_custom_template_attribute() {
        // Port of: 'should process transient node for with template'
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items" template="static"><span>Item</span></for>"#,
        );
        assert_fragments!(fragments, [for_loop("item", "items", "static"),]);
        assert_stream!(records, "static", [raw("<span>Item</span>"),]);
    }

    #[test]
    fn test_for_recursive_template() {
        // Port of: 'should process recursive transient nodes'
        let mut parser = HtmlParser::new();
        let html = r#"<for template="static" each="outerItem in outerItems"><div><span>{{outerItem.name}}</span><for template="static" each="innerItem in innerItems" /></div></for>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [for_loop("outerItem", "outerItems", "static"),]
        );

        assert_stream!(
            records,
            "static",
            [
                raw("<div><span>"),
                signal("outerItem.name"),
                raw("</span>"),
                for_loop("innerItem", "innerItems", "static"),
                raw("</div>"),
            ]
        );
    }

    // ── Feature 2: <if> / <for> with multiple children ──────────────────

    #[test]
    fn test_if_multiple_children() {
        // Port of: 'should handle <if> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<if condition="valid"><p>hello</p><p>world</p></if>"#);
        assert_fragments!(fragments, [if_cond("if-1"),]);
        assert_stream!(records, "if-1", [raw("<p>hello</p><p>world</p>"),]);
    }

    #[test]
    fn test_for_multiple_children() {
        // Port of: 'should handle <for> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<for each="item in items"><p>hello</p><p>world</p></for>"#);
        assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
        assert_stream!(records, "for-1", [raw("<p>hello</p><p>world</p>"),]);
    }

    // ── Feature 3: Handlebars at beginning/end of text ──────────────────

    #[test]
    fn test_handlebars_at_beginning() {
        // Port of: 'should process handlebars from text at beginning'
        let (fragments, _) = parse_and_get_fragments("{{first}}");
        assert_fragments!(fragments, [signal("first"),]);
    }

    #[test]
    fn test_handlebars_at_beginning_and_raw() {
        // Port of: 'should process handlebars from text at beginning and raw'
        let (fragments, _) = parse_and_get_fragments("{{first}}test");
        assert_fragments!(fragments, [signal("first"), raw("test"),]);
    }

    #[test]
    fn test_handlebars_raw_and_end() {
        // Port of: 'should process handlebars from text at raw and end'
        let (fragments, _) = parse_and_get_fragments("test{{first}}");
        assert_fragments!(fragments, [raw("test"), signal("first"),]);
    }

    // ── Feature 4: Handlebars edge cases ────────────────────────────────

    #[test]
    fn test_handlebars_invalid_triple_open() {
        // Port of: 'should not process handlebars when invalid'
        let (fragments, _) = parse_and_get_fragments("{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{invalid}}"),]);
    }

    #[test]
    fn test_handlebars_four_open_braces() {
        // Port of: 'should not process handlebars when invalid since triple exists'
        let (fragments, _) = parse_and_get_fragments("{{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{{invalid}}"),]);
    }

    #[test]
    fn test_handlebars_five_open_with_valid_double() {
        // Port of: 'should not process handlebars when invalid but with valid triple'
        let (fragments, _) = parse_and_get_fragments("{{{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{"), signal("invalid"),]);
    }

    #[test]
    fn test_entities_preserved() {
        // Port of: 'should process entities correctly'
        let (fragments, _) = parse_and_get_fragments("<p>Hello&#125;World</p>");
        assert_fragments!(fragments, [raw("<p>Hello&#125;World</p>"),]);
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

    // ── Feature 6: Component attribute skip / multiple nested ───────────

    #[test]
    fn test_component_attr_skip() {
        // Port of: 'should set attrSkip for skipped component attributes'
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-element", "<slot></slot>", None)
            .expect("register");
        let html = r#"<custom-element :config="{{config}}" class="{{value0}}" style="{{value1}}" role="{{value2}}" data-test="{{value3}}" aria-test="{{value4}}"></custom-element>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        // <custom-element, :config(attrStart), class(attrSkip), style(attrSkip),
        // role(attrSkip), data-test(attrSkip), aria-test(attrSkip), >, component, </custom-element>
        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                // :config with attrStart
                attr_complex_start(":config", "config"),
                // Skipped attrs
                attr_skip("class", "value0"),
                attr_skip("style", "value1"),
                attr_skip("role", "value2"),
                attr_skip("data-test", "value3"),
                attr_skip("aria-test", "value4"),
                raw(">"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
        );
    }

    #[test]
    fn test_component_multiple_nested() {
        // Port of: 'handle multiple nested web components'
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(
                "custom-element",
                "<custom-child></custom-child><slot></slot>",
                None,
            )
            .expect("register");
        parser
            .component_registry
            .register_component("custom-button", "<slot></slot>", None)
            .expect("register");
        parser
            .component_registry
            .register_component("custom-child", "<h1>Hello World!</h1>", None)
            .expect("register");

        let html = r#"<for each="item in items"><custom-element><custom-button>Ok</custom-button></custom-element></for>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        // Entry stream
        assert_fragments!(
            records["index.html"].fragments,
            [for_loop("item", "items", "for-1"),]
        );

        // For stream
        assert_stream!(
            records,
            "for-1",
            [
                raw("<custom-element>"),
                component("custom-element"),
                raw("<custom-button>"),
                component("custom-button"),
                raw("Ok</custom-button></custom-element>"),
            ]
        );

        // Component streams — custom-element has contains() checks, keep manual
        let ce = &records["custom-element"].fragments;
        assert_eq!(ce.len(), 3);
        assert!(
            matches!(ce[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.starts_with("<template shadowrootmode=\"open\"><custom-child>"))
        );
        assert!(
            matches!(ce[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-child")
        );
        assert!(
            matches!(ce[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</custom-child><slot></slot></template>"))
        );

        assert_stream!(
            records,
            "custom-button",
            [raw(
                "<template shadowrootmode=\"open\"><slot></slot></template>"
            ),]
        );

        assert_stream!(
            records,
            "custom-child",
            [raw(
                "<template shadowrootmode=\"open\"><h1>Hello World!</h1></template>"
            ),]
        );
    }

    // ── Error handling tests ──────────────────────────────────────────

    #[test]
    fn test_invalid_markup_returns_error() {
        // Port of: 'should fail with invalid markup'
        // tree-sitter is lenient — it recovers from unclosed tags
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<div><span>Unclosed div");
        assert!(result.is_ok());
    }

    // ── Integration tests ─────────────────────────────────────────────

    #[test]
    fn test_complex_raw_text_full_page() {
        // Port of: 'should process a complex raw text'
        let html = r#"<!DOCTYPE HTML><html dir="auto" lang="en"><head><meta charset="utf-8"><title>Test</title><style>html { margin: 0; }</style></head><body><app-shell></app-shell><script type="module" src="./index.js"></script></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        // DOCTYPE + head + <body>, body_start, body content, body_end, </body></html>
        assert!(fragments.len() >= 5);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE HTML>") && raw.value.ends_with("<body>"))
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_start" && s.raw)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<app-shell>"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_end" && s.raw)
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</body>") && raw.value.contains("</html>"))
        );
    }

    #[test]
    fn test_css_strategy_external_emits_link_tag() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component("my-card", "<p><slot></slot></p>", Some("p { color: red; }"))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let raw_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            raw_text.contains(r#"<link rel="stylesheet" href="/my-card.css">"#),
            "Expected external <link> tag in: {}",
            raw_text
        );
    }

    #[test]
    fn test_css_strategy_inline_emits_style_tag() {
        let mut parser = HtmlParser::new();
        parser.set_css_strategy(CssStrategy::Style);
        parser
            .component_registry_mut()
            .register_component("my-card", "<p><slot></slot></p>", Some("p { color: red; }"))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let raw_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            raw_text.contains("<style>p { color: red; }</style>"),
            "Expected inline <style> tag in: {}",
            raw_text
        );
        assert!(
            !raw_text.contains("<link"),
            "Should not have <link> tag in inline mode: {}",
            raw_text
        );
    }

    // ── Ported from NodeJS generator.test.js ─────────────────────────

    // test_signal_with_default_value — SKIPPED
    // The NodeJS `<f-signal value="testSignal">Default Text</f-signal>` feature
    // is not supported in the Rust parser. There is no `f-signal` element
    // handling in HtmlParser and no corresponding fragment type in
    // webui_protocol.

    // test_estimated_buffer_size — SKIPPED
    // The NodeJS `estimatedBufferSize` field does not exist in the Rust
    // WebUIFragmentRecords / WebUIProtocol types. Buffer size estimation is
    // not part of the Rust parser output.

    #[test]
    fn test_body_start_end_injection() {
        // Port of: 'should inject body_start and body_end signals around body content'
        // Verifies body_start appears immediately after <body> and body_end
        // appears immediately before </body> in a full HTML page.
        let html = r#"<html><head><title>Test</title></head><body><div>Content</div><p>More</p></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head><title>Test</title></head><body>"),
                signal_raw("body_start"),
                raw("<div>Content</div><p>More</p>"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_fail_with_invalid_markup() {
        // Port of: 'should fail with invalid markup'
        // tree-sitter HTML grammar is lenient — it recovers from overlapping /
        // misnested tags rather than returning a parse error. This test
        // documents that behavior: deliberately overlapping tags do NOT
        // produce an error.
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<div><span></div></span>");

        // tree-sitter recovers gracefully; the parse succeeds
        assert!(
            result.is_ok(),
            "tree-sitter is lenient and recovers from overlapping tags"
        );
    }

    #[test]
    fn test_complex_raw_text_page() {
        // Port of: 'should process a complex raw text page with DOCTYPE,
        // meta tags, styles, and scripts'
        let html = concat!(
            "<!DOCTYPE html>",
            "<html lang=\"en\">",
            "<head>",
            "<meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">",
            "<title>Complex Page</title>",
            "<style>body { margin: 0; padding: 0; } h1 { color: blue; }</style>",
            "<link rel=\"stylesheet\" href=\"styles.css\">",
            "</head>",
            "<body>",
            "<h1>Hello World</h1>",
            "<script type=\"module\" src=\"./app.js\"></script>",
            "</body>",
            "</html>",
        );
        let (fragments, _) = parse_and_get_fragments(html);

        // Should have: raw(DOCTYPE+head+<body>), body_start, raw(body content),
        // body_end, raw(</body></html>)
        assert!(
            fragments.len() >= 5,
            "Expected at least 5 fragments, got {}",
            fragments.len()
        );

        // First fragment: DOCTYPE through opening <body>
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE html>") &&
                raw.value.contains("<meta charset=\"utf-8\">") &&
                raw.value.contains("<meta name=\"viewport\"") &&
                raw.value.contains("<title>Complex Page</title>") &&
                raw.value.contains("<style>") &&
                raw.value.contains("body { margin: 0; padding: 0; }") &&
                raw.value.ends_with("<body>")),
            "First fragment should contain all head content through <body>, got: {:?}",
            fragments[0]
        );

        // body_start signal
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_start" && s.raw),
            "Second fragment should be body_start signal"
        );

        // Body content (h1 and script)
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<h1>Hello World</h1>") &&
                raw.value.contains("<script")),
            "Third fragment should contain body content"
        );

        // body_end signal
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_end" && s.raw),
            "Fourth fragment should be body_end signal"
        );

        // Closing tags
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</body>") && raw.value.contains("</html>")),
            "Fifth fragment should contain closing tags"
        );
    }

    // --- Binding count tests (with mock plugin) ---

    /// Mock plugin that records the binding attribute count for each element.
    struct BindingCountPlugin {
        counts: Vec<u32>,
    }

    impl BindingCountPlugin {
        fn new() -> Self {
            Self { counts: Vec::new() }
        }
    }

    impl crate::plugin::ParserPlugin for BindingCountPlugin {
        fn on_parse_component(&mut self, _tag_name: &str, _component: &Component) -> Result<()> {
            Ok(())
        }

        fn should_skip_attribute(&self, attr_name: &str) -> bool {
            attr_name.starts_with('@') || attr_name == "f-ref"
        }

        fn on_body_end(&mut self) -> Option<String> {
            None
        }

        fn on_element_parsed(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
            self.counts.push(binding_attribute_count);
            if binding_attribute_count > 0 {
                Some(binding_attribute_count.to_le_bytes().to_vec())
            } else {
                None
            }
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn test_component_static_attrs_not_counted_as_bindings() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component("my-btn", "<button><slot></slot></button>", None)
            .expect("register");

        // All attributes are static — binding count should be 0
        let html = r#"<my-btn class="primary" title="Click me">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();

        // No Plugin fragment should appear (binding count = 0)
        let frags = &records["index.html"].fragments;
        let plugin_count = frags
            .iter()
            .filter(|f| matches!(f.fragment.as_ref(), Some(Fragment::Plugin(_))))
            .count();
        assert_eq!(
            plugin_count, 0,
            "Static-only component attributes should not emit a Plugin fragment"
        );
    }

    #[test]
    fn test_component_dynamic_attr_counted_as_binding() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component("my-btn", "<button><slot></slot></button>", None)
            .expect("register");

        // One dynamic attribute ({{...}}) — binding count should be 1
        let html = r#"<my-btn appearance="{{style}}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        // Exactly one Plugin fragment with count = 1
        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1, "Expected 1 Plugin fragment");
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        assert_eq!(count, 1, "Binding attribute count should be 1");
    }

    #[test]
    fn test_component_mixed_static_and_dynamic_attrs_binding_count() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component("my-btn", "<button><slot></slot></button>", None)
            .expect("register");

        // 2 static, 1 dynamic, 1 skipped-with-plugin (@click) — only dynamic + skipped counted
        let html = r#"<my-btn class="primary" title="Submit" appearance="{{look}}" @click="{go}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1);
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        // 1 dynamic (appearance) + 1 skipped-but-counted (@click) = 2
        assert_eq!(
            count, 2,
            "Binding count should include dynamic + plugin-skipped attrs, not static"
        );
    }

    #[test]
    fn test_component_only_skipped_plugin_attrs_counted() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component("my-btn", "<button><slot></slot></button>", None)
            .expect("register");

        // Only plugin-skipped attrs (@click, f-ref) plus static — only skipped counted
        let html = r#"<my-btn title="Hello" @click="{go}" f-ref="{btn}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1);
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        // @click + f-ref = 2, title is static = not counted
        assert_eq!(
            count, 2,
            "Only plugin-skipped attrs should be counted, not static title"
        );
    }

    // ── Comment binding tests ────────────────────────────────────────

    #[test]
    fn test_comment_handlebars_signal() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{tokens}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [signal("tokens")]);
    }

    #[test]
    fn test_comment_triple_brace_raw_signal() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{{tokens}}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [signal_raw("tokens")]);
    }

    #[test]
    fn test_comment_regular_preserved() {
        let mut parser = HtmlParser::new();
        let html = "<!-- regular comment -->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [raw("<!-- regular comment -->")]);
    }

    #[test]
    fn test_comment_dotted_signal() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{tokens.light}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [signal("tokens.light")]);
    }

    #[test]
    fn test_comment_arbitrary_identifier() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{someOtherBinding}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [signal("someOtherBinding")]);
    }

    #[test]
    fn test_comment_with_surrounding_content() {
        let mut parser = HtmlParser::new();
        let html = "<div><!--{{tokens}}--></div>";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "test.html",
            [raw("<div>"), signal("tokens"), raw("</div>")]
        );
    }

    // ── Token collection tests ───────────────────────────────────────

    #[test]
    fn test_tokens_from_style_tag() {
        let mut parser = HtmlParser::new();
        let html = r#"<style>
            .btn { color: var(--colorPrimary); background: var(--bgColor); }
        </style>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["bgColor", "colorPrimary"]);
    }

    #[test]
    fn test_tokens_from_component_css() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-button",
                "<button>Click</button>",
                Some(":host { color: var(--textColor); border: var(--borderWidth); }"),
            )
            .expect("register failed");

        let html = "<my-button></my-button>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["borderWidth", "textColor"]);
    }

    #[test]
    fn test_tokens_merged_from_style_and_components() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-widget",
                "<div>Widget</div>",
                Some(".w { padding: var(--spacingM); }"),
            )
            .expect("register failed");

        let html = r#"<style>.root { color: var(--textColor); }</style><my-widget></my-widget>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["spacingM", "textColor"]);
    }

    #[test]
    fn test_tokens_deduplicated() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-btn",
                "<button>B</button>",
                Some(".b { color: var(--shared); }"),
            )
            .expect("register failed");

        let html = r#"<style>.x { color: var(--shared); }</style><my-btn></my-btn>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["shared"]);
    }

    #[test]
    fn test_tokens_empty_when_no_vars() {
        let mut parser = HtmlParser::new();
        let html = "<div>Hello</div>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokens_exclude_locally_defined_in_component() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-card",
                "<div>Card</div>",
                Some(":host { --local: 5px; width: var(--external); }"),
            )
            .expect("register failed");

        let html = "<my-card></my-card>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["external"]);
    }

    #[test]
    fn test_tokens_exclude_entry_root_definitions() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-btn",
                "<button>B</button>",
                Some(".b { color: var(--color-primary); border-radius: var(--radius-m); }"),
            )
            .expect("register failed");

        // Entry HTML defines --color-primary and --radius-m in :root
        // Components use them — they should NOT appear in hoisted tokens
        let html = r#"<style>
            :root {
                --color-primary: #0078d4;
                --radius-m: 6px;
            }
            body { color: var(--color-primary); }
        </style>
        <my-btn></my-btn>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        // Both tokens are defined in entry :root, so neither should be hoisted
        assert!(
            tokens.is_empty(),
            "Tokens defined in entry :root should be excluded: {tokens:?}"
        );
    }

    #[test]
    fn test_tokens_entry_defs_exclude_but_external_kept() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(
                "my-card",
                "<div>Card</div>",
                Some(".c { color: var(--color-primary); margin: var(--external-spacing); }"),
            )
            .expect("register failed");

        // Entry defines --color-primary but NOT --external-spacing
        let html = r#"<style>
            :root { --color-primary: #0078d4; }
        </style>
        <my-card></my-card>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        // Only --external-spacing should be hoisted (not defined in entry)
        assert_eq!(tokens, vec!["external-spacing"]);
    }

    // ── Route parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_simple_route() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/profile" component="profile-page" name="profile" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Routes are emitted as Fragment::Route
        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 1);
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile");
                assert_eq!(r.name, "profile");
                assert_eq!(r.fragment_id, "profile-page");
                assert!(r.exact);
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_route_with_params() {
        let mut parser = HtmlParser::new();
        let html =
            r#"<route path="/profile/:id/view/:section" component="detail" name="detail" />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Route is registered in the route registry
        let routes = parser.take_routes();
        assert_eq!(routes["detail"].path, "/profile/:id/view/:section");
    }

    #[test]
    fn test_parse_route_requires_component() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/old" />"#;
        let result = parser.parse("test.html", html);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_multiple_routes() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" name="app" component="app-layout" />
            <route path="/dashboard" name="dashboard" component="dash-page" exact />
            <route path="/contacts" name="contacts" component="contacts-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        let routes = parser.take_routes();
        // Should have 3 routes
        assert_eq!(routes.len(), 3);
        assert!(routes.contains_key("app"));
        assert!(routes.contains_key("dashboard"));
        assert!(routes.contains_key("contacts"));

        // App has correct path
        let app = &routes["app"];
        assert_eq!(app.path, "/");
    }

    #[test]
    fn test_parse_route_duplicate_name_error() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/a" name="home" component="a" />
            <route path="/b" name="home" component="b" />"#;
        let result = parser.parse("test.html", html);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_route_requires_component_with_body() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/error" name="error-page">
            <div class="error"><h1>Not Found</h1></div>
        </route>"#;
        let result = parser.parse("test.html", html);
        assert!(
            result.is_err(),
            "Route without component attribute should fail"
        );
    }

    #[test]
    fn test_parse_multiple_routes_with_registry() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" name="home" component="home-page" exact />
            <route path="/about" name="about" component="about-page" exact />
            <route path="/contact/:id" name="contact" component="contact-page" />"#;
        parser.parse("test.html", html).expect("parse failed");

        let routes = parser.take_routes();
        assert_eq!(routes.len(), 3);
        assert_eq!(routes["home"].path, "/");
        assert_eq!(routes["about"].path, "/about");
        assert_eq!(routes["contact"].path, "/contact/:id");
    }
}
