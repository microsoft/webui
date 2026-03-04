//! FAST Hydration plugin for the WebUI handler.
//!
//! Injects HTML comment markers and data attributes that enable client-side
//! FAST-HTML to locate and re-hydrate server-rendered dynamic content.
//!
//! ## Comment Format
//!
//! - **Binding start**: `<!--fe-b$$start$$INDEX$$NAME$$fe-b-->`
//! - **Binding end**: `<!--fe-b$$end$$INDEX$$NAME$$fe-b-->`
//! - **Repeat item start**: `<!--fe-repeat$$start$$INDEX$$fe-repeat-->`
//! - **Repeat item end**: `<!--fe-repeat$$end$$INDEX$$fe-repeat-->`
//! - **Attribute single**: ` data-fe-b-INDEX`
//! - **Attribute multi**: ` data-fe-c-INDEX-COUNT`
//!
//! ## Scoping
//!
//! Each component and for-loop item gets its own scope with an independent
//! binding counter starting from 0. This matches the C++ prototype behavior.

use super::HandlerPlugin;
use crate::{ResponseWriter, Result};
use std::fmt::Write;

// Comment format constants
const BINDING_START_PREFIX: &str = "<!--fe-b$$start$$";
const BINDING_END_PREFIX: &str = "<!--fe-b$$end$$";
const BINDING_SUFFIX: &str = "$$fe-b-->";
const SEPARATOR: &str = "$$";
const REPEAT_START_PREFIX: &str = "<!--fe-repeat$$start$$";
const REPEAT_END_PREFIX: &str = "<!--fe-repeat$$end$$";
const REPEAT_SUFFIX: &str = "$$fe-repeat-->";
const ATTR_SINGLE_PREFIX: &str = " data-fe-b-";
const ATTR_MULTI_PREFIX: &str = " data-fe-c-";

/// FAST Hydration handler plugin.
///
/// Emits HTML comment markers around dynamic bindings so that FAST-HTML
/// can re-hydrate server-rendered content on the client side.
///
/// The root scope is disabled (no markers) — hydration only activates in
/// child scopes (components, for-loop items, if-condition bodies).
/// This matches the C++ and JS prototype behavior.
pub struct FastHydrationPlugin {
    /// Stack of local binding counters (one per scope).
    /// The bottom of the stack is the root scope (disabled).
    scopes: Vec<usize>,
    /// Stack of binding indices for matching start/end pairs.
    binding_stack: Vec<usize>,
    /// Reusable buffer for formatting markers without allocation.
    buffer: String,
}

impl FastHydrationPlugin {
    /// Create a new FAST hydration plugin.
    /// The initial root scope is disabled — markers only emitted in child scopes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Root scope (index 0) is disabled — only scopes.len() > 1 are active
            scopes: vec![0],
            binding_stack: Vec::with_capacity(8),
            buffer: String::with_capacity(64),
        }
    }

    /// Whether the current scope is active (not the root scope).
    fn is_active(&self) -> bool {
        self.scopes.len() > 1
    }

    /// Get the next binding index in the current scope, advancing the counter.
    fn next_index(&mut self) -> usize {
        if let Some(counter) = self.scopes.last_mut() {
            let index = *counter;
            *counter += 1;
            index
        } else {
            0
        }
    }

    /// Get the next binding index, advancing the counter by `count`.
    fn next_index_n(&mut self, count: u32) -> usize {
        if let Some(counter) = self.scopes.last_mut() {
            let index = *counter;
            *counter += count as usize;
            index
        } else {
            0
        }
    }

    /// Build a binding comment into the reusable buffer.
    fn build_binding_comment(&mut self, prefix: &str, index: usize, name: &str) {
        self.buffer.clear();
        self.buffer.push_str(prefix);
        let _ = write!(self.buffer, "{}", index);
        self.buffer.push_str(SEPARATOR);
        self.buffer.push_str(name);
        self.buffer.push_str(BINDING_SUFFIX);
    }

    /// Build a repeat comment into the reusable buffer.
    fn build_repeat_comment(&mut self, prefix: &str, index: usize) {
        self.buffer.clear();
        self.buffer.push_str(prefix);
        let _ = write!(self.buffer, "{}", index);
        self.buffer.push_str(REPEAT_SUFFIX);
    }

    /// Build an attribute binding marker into the reusable buffer.
    fn build_attribute_marker(&mut self, binding_index: usize, count: u32) {
        self.buffer.clear();
        if count == 1 {
            self.buffer.push_str(ATTR_SINGLE_PREFIX);
            let _ = write!(self.buffer, "{}", binding_index);
        } else {
            self.buffer.push_str(ATTR_MULTI_PREFIX);
            let _ = write!(self.buffer, "{}-{}", binding_index, count);
        }
    }
}

