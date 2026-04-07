// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

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

use crate::plugin::{AttributeAction, ParserPlugin, ParserPluginArtifacts};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIteratorMut};
use tree_sitter_html::LANGUAGE;
use webui_protocol::{
    web_ui_fragment, web_ui_fragment::Fragment, ConditionExpr, FragmentList, WebUIFragment,
    WebUIFragmentAttribute, WebUIFragmentRecords, WebUiFragmentRoute,
};

/// Strategy for how component CSS is delivered in rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum CssStrategy {
    /// Emit `<link rel="stylesheet" href="/component.css">` tags (default).
    #[default]
    Link,
    /// Embed CSS content in `<style>` tags within the component.
    Style,
    /// Emit a `<style type="module" specifier="component">` definition once per
    /// page. The client runtime applies it via the document's adopted stylesheets.
    Module,
}

impl std::fmt::Display for CssStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssStrategy::Link => write!(f, "link"),
            CssStrategy::Style => write!(f, "style"),
            CssStrategy::Module => write!(f, "module"),
        }
    }
}

impl std::str::FromStr for CssStrategy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "link" => Ok(CssStrategy::Link),
            "style" => Ok(CssStrategy::Style),
            "module" => Ok(CssStrategy::Module),
            other => Err(format!(
                "Unknown CSS strategy: {other}. Use \"link\", \"style\", or \"module\"."
            )),
        }
    }
}

/// Strategy for how component DOM is structured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum DomStrategy {
    /// Use shadow DOM with declarative shadow roots for SSR (default).
    #[default]
    Shadow,
    /// Use light DOM — component content is rendered as direct children.
    Light,
}

impl std::fmt::Display for DomStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomStrategy::Shadow => write!(f, "shadow"),
            DomStrategy::Light => write!(f, "light"),
        }
    }
}

impl std::str::FromStr for DomStrategy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "shadow" => Ok(DomStrategy::Shadow),
            "light" => Ok(DomStrategy::Light),
            other => Err(format!(
                "Unknown DOM strategy: {other}. Use \"shadow\" or \"light\"."
            )),
        }
    }
}

/// Framework plugin to load for build-time and render-time processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Plugin {
    /// Fast hydration plugin — lightweight client-side interactivity.
    Fast,
    /// WebUI plugin — full component model with shadow DOM support.
    #[cfg_attr(feature = "cli", value(name = "webui"))]
    WebUI,
}

impl std::fmt::Display for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Plugin::Fast => write!(f, "fast"),
            Plugin::WebUI => write!(f, "webui"),
        }
    }
}