impl Default for FastHydrationPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerPlugin for FastHydrationPlugin {
    fn push_scope(&mut self) {
        self.scopes.push(0);
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let index = self.next_index();
        self.binding_stack.push(index);
        self.build_binding_comment(BINDING_START_PREFIX, index, name);
        writer.write(&self.buffer)
    }

    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let index = self.binding_stack.pop().unwrap_or(0);
        self.build_binding_comment(BINDING_END_PREFIX, index, name);
        writer.write(&self.buffer)
    }

    fn on_repeat_item_start(
        &mut self,
        index: usize,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        self.build_repeat_comment(REPEAT_START_PREFIX, index);
        writer.write(&self.buffer)
    }

    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        self.build_repeat_comment(REPEAT_END_PREFIX, index);
        writer.write(&self.buffer)
    }

    fn on_plugin_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        // FAST hydration protocol: data is a u32 LE attribute count
        if data.len() >= 4 {
            let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            if count > 0 {
                let binding_index = self.next_index_n(count);
                self.build_attribute_marker(binding_index, count);
                writer.write(&self.buffer)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestWriter {
        output: String,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                output: String::new(),
            }
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            Ok(())
        }
        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_root_scope_disabled() {
        let mut plugin = FastHydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Root scope should not emit markers
        plugin.on_binding_start("x", &mut writer).unwrap();
        plugin.on_binding_end("x", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        let data = 3u32.to_le_bytes();
        plugin.on_plugin_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_binding_start_format() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$0$$userName$$fe-b-->");
    }

    #[test]
    fn test_binding_end_format() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_end("userName", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$end$$0$$userName$$fe-b-->");
    }

    #[test]
    fn test_binding_index_increments() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_start("b", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$1$$b$$fe-b-->");
    }

    #[test]
    fn test_scope_resets_counter() {
        let mut plugin = FastHydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Push first active scope (root is disabled)
        plugin.push_scope();
        // Active scope: index 0
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        // Push child scope: counter resets to 0
        plugin.push_scope();
        writer.output.clear();
        plugin.on_binding_start("b", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$0$$b$$fe-b-->");
        plugin.on_binding_end("b", &mut writer).unwrap();
        // Pop child scope: back to parent (counter was at 1)
        plugin.pop_scope();
        writer.output.clear();
        plugin.on_binding_start("c", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$1$$c$$fe-b-->");
    }

    #[test]
    fn test_repeat_item_markers() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-repeat$$start$$0$$fe-repeat-->");
        writer.output.clear();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-repeat$$end$$0$$fe-repeat-->");
    }

    #[test]
    fn test_attribute_binding_single() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 1u32.to_le_bytes();
        plugin.on_plugin_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-b-0");
    }

    #[test]
    fn test_attribute_binding_multi() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 3u32.to_le_bytes();
        plugin.on_plugin_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-c-0-3");
    }

    #[test]
    fn test_attribute_binding_zero_count_no_output() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 0u32.to_le_bytes();
        plugin.on_plugin_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_attribute_binding_advances_counter() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        // 3 attributes → counter goes from 0 to 3
        let data = 3u32.to_le_bytes();
        plugin.on_plugin_data(&data, &mut writer).unwrap();
        // Next binding should be at index 3
        writer.output.clear();
        plugin.on_binding_start("x", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$3$$x$$fe-b-->");
    }

    #[test]
    fn test_nested_scopes_independent_counters() {
        let mut plugin = FastHydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Push first active scope (root is disabled)
        plugin.push_scope();
        // Active scope: binding 0
        plugin.on_binding_start("root", &mut writer).unwrap();
        plugin.on_binding_end("root", &mut writer).unwrap();
        // Component scope
        plugin.push_scope();
        // For-loop binding in component: index 0 (new scope)
        writer.output.clear();
        plugin.on_binding_start("for-1", &mut writer).unwrap();
        assert!(writer.output.contains("start$$0$$for-1"));
        // For-loop item scope
        plugin.push_scope();
        writer.output.clear();
        plugin.on_binding_start("signal", &mut writer).unwrap();
        assert!(writer.output.contains("start$$0$$signal"));
        plugin.on_binding_end("signal", &mut writer).unwrap();
        plugin.pop_scope();
        plugin.on_binding_end("for-1", &mut writer).unwrap();
        plugin.pop_scope();
        // Back to first active scope: counter should be at 1
        writer.output.clear();
        plugin.on_binding_start("root2", &mut writer).unwrap();
        assert!(writer.output.contains("start$$1$$root2"));
    }

    #[test]
    fn test_empty_plugin_data_ignored() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_plugin_data(&[], &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_short_plugin_data_ignored() {
        let mut plugin = FastHydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_plugin_data(&[1, 2], &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    // ── Integration tests (full render cycles with WebUIHandler) ────────

    use std::collections::HashMap;
    use webui_protocol::{
        web_ui_fragment, ConditionExpr, FragmentList, LogicalOperator, WebUIFragment,
        WebUIFragmentAttribute, WebUIProtocol,
    };
    use webui_test_utils::test_json;

    use crate::WebUIHandler;

    fn render_with_plugin(
        protocol: &WebUIProtocol,
        state: &serde_json::Value,
        plugin: Box<dyn HandlerPlugin>,
    ) -> String {
        let mut writer = TestWriter::new();
        let mut handler = WebUIHandler::with_plugin(plugin);
        handler.handle(protocol, state, &mut writer).unwrap();
        writer.output
    }

    fn render_no_plugin(protocol: &WebUIProtocol, state: &serde_json::Value) -> String {
        let mut writer = TestWriter::new();
        let mut handler = WebUIHandler::new();
        handler.handle(protocol, state, &mut writer).unwrap();
        writer.output
    }

    #[test]
    fn test_no_plugin_no_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"name": "Alice"});
        let output = render_no_plugin(&protocol, &state);
        assert_eq!(output, "<p>Alice</p>");
        assert!(!output.contains("fe-b"));
    }

    #[test]
    fn test_hydration_signal_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"name": "Alice"});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        // Root scope is disabled — no markers at root level
        assert_eq!(output, "<p>Alice</p>");
    }

    #[test]
    fn test_hydration_for_loop_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "for-1")],
            },
        );
        fragments.insert(
            "for-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item", false)],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": ["a", "b"]});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        // Root scope disabled — no for-loop binding or repeat item markers
        assert!(!output.contains("$$for-1$$"));
        assert!(!output.contains("fe-repeat"));
        // Signal bindings inside each item ARE emitted (for-loop items push scope)
        let item_pattern = "<!--fe-b$$start$$0$$item$$fe-b-->";
        assert_eq!(output.matches(item_pattern).count(), 2);
    }

    #[test]
    fn test_hydration_if_condition_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("show"),
                    "if-1",
                )],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Visible</p>")],
            },
        );
        let protocol = WebUIProtocol { fragments };

        // True case — root scope disabled, no markers; content still rendered
        let state = test_json!({"show": true});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        assert!(output.contains("<p>Visible</p>"));
        assert!(!output.contains("fe-b"));

        // False case — no content, no markers
        let state = test_json!({"show": false});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        assert!(!output.contains("<p>Visible</p>"));
        assert!(!output.contains("fe-b"));
    }

    #[test]
    fn test_hydration_component_scope_reset() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("before", false),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::signal("after", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("inner", false)],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"before": "B", "inner": "I", "after": "A"});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        // Root scope disabled — no markers for root-level signals
        assert!(!output.contains("$$before$$"));
        assert!(!output.contains("$$after$$"));
        // Inner signal in component scope gets markers (index 0)
        assert!(output.contains("<!--fe-b$$start$$0$$inner$$fe-b-->"));
        assert!(output.contains("<!--fe-b$$end$$0$$inner$$fe-b-->"));
    }

    #[test]
    fn test_hydration_plugin_data_fragment() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div"),
                    WebUIFragment::attribute("id", "itemId"),
                    WebUIFragment::attribute("title", "itemTitle"),
                    WebUIFragment::plugin(2u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">content</div>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"itemId": "42", "itemTitle": "Hello"});
        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));
        // Root scope disabled — no plugin data markers
        assert!(!output.contains("data-fe-c"));
        assert!(output.contains("id=\"42\""));
        assert!(output.contains("title=\"Hello\""));
    }

    #[test]
    fn test_hydration_no_markers_in_mixed_attribute_value() {
        // Port of C++ HydrationEnabledDoesNotInsertMarkersIntoMixedAttributeValue
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".to_string(),
                                template: "attr-title".to_string(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello "),
                    WebUIFragment::signal("name", false),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("content", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"name": "World", "content": "CONTENT"});

        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));

        // Hydration markers should exist in the output (around component content)
        assert!(
            output.contains("<!--fe-b"),
            "Expected hydration markers in output"
        );

        // The attribute value must NOT contain hydration markers
        assert!(
            output.contains("title=\"Hello World\""),
            "Expected clean attribute value without markers, got: {output}"
        );

        // Verify no markers leaked into the attribute
        let title_start = output.find("title=\"").unwrap();
        let title_end = output[title_start..].find('"').unwrap()
            + output[title_start + 7..].find('"').unwrap()
            + 7;
        let title_value = &output[title_start..title_start + title_end + 1];
        assert!(
            !title_value.contains("fe-b"),
            "Hydration markers leaked into attribute value: {title_value}"
        );
    }

    #[test]
    fn test_hydration_nested_for_if_streams_full() {
        // Port of C++ HydrationEnabledWithNestedForAndIfStreams
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("hydratableComponent")],
            },
        );
        fragments.insert(
            "hydratableComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::for_loop("category", "categories", "categoryTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "categoryTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section"),
                    WebUIFragment::attribute("data-category", "category.name"),
                    WebUIFragment::plugin(1u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("category.title", false),
                    // NodeJS: binary(identifier('category.hasItems'), '&&', identifier('category.alwaysTrue'))
                    WebUIFragment::if_cond(
                        ConditionExpr::compound(
                            ConditionExpr::identifier("category.hasItems"),
                            LogicalOperator::And,
                            ConditionExpr::identifier("category.alwaysTrue"),
                        ),
                        "itemsTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "itemsTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<ul>"),
                    WebUIFragment::for_loop("item", "category.items", "itemTemplate"),
                    WebUIFragment::raw("</ul>"),
                ],
            },
        );
        fragments.insert(
            "itemTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<li"),
                    WebUIFragment::attribute_template("id", "itemIdAttr"),
                    WebUIFragment::attribute("data-name", "item.name"),
                    WebUIFragment::plugin(2u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("item.special"),
                        "specialTemplate",
                    ),
                    WebUIFragment::raw("</li>"),
                ],
            },
        );
        fragments.insert(
            "itemIdAttr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("item-"),
                    WebUIFragment::signal("item.id", false),
                ],
            },
        );
        fragments.insert(
            "specialTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw(" ("),
                    WebUIFragment::signal("item.specialText", false),
                    WebUIFragment::raw(")"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "title": "My Store",
            "categories": [
                {
                    "name": "electronics",
                    "title": "Electronics",
                    "hasItems": true,
                    "alwaysTrue": true,
                    "items": [
                        {"id": "1", "name": "Laptop", "special": true, "specialText": "On Sale"},
                        {"id": "2", "name": "Phone", "special": false}
                    ]
                },
                {"name": "books", "title": "Books", "hasItems": false},
                {"name": "toys", "title": "Toys", "hasItems": true, "alwaysTrue": true, "items": []}
            ]
        });

        let output = render_with_plugin(&protocol, &state, Box::new(FastHydrationPlugin::new()));

        let expected = "\
            <div>\
            <!--fe-b$$start$$0$$title$$fe-b-->My Store<!--fe-b$$end$$0$$title$$fe-b-->\
            <!--fe-b$$start$$1$$categoryTemplate$$fe-b-->\
            <!--fe-repeat$$start$$0$$fe-repeat-->\
            <section data-category=\"electronics\" data-fe-b-0>\
            <!--fe-b$$start$$1$$category.title$$fe-b-->Electronics<!--fe-b$$end$$1$$category.title$$fe-b-->\
            <!--fe-b$$start$$2$$itemsTemplate$$fe-b-->\
            <ul>\
            <!--fe-b$$start$$0$$itemTemplate$$fe-b-->\
            <!--fe-repeat$$start$$0$$fe-repeat-->\
            <li id=\"item-1\" data-name=\"Laptop\" data-fe-c-0-2>\
            <!--fe-b$$start$$2$$item.name$$fe-b-->Laptop<!--fe-b$$end$$2$$item.name$$fe-b-->\
            <!--fe-b$$start$$3$$specialTemplate$$fe-b--> \
            (<!--fe-b$$start$$0$$item.specialText$$fe-b-->On Sale<!--fe-b$$end$$0$$item.specialText$$fe-b-->)\
            <!--fe-b$$end$$3$$specialTemplate$$fe-b-->\
            </li>\
            <!--fe-repeat$$end$$0$$fe-repeat-->\
            <!--fe-repeat$$start$$1$$fe-repeat-->\
            <li id=\"item-2\" data-name=\"Phone\" data-fe-c-0-2>\
            <!--fe-b$$start$$2$$item.name$$fe-b-->Phone<!--fe-b$$end$$2$$item.name$$fe-b-->\
            <!--fe-b$$start$$3$$specialTemplate$$fe-b-->\
            <!--fe-b$$end$$3$$specialTemplate$$fe-b-->\
            </li>\
            <!--fe-repeat$$end$$1$$fe-repeat-->\
            <!--fe-b$$end$$0$$itemTemplate$$fe-b-->\
            </ul>\
            <!--fe-b$$end$$2$$itemsTemplate$$fe-b-->\
            </section>\
            <!--fe-repeat$$end$$0$$fe-repeat-->\
            <!--fe-repeat$$start$$1$$fe-repeat-->\
            <section data-category=\"books\" data-fe-b-0>\
            <!--fe-b$$start$$1$$category.title$$fe-b-->Books<!--fe-b$$end$$1$$category.title$$fe-b-->\
            <!--fe-b$$start$$2$$itemsTemplate$$fe-b-->\
            <!--fe-b$$end$$2$$itemsTemplate$$fe-b-->\
            </section>\
            <!--fe-repeat$$end$$1$$fe-repeat-->\
            <!--fe-repeat$$start$$2$$fe-repeat-->\
            <section data-category=\"toys\" data-fe-b-0>\
            <!--fe-b$$start$$1$$category.title$$fe-b-->Toys<!--fe-b$$end$$1$$category.title$$fe-b-->\
            <!--fe-b$$start$$2$$itemsTemplate$$fe-b-->\
            <ul>\
            <!--fe-b$$start$$0$$itemTemplate$$fe-b-->\
            <!--fe-b$$end$$0$$itemTemplate$$fe-b-->\
            </ul>\
            <!--fe-b$$end$$2$$itemsTemplate$$fe-b-->\
            </section>\
            <!--fe-repeat$$end$$2$$fe-repeat-->\
            <!--fe-b$$end$$1$$categoryTemplate$$fe-b-->\
            </div>";

        assert_eq!(output, expected);
    }
}