impl std::str::FromStr for Plugin {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "fast" => Ok(Plugin::Fast),
            "webui" => Ok(Plugin::WebUI),
            other => Err(format!("Unknown plugin: {other}")),
        }
    }
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

    /// How component DOM is structured (shadow or light).
    dom_strategy: DomStrategy,

    /// Optional parser plugin for framework-specific behavior.
    plugin: Option<Box<dyn ParserPlugin>>,

    /// Accumulated CSS custom property token names from all processed
    /// components and inline `<style>` tags.
    token_store: HashSet<String>,

    /// CSS custom property names **defined** in inline `<style>` tags
    /// (e.g., `:root { --color-primary: #0078d4; }`). These are excluded
    /// from the final token set since the app already provides their values.
    token_definitions: HashSet<String>,
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
            dom_strategy: DomStrategy::default(),
            plugin: None,
            token_store: HashSet::new(),
            token_definitions: HashSet::new(),
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

    /// Set the DOM strategy for component rendering (shadow or light).
    pub fn set_dom_strategy(&mut self, strategy: DomStrategy) -> &mut Self {
        self.dom_strategy = strategy;
        self
    }

    /// Get a mutable reference to the component registry.
    pub fn component_registry_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.component_registry
    }

    pub fn into_fragment_records(mut self) -> WebUIFragmentRecords {
        std::mem::take(&mut self.fragment_records)
    }

    /// Check if a fragment ID has been parsed (exists in the fragment records).
    pub fn has_fragment(&self, fragment_id: &str) -> bool {
        self.fragment_records.contains_key(fragment_id)
    }

    /// Take any post-parse artifacts captured by the parser plugin.
    #[must_use]
    pub fn take_plugin_artifacts(&mut self) -> ParserPluginArtifacts {
        self.plugin
            .take()
            .map_or(ParserPluginArtifacts::None, |plugin| {
                plugin.into_artifacts()
            })
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

    /// Parse HTML content to generate WebUI fragments.
    pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<()> {
        // Reset sub-fragments for new parse
        self.raw_buffer.clear();
        if let Some(ref mut plugin) = self.plugin {
            plugin.start_fragment(fragment_id);
        }

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

    /// Returns true when a text node should be emitted into the fragment stream.
    ///
    /// Pure formatting nodes that contain line breaks are still dropped, but
    /// inline whitespace-only separators such as the spaces around `&gt;` in
    /// `{{sectionName}} &gt; {{topicName}}` must be preserved. Tree-sitter can
    /// split those separators into standalone text nodes.
    fn should_emit_text_content(content: &str) -> bool {
        if content.is_empty() {
            return false;
        }

        if !content.trim().is_empty() {
            return true;
        }

        content.chars().all(char::is_whitespace)
            && !content.contains('\n')
            && !content.contains('\r')
    }

    /// Process child nodes while preserving raw source gaps between them.
    ///
    /// Tree-sitter HTML treats some authored whitespace as extras rather than
    /// concrete child nodes. Reconstructing the byte gaps between children keeps
    /// inline separators like `{{a}} &gt; {{b}}` intact.
    fn process_children_with_source_gaps(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
        skip_structural_tags: bool,
    ) -> Result<()> {
        let mut cursor = node.walk();
        let mut last_end = node.start_byte();

        for child in node.children(&mut cursor) {
            if child.start_byte() > last_end {
                let gap = &source[last_end..child.start_byte()];
                if Self::should_emit_text_content(gap) {
                    self.add_raw_fragment(gap);
                }
            }

            let kind = child.kind();
            if !skip_structural_tags || (kind != "start_tag" && kind != "end_tag") {
                self.process_child_node(child, source, fragments)?;
            }

            last_end = child.end_byte();
        }

        if node.end_byte() > last_end {
            let gap = &source[last_end..node.end_byte()];
            if Self::should_emit_text_content(gap) {
                self.add_raw_fragment(gap);
            }
        }

        Ok(())
    }

    /// Scan CSS text for `/* ... */` comments that contain exactly one
    /// handlebars expression (e.g. `/*{{{tokens.light}}}*/`). Replace
    /// each matching comment with a signal fragment and keep everything
    /// else as raw CSS.
    ///
    /// A comment is a "signal placeholder" when:
    /// 1. Its trimmed inner text parses to exactly one signal fragment.
    /// 2. There is no other text around the signal inside the comment.
    ///
    /// Non-matching comments and all non-comment CSS are emitted as raw.
    fn extract_style_comment_signals(
        &mut self,
        css: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        let bytes = css.as_bytes();
        let len = bytes.len();
        let mut pos = 0;

        while pos < len {
            // Find the next comment opening
            let Some(open) = css[pos..].find("/*") else {
                // No more comments — emit remaining CSS as raw
                self.add_raw_fragment(&css[pos..]);
                break;
            };
            let open = pos + open;

            // Find the matching close
            let Some(close_offset) = css[open + 2..].find("*/") else {
                // Unterminated comment — emit rest as raw
                self.add_raw_fragment(&css[pos..]);
                break;
            };
            let close = open + 2 + close_offset;
            let after_close = close + 2;

            // Extract and trim the inner text of the comment
            let inner = css[open + 2..close].trim();

            // Try to parse the inner text with the handlebars parser.
            // Accept only if it produces exactly one signal fragment.
            let is_signal = if !inner.is_empty() {
                match self.handlebars_parser.parse(inner) {
                    Ok(ref parsed) if parsed.len() == 1 => {
                        matches!(parsed[0].fragment.as_ref(), Some(Fragment::Signal(_)))
                    }
                    _ => false,
                }
            } else {
                false
            };

            if is_signal {
                // Emit CSS before the comment as raw
                self.add_raw_fragment(&css[pos..open]);

                // Emit the signal fragment (re-parse to extract it)
                let parsed = self.handlebars_parser.parse(inner)?;
                self.add_fragment(parsed.into_iter().next().unwrap_or_default(), fragments);
            } else {
                // Not a signal placeholder — emit everything including
                // the comment as raw CSS
                self.add_raw_fragment(&css[pos..after_close]);
            }

            pos = after_close;
        }

        Ok(())
    }

    /// Process all non-structural child nodes inside an element-like container.
    fn process_content_children(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        self.process_children_with_source_gaps(node, source, fragments, true)
    }
    /// Process an HTML node to generate WebUI fragments.
    fn process_html_node(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        if node.kind() == "document" || node.kind() == "fragment" || node.kind() == "element" {
            self.process_children_with_source_gaps(node, source, fragments, false)?;
        } else {
            // Add text content as raw fragment
            let content = &source[node.start_byte()..node.end_byte()];
            if Self::should_emit_text_content(content) {
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
                    "head" => return self.process_head_element(node, source, fragments),
                    "route" => return self.process_route_directive(node, source, fragments),
                    "outlet" => {
                        self.flush_raw_buffer(fragments);
                        fragments.push(WebUIFragment::outlet());
                        return Ok(());
                    }
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
                // Process inline CSS — extract tokens and recognise
                // comment-delimited signal placeholders (e.g. /*{{{tokens.light}}}*/).
                self.add_raw_fragment("<style>");
                for child in node.named_children(&mut node.walk()) {
                    if child.kind() == "raw_text" {
                        let style_content = &source[child.start_byte()..child.end_byte()];

                        // Single parse: extract both token usages and definitions.
                        // This must run on the *original* style source, unmodified.
                        if let Ok((tokens, defs)) = self
                            .css_parser
                            .extract_tokens_and_definitions(style_content)
                        {
                            self.token_store.extend(tokens);
                            self.token_definitions.extend(defs);
                        }

                        // Scan for CSS comments that contain exactly one
                        // handlebars expression. Replace those comments with
                        // signal fragments; leave everything else as raw CSS.
                        self.extract_style_comment_signals(style_content, fragments)?;
                    }
                }
                self.add_raw_fragment("</style>");
            }
            "text" | "raw_text" => {
                let content = &source[node.start_byte()..node.end_byte()];
                if Self::should_emit_text_content(content) {
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

        // Check for boolean (valueless) attributes via tree-sitter query.
        // Only match attributes directly on this element's start_tag/self_closing_tag,
        // not on nested child elements (cursor.matches descends into subtrees).
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

        // Find the start_tag or self_closing_tag node for this element
        let tag_node_id = node
            .named_children(&mut node.walk())
            .find(|c| c.kind() == "start_tag" || c.kind() == "self_closing_tag")
            .map(|c| c.id());

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, node, source.as_bytes());
        while let Some(m) = matches.next_mut() {
            for capture in m.captures.iter() {
                let capture_name = query.capture_names()[capture.index as usize];
                if capture_name == "name" {
                    // Only accept attributes whose parent tag belongs to THIS element
                    let attr_parent = capture.node.parent().and_then(|a| a.parent());
                    if attr_parent
                        .map(|p| Some(p.id()) == tag_node_id)
                        .unwrap_or(false)
                    {
                        let name_text =
                            capture.node.utf8_text(source.as_bytes()).map_err(|_| {
                                ParserError::Html("Invalid UTF-8 for attribute name".to_string())
                            })?;
                        if name_text == attr_name {
                            return Ok(true);
                        }
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
                if let Some(data) = p.finish_element(binding_count) {
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

                self.process_content_children(node, source, fragments)?;

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

            if let Some(ref mut p) = self.plugin {
                match p.classify_attribute(&attr_name) {
                    AttributeAction::Keep => {}
                    AttributeAction::Skip => continue,
                    AttributeAction::SkipAndCountBinding => {
                        binding_count += 1;
                        continue;
                    }
                }
            }

            let attr_value = self.get_attr_value(child, source);

            if let Some(bool_name) = attr_name.strip_prefix('?') {
                // Boolean attribute: ?disabled={{isDisabled}}
                if is_component {
                    if let Some(condition) = self.parse_boolean_condition(attr_value.as_deref()) {
                        let frag = Self::maybe_mark_attr_start(
                            WebUIFragment::attribute_boolean(bool_name, condition),
                            &mut first_dynamic_emitted,
                        );
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    }
                } else if self.process_boolean_attribute(
                    bool_name,
                    attr_value.as_deref(),
                    fragments,
                )? {
                    binding_count += 1;
                }
            } else if let Some(prop_name) = attr_name.strip_prefix(':') {
                // Complex attribute: :config="{{settings}}"
                // Only valid on custom elements — passes JS values by
                // reference (objects, arrays). Native HTML elements should
                // use regular attribute bindings (the framework already
                // handles value, checked, selected as DOM properties).
                if !is_component {
                    return Err(ParserError::Parse(format!(
                        ":{prop_name} complex binding is only allowed on custom elements. \
                         Use {prop_name}=\"{{{{expr}}}}\" for native HTML elements."
                    )));
                }
                if Self::is_blocked_complex_property(prop_name) {
                    return Err(ParserError::Parse(format!(
                        ":{prop_name} is not allowed as a complex attribute binding \
                         because it enables arbitrary HTML injection. \
                         Use {{{{{{expr}}}}}} (triple-brace) syntax for raw HTML."
                    )));
                }
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
            } else if is_component && Self::is_skipped_attribute(&attr_name) {
                // Skipped component attribute (class, style, role, data-*, aria-*)
                if let Some(val) = &attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        // Pure binding: role="{{dynamicRole}}"
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
                    } else if Self::contains_handlebars(val) {
                        // Embedded binding: aria-labelledby="prefix-{{group.id}}"
                        let template_id = self.id_counter.next_id("attr");
                        let parsed = self.handlebars_parser.parse(val)?;
                        self.fragment_records
                            .insert(template_id.clone(), FragmentList { fragments: parsed });
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name,
                                    template: template_id,
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    } else {
                        // Static value: role="list"
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name,
                                    value: val.clone(),
                                    raw_value: true,
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
    fn parse_boolean_condition(&self, value: Option<&str>) -> Option<ConditionExpr> {
        if let Some(val) = value {
            if let Some(expr_str) = Self::extract_single_handlebars(val) {
                return Some(
                    self.condition_parser
                        .parse(&expr_str)
                        .unwrap_or_else(|_| ConditionExpr::identifier(&expr_str)),
                );
            }
        }

        None
    }

    fn process_boolean_attribute(
        &mut self,
        name: &str,
        value: Option<&str>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<bool> {
        if let Some(condition) = self.parse_boolean_condition(value) {
            self.add_fragment(WebUIFragment::attribute_boolean(name, condition), fragments);
            return Ok(true);
        }
        // Invalid boolean attribute — silently drop (no output at all)
        Ok(false)
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

    /// Process a `<head>` element, injecting a head_end signal before `</head>`.
    fn process_head_element(
        &mut self,
        node: Node,
        source: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        self.add_raw_fragment("<head>");
        self.process_content_children(node, source, fragments)?;
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::signal("head_end", true));
        self.add_raw_fragment("</head>");
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
        self.process_content_children(node, source, fragments)?;
        self.flush_raw_buffer(fragments);
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
        self.process_content_children(node, source, &mut for_fragment)?;

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
        self.process_content_children(node, source, &mut if_fragment)?;

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

        let exact = self.has_element_attribute(node, "exact", source)?;

        let attrs = route_parser::RouteAttributes {
            path: path.clone(),
            component: component.clone(),
            exact,
        };

        // Validate attributes (component is required)
        route_parser::validate_attributes(&attrs)?;

        // Extract params from path template (validation only)
        route_parser::extract_params(&path)?;

        // Ensure the component's template is parsed and registered
        self.ensure_route_component_parsed(&component)?;

        // Recursively parse nested <route> children
        let children = self.parse_child_routes(node, source)?;

        // Flush any pending raw content before the route fragment
        self.flush_raw_buffer(fragments);

        // Build route fragment with children
        let route_fragment =
            route_parser::build_route_fragment(&attrs, component.clone(), children);

        // Emit Fragment::Route — the handler renders it as <webui-route>
        fragments.push(WebUIFragment::route_from(route_fragment));

        Ok(())
    }

    /// Parse nested `<route>` children of a route element.
    fn parse_child_routes(&mut self, node: Node, source: &str) -> Result<Vec<WebUiFragmentRoute>> {
        let mut children = Vec::new();
        let mut cursor = node.walk();

        for child in node.named_children(&mut cursor) {
            if child.kind() == "element" {
                if let Ok(tag) = self.get_element_tag_name(child, source) {
                    if tag == "route" {
                        let child_route = self.parse_route_as_fragment(child, source)?;
                        children.push(child_route);
                    }
                }
            }
        }

        Ok(children)
    }

    /// Parse a `<route>` element into a `WebUiFragmentRoute` (for nesting).
    fn parse_route_as_fragment(&mut self, node: Node, source: &str) -> Result<WebUiFragmentRoute> {
        let path = self
            .get_element_attribute(node, "path", source)?
            .unwrap_or_default();
        let component = self
            .get_element_attribute(node, "component", source)?
            .unwrap_or_default();
        let exact = self.has_element_attribute(node, "exact", source)?;

        let attrs = route_parser::RouteAttributes {
            path: path.clone(),
            component: component.clone(),
            exact,
        };

        route_parser::validate_attributes(&attrs)?;
        route_parser::extract_params(&path)?;

        // Ensure the component's template is parsed
        self.ensure_route_component_parsed(&component)?;

        // Recursively parse nested children
        let children = self.parse_child_routes(node, source)?;

        Ok(route_parser::build_route_fragment(
            &attrs, component, children,
        ))
    }

    /// Ensure a route-referenced component is parsed and registered.
    fn ensure_route_component_parsed(&mut self, component: &str) -> Result<()> {
        if component.is_empty()
            || !self.component_registry.contains(component)
            || self.fragment_records.contains_key(component)
        {
            return Ok(());
        }

        let component_data = self
            .component_registry
            .get(component)
            .ok_or_else(|| {
                crate::error::ParserError::Directive(format!("Component not found: {component}"))
            })?
            .clone();

        self.token_store
            .extend(component_data.css_tokens.iter().cloned());

        let processed = self.build_component_template(
            component,
            &component_data.html_content,
            component_data.css_content.as_deref(),
        );

        if let Some(ref mut p) = self.plugin {
            p.register_component_template(component, &component_data, &processed)?;
        }

        let saved_buffer = std::mem::take(&mut self.raw_buffer);
        self.parse(component, &processed)?;
        self.raw_buffer = saved_buffer;

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

    /// Properties that must never be set via `:attr` complex bindings.
    /// These enable XSS (HTML injection) or arbitrary code execution.
    fn is_blocked_complex_property(name: &str) -> bool {
        matches!(name, "innerHTML" | "outerHTML" | "srcdoc" | "content") || name.starts_with("on")
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
                if let Some(data) = p.finish_element(binding_count) {
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
            let component_data = self
                .component_registry
                .get(tag_name)
                .ok_or_else(|| {
                    ParserError::Directive(format!("Component not found: {}", tag_name))
                })?
                .clone();
            let processed =
                self.build_component_template(tag_name, &html_content, css_content.as_deref());

            // Notify plugin about the final component template (only on first encounter)
            if let Some(ref mut p) = self.plugin {
                p.register_component_template(tag_name, &component_data, &processed)?;
            }

            self.parse(tag_name, &processed)?;
        }

        // Emit component fragment
        fragments.push(WebUIFragment::component(tag_name.to_string()));

        // Process slot content (skip start_tag/end_tag/self_closing_tag)
        if !is_self_closing {
            self.process_content_children(node, source, fragments)?;
        }

        // Emit closing tag
        if !is_self_closing {
            self.add_raw_fragment(&format!("</{}>", tag_name));
        }

        Ok(())
    }

    /// Process component template HTML: wrap in shadow DOM template if needed,
    /// inject CSS snippet (link or inline style), and strip runtime-only attributes.
    /// When `adopted_specifier` is `Some`, it is stored in template metadata
    /// for the client runtime to handle document-level CSS adoption.
    fn build_component_template(
        &mut self,
        tag_name: &str,
        html: &str,
        css_content: Option<&str>,
    ) -> String {
        let adopted_specifier = match self.css_strategy {
            CssStrategy::Module if css_content.is_some() => Some(tag_name.to_string()),
            _ => None,
        };
        let css_injection = match self.css_strategy {
            CssStrategy::Link => {
                // In light DOM mode, CSS links go in <head> (emitted by handler),
                // not inside each component template.
                if css_content.is_some() && self.dom_strategy == DomStrategy::Shadow {
                    Some(format!(
                        "<link rel=\"stylesheet\" href=\"/{tag_name}.css\">"
                    ))
                } else {
                    None
                }
            }
            CssStrategy::Style => css_content.map(|css| format!("<style>{}</style>", css.trim())),
            CssStrategy::Module => None,
        };

        self.process_component_template(
            html,
            css_injection.as_deref(),
            adopted_specifier.as_deref(),
        )
    }

    /// Process component template HTML.
    ///
    /// - **Shadow DOM** (`DomStrategy::Shadow`): wraps content in
    ///   `<template shadowrootmode="open">`, preserves `:host` CSS,
    ///   optionally adds `shadowrootadoptedstylesheets`.
    /// - **Light DOM** (`DomStrategy::Light`): strips any existing shadow DOM
    ///   wrapper, outputs plain HTML.
    ///
    /// In both modes, runtime-only attributes (`@`, `:`, `?`) are stripped
    /// from the opening `<template>` tag.
    fn process_component_template(
        &mut self,
        html: &str,
        css_snippet: Option<&str>,
        adopted_specifier: Option<&str>,
    ) -> String {
        let trimmed = html.trim();
        let snippet = css_snippet.unwrap_or_default();

        // Extract inner content — strip <template shadowrootmode> wrapper if present
        let has_template = trimmed.starts_with("<template");
        let inner = if has_template {
            let stripped = self.strip_runtime_attrs_from_template(trimmed);
            if let Some(open_end) = stripped.find('>') {
                let inner_start = open_end + 1;
                let inner_end = stripped.rfind("</template>").unwrap_or(stripped.len());
                if inner_start < inner_end {
                    stripped[inner_start..inner_end].to_string()
                } else {
                    String::new()
                }
            } else {
                trimmed.to_string()
            }
        } else {
            trimmed.to_string()
        };

        match self.dom_strategy {
            DomStrategy::Shadow => {
                // Re-wrap in shadow DOM template
                let adopted_attr = adopted_specifier.map(|spec| {
                    let mut s = String::with_capacity(35 + spec.len());
                    s.push_str(" shadowrootadoptedstylesheets=\"");
                    s.push_str(spec);
                    s.push('"');
                    s
                });
                let adopted_ref = adopted_attr.as_deref().unwrap_or_default();

                let mut result =
                    String::with_capacity(45 + adopted_ref.len() + snippet.len() + inner.len());
                result.push_str("<template shadowrootmode=\"open\"");
                result.push_str(adopted_ref);
                result.push('>');
                result.push_str(snippet);
                result.push_str(&inner);
                result.push_str("</template>");
                result
            }
            DomStrategy::Light => {
                // Plain light-DOM output
                if snippet.is_empty() && adopted_specifier.is_none() {
                    return inner;
                }
                let mut result = String::with_capacity(snippet.len() + inner.len() + 16);
                result.push_str(snippet);
                result.push_str(&inner);
                result
            }
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
    fn test_parse_preserves_inline_spaces_around_entity_between_bindings() {
        let mut parser = HtmlParser::new();
        let html = "<nav>{{sectionName}} &gt; {{topicName}}</nav>";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<nav>"),
                signal("sectionName"),
                raw(" &gt; "),
                signal("topicName"),
                raw("</nav>"),
            ]
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
        parser.set_dom_strategy(DomStrategy::Light);
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

        // Component template stream should contain the component content (no shadow DOM wrapper)
        let comp = &records["my-component"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                !raw.value.contains("<template shadowrootmode") && raw.value.contains("<div>My Component</div>"))
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
        parser.set_dom_strategy(DomStrategy::Light);
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

        assert_stream!(records, "custom-element", [raw("<slot></slot>"),]);
    }

    #[test]
    fn test_component_styled_no_double_wrap() {
        let mut parser = HtmlParser::new();
        parser.set_dom_strategy(DomStrategy::Light);
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

        assert_stream!(records, "custom-element", [raw(r#"<slot></slot>"#),]);
    }

    #[test]
    fn test_component_strip_runtime_attrs() {
        let mut parser = HtmlParser::new();
        parser.set_dom_strategy(DomStrategy::Light);
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

        assert_stream!(records, "custom-element", [raw("<slot></slot>"),]);
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
        parser.set_dom_strategy(DomStrategy::Light);
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
            [raw("<div>Custom Element</div>"),]
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
        parser.set_dom_strategy(DomStrategy::Light);
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

        assert_stream!(records, "custom-icon", [raw("<svg><slot></slot></svg>"),]);
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
    fn test_component_boolean_predicate_preserves_condition_tree() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("custom-element", "<slot></slot>", None)
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element ?disabled="{{page == 'dashboard'}}"></custom-element>"#,
        );
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let fragments = &records["index.html"].fragments;
        match fragments
            .get(1)
            .and_then(|fragment| fragment.fragment.as_ref())
        {
            Some(webui_protocol::web_ui_fragment::Fragment::Attribute(attr)) => {
                assert_eq!(attr.name, "disabled");
                assert!(attr.attr_start);
                match attr
                    .condition_tree
                    .as_ref()
                    .and_then(|condition| condition.expr.as_ref())
                {
                    Some(webui_protocol::condition_expr::Expr::Predicate(pred)) => {
                        assert_eq!(pred.left, "page");
                        assert_eq!(pred.operator, 3);
                        assert_eq!(pred.right, "'dashboard'");
                    }
                    other => panic!("expected predicate condition tree, got {:?}", other),
                }
            }
            other => panic!("expected attribute fragment, got {:?}", other),
        }
    }

    #[test]
    fn test_component_meta_link_tags() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<head><meta charset="utf-8" /><link rel="stylesheet" href="{{cssFile}}" /></head>"#,
        );
        assert!(fragments.len() >= 5);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<head><meta charset=\"utf-8\"") && raw.value.contains("<link"))
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Attribute(a)) if a.name == "href" && a.value == "cssFile")
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("/>"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "head_end" && s.raw)
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</head>"))
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

    /// Helper to parse HTML with a pre-registered component.
    fn parse_with_component(tag: &str, html: &str) -> (Vec<WebUIFragment>, WebUIFragmentRecords) {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(tag, "<div></div>", None)
            .expect("register");
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
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :config="{{settings}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":config", "settings"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_multiple_colon_prefixed() {
        // Port of: 'should process multiple colon-prefixed complex attributes'
        // <my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":prop1", "val1"),
                attr_complex(":prop2", "val2"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_blocked_complex_property_innerhtml() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", r#"<div :innerHTML="{{content}}"></div>"#);
        assert!(
            result.is_err(),
            "Expected error for :innerHTML on native element"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed on custom elements"),
            "Error: {err}"
        );
    }

    #[test]
    fn test_blocked_complex_on_native_element() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", r#"<div :data="{{config}}"></div>"#);
        assert!(
            result.is_err(),
            "Expected error for :data on native element"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed on custom elements"),
            "Error: {err}"
        );
    }

    #[test]
    fn test_blocked_complex_property_on_component() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("my-widget", "<div></div>", None)
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<my-widget :innerHTML="{{html}}"></my-widget>"#,
        );
        assert!(
            result.is_err(),
            "Expected error for :innerHTML on component"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("HTML injection"), "Error: {err}");
    }

    #[test]
    fn test_allowed_complex_property() {
        // :config on a component should still work
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :config="{{settings}}"></my-component>"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":config", "settings"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_mixed_normal_boolean_colon() {
        // Port of: 'should process mixed normal, boolean, and colon-prefixed attributes'
        // <my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_raw_start("id", "comp"),
                attr_complex(":config", "settings"),
                bool_attr("enabled", "isEnabled"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
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
    fn test_component_attr_skip_static_and_embedded() {
        // Skipped attrs with static values and embedded bindings should
        // emit fragments (not be silently dropped).
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component("item-group", "<slot></slot>", None)
            .expect("register");

        let html = r#"<item-group role="list" aria-labelledby="group-date-{{group.id}}" data-testid="grp-{{group.id}}" class="fixed-class"></item-group>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<item-group"),
                attr_skip_raw("role", "list"),
                attr_skip_template("aria-labelledby", "attr-1"),
                attr_skip_template("data-testid", "attr-2"),
                attr_skip_raw("class", "fixed-class"),
                raw(">"),
                component("item-group"),
                raw("</item-group>"),
            ]
        );

        // Verify the embedded-binding template sub-streams exist and
        // contain the expected static + signal fragments.
        assert_stream!(records, "attr-1", [raw("group-date-"), signal("group.id"),]);
        assert_stream!(records, "attr-2", [raw("grp-"), signal("group.id"),]);
    }

    #[test]
    fn test_component_multiple_nested() {
        // Port of: 'handle multiple nested web components'
        let mut parser = HtmlParser::new();
        parser.set_dom_strategy(DomStrategy::Light);
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
            matches!(ce[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.starts_with("<custom-child>"))
        );
        assert!(
            matches!(ce[1].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-child")
        );
        assert!(
            matches!(ce[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</custom-child><slot></slot>"))
        );

        assert_stream!(records, "custom-button", [raw("<slot></slot>"),]);

        assert_stream!(records, "custom-child", [raw("<h1>Hello World!</h1>"),]);
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

        // DOCTYPE + head content, head_end, </head><body>, body_start, body content, body_end, </body></html>
        assert!(fragments.len() >= 7);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE HTML>") && raw.value.contains("<title>Test</title>"))
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "head_end" && s.raw)
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</head>") && raw.value.ends_with("<body>"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_start" && s.raw)
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<app-shell>"))
        );
        assert!(
            matches!(fragments[5].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_end" && s.raw)
        );
        assert!(
            matches!(fragments[6].fragment.as_ref(), Some(Fragment::Raw(raw)) if
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

    #[test]
    fn test_css_strategy_module_emits_adopted_stylesheets() {
        let mut parser = HtmlParser::new();
        parser.set_dom_strategy(DomStrategy::Light);
        parser.set_css_strategy(CssStrategy::Module);
        parser
            .component_registry_mut()
            .register_component("my-card", "<p><slot></slot></p>", Some("p { color: red; }"))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();

        // Component template should have shadowrootadoptedstylesheets, no CSS
        // module in raw fragments (CSS lives on the component fragment's css field)
        let my_card = &records["my-card"].fragments;
        let template_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !template_text.contains("shadowrootadoptedstylesheets"),
            "shadowrootadoptedstylesheets should NOT be in HTML output (now in metadata only): {template_text}"
        );
        assert!(
            !template_text.contains("<link"),
            "Should not have <link> in module mode: {template_text}"
        );
        // No CSS module baked into raw fragments — CSS is stored in
        // protocol.components, populated by the build system.
        assert!(
            !template_text.contains(r#"<style type="module""#),
            "CSS module should NOT be in raw fragments: {template_text}"
        );
    }

    #[test]
    fn test_css_strategy_module_no_css_no_adopted_attr() {
        let mut parser = HtmlParser::new();
        parser.set_css_strategy(CssStrategy::Module);
        parser
            .component_registry_mut()
            .register_component("my-card", "<p><slot></slot></p>", None)
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let template_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        // No CSS → no shadowrootadoptedstylesheets attribute
        assert!(
            !template_text.contains("shadowrootadoptedstylesheets"),
            "Should not have adopted attr without CSS: {template_text}"
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
                raw("<html><head><title>Test</title>"),
                signal_raw("head_end"),
                raw("</head><body>"),
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

        // Should have: raw(DOCTYPE+head content), head_end, raw(</head><body>),
        // body_start, raw(body content), body_end, raw(</body></html>)
        assert!(
            fragments.len() >= 7,
            "Expected at least 7 fragments, got {}",
            fragments.len()
        );

        // First fragment: DOCTYPE through head content (before </head>)
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE html>") &&
                raw.value.contains("<meta charset=\"utf-8\">") &&
                raw.value.contains("<meta name=\"viewport\"") &&
                raw.value.contains("<title>Complex Page</title>") &&
                raw.value.contains("<style>") &&
                raw.value.contains("body { margin: 0; padding: 0; }")),
            "First fragment should contain all head content, got: {:?}",
            fragments[0]
        );

        // head_end signal
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "head_end" && s.raw),
            "Second fragment should be head_end signal"
        );

        // </head><body>
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</head>") && raw.value.ends_with("<body>")),
            "Third fragment should contain </head><body>"
        );

        // body_start signal
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_start" && s.raw),
            "Fourth fragment should be body_start signal"
        );

        // Body content (h1 and script)
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<h1>Hello World</h1>") &&
                raw.value.contains("<script")),
            "Fifth fragment should contain body content"
        );

        // body_end signal
        assert!(
            matches!(fragments[5].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.value == "body_end" && s.raw),
            "Sixth fragment should be body_end signal"
        );

        // Closing tags
        assert!(
            matches!(fragments[6].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</body>") && raw.value.contains("</html>")),
            "Seventh fragment should contain closing tags"
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
        fn register_component_template(
            &mut self,
            _tag_name: &str,
            _component: &Component,
            _processed_template: &str,
        ) -> Result<()> {
            Ok(())
        }

        fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction {
            if attr_name.starts_with('@') || attr_name == "f-ref" {
                AttributeAction::SkipAndCountBinding
            } else {
                AttributeAction::Keep
            }
        }

        fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
            self.counts.push(binding_attribute_count);
            if binding_attribute_count > 0 {
                Some(binding_attribute_count.to_le_bytes().to_vec())
            } else {
                None
            }
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
        let html = r#"<route path="/profile" component="profile-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Routes are emitted as Fragment::Route
        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 1);
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile");
                assert_eq!(r.fragment_id, "profile-page");
                assert!(r.exact);
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_route_with_params() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/profile/:id/view/:section" component="detail" />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Route is emitted as Fragment::Route with correct path
        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile/:id/view/:section");
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
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
        let html = r#"<route path="/" component="app-layout" />
            <route path="/dashboard" component="dash-page" exact />
            <route path="/contacts" component="contacts-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 3);
        // All should be Fragment::Route
        for frag in frags {
            assert!(matches!(
                frag.fragment.as_ref(),
                Some(web_ui_fragment::Fragment::Route(_))
            ));
        }
    }

    #[test]
    fn test_parse_route_requires_component_with_body() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/error">
            <div class="error"><h1>Not Found</h1></div>
        </route>"#;
        let result = parser.parse("test.html", html);
        assert!(
            result.is_err(),
            "Route without component attribute should fail"
        );
    }

    #[test]
    fn test_parse_multiple_routes_with_fragments() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" component="home-page" exact />
            <route path="/about" component="about-page" exact />
            <route path="/contact/:id" component="contact-page" />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 3);

        // Verify individual routes
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[0].fragment.as_ref() {
            assert_eq!(r.path, "/");
        }
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[1].fragment.as_ref() {
            assert_eq!(r.path, "/about");
        }
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[2].fragment.as_ref() {
            assert_eq!(r.path, "/contact/:id");
        }
    }

    #[test]
    fn test_outlet_not_captured_by_for_loop() {
        let mut parser = HtmlParser::new();
        let html = r#"<template shadowrootmode="open">
  <ul>
    <for each="item in items">
      <li>{{item.name}}</li>
    </for>
  </ul>
  <main>
    <outlet />
  </main>
</template>"#;
        parser.parse("comp.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["comp.html"].fragments;

        // Print fragment types for debugging
        let frag_types: Vec<String> = frags
            .iter()
            .map(|f| match f.fragment.as_ref() {
                Some(web_ui_fragment::Fragment::Raw(r)) => {
                    format!("Raw({:?})", &r.value[..r.value.len().min(40)])
                }
                Some(web_ui_fragment::Fragment::ForLoop(fl)) => {
                    format!("ForLoop({})", fl.fragment_id)
                }
                Some(web_ui_fragment::Fragment::Outlet(_)) => "Outlet".to_string(),
                other => format!("{:?}", other),
            })
            .collect();
        eprintln!("Fragment order: {:#?}", frag_types);

        // The outlet should be a top-level fragment, NOT inside the for-loop's body
        let outlet_count = frags
            .iter()
            .filter(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            })
            .count();
        assert_eq!(
            outlet_count, 1,
            "expected exactly 1 outlet in top-level fragments, got {outlet_count}. Fragments: {frag_types:?}"
        );

        // Verify outlet comes AFTER the raw "</ul>" text
        let outlet_idx = frags
            .iter()
            .position(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            })
            .expect("no outlet found");
        let close_ul_idx = frags.iter().position(|f| match f.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Raw(r)) => r.value.contains("</ul>"),
            _ => false,
        });
        if let Some(ul_idx) = close_ul_idx {
            assert!(
                outlet_idx > ul_idx,
                "outlet (at {outlet_idx}) should come after </ul> (at {ul_idx}). Fragments: {frag_types:?}"
            );
        }
    }

    #[test]
    fn test_outlet_position_after_for_not_inside() {
        let mut parser = HtmlParser::new();
        let html = r#"<ul><for each="x in items"><li>ok</li></for></ul><outlet />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;

        // Outlet should be its own fragment at top level
        let has_outlet = frags.iter().any(|f| {
            matches!(
                f.fragment.as_ref(),
                Some(web_ui_fragment::Fragment::Outlet(_))
            )
        });
        assert!(
            has_outlet,
            "outlet should be in top-level fragments: {frags:?}"
        );

        // The for-loop body should NOT contain the outlet
        let for_id = frags.iter().find_map(|f| match f.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::ForLoop(fl)) => Some(fl.fragment_id.clone()),
            _ => None,
        });
        if let Some(id) = for_id {
            let for_frags = &records[&id].fragments;
            let outlet_in_for = for_frags.iter().any(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            });
            assert!(
                !outlet_in_for,
                "outlet should NOT be inside for-loop body: {for_frags:?}"
            );
        }
    }

    #[test]
    fn test_style_element_with_handlebars_signal() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
:root {
    /*{{{tokens.light}}}*/
}
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        // Comment-delimited signal: the /* */ are stripped, signal emitted.
        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>\n:root {\n    "),
                signal_raw("tokens.light"),
                raw("\n}\n</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_signal_with_spaces() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
:root {
    /* {{{tokens.light}}} */
}
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        // Spaces inside comment are trimmed before parsing.
        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>\n:root {\n    "),
                signal_raw("tokens.light"),
                raw("\n}\n</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_signal_double_brace() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>/*{{themeCss}}*/</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        // Double-brace in a comment is also valid.
        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>"),
                signal("themeCss"),
                raw("</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_with_extra_text_stays_raw() {
        let mut parser = HtmlParser::new();
        // Extra text around the signal inside the comment → not a placeholder
        let html = "<html><head><style>/* theme: {{token}} */</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>/* theme: {{token}} */</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_with_multiple_signals_stays_raw() {
        let mut parser = HtmlParser::new();
        // Two signals in one comment → not a placeholder
        let html = "<html><head><style>/*{{a}}{{b}}*/</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>/*{{a}}{{b}}*/</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_bare_handlebars_stays_raw() {
        let mut parser = HtmlParser::new();
        // Handlebars outside a CSS comment must NOT be parsed as signals.
        let html =
            "<html><head><style>body { color: {{textColor}}; }</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>body { color: {{textColor}}; }</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_mixed_css_and_comment_signal() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
  .a { color: red; }
  /*{{themeCss}}*/
  .b { color: blue; }
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>\n  .a { color: red; }\n  "),
                signal("themeCss"),
                raw("\n  .b { color: blue; }\n</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_element_plain_css_unchanged() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>body { margin: 0; }</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        // Plain CSS with no handlebars should remain as a single raw fragment.
        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head><style>body { margin: 0; }</style>"),
                signal_raw("head_end"),
                raw("</head><body>"),
                signal_raw("body_start"),
                signal_raw("body_end"),
                raw("</body></html>"),
            ]
        );
    }
}
