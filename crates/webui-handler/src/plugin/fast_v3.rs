// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST 3 hydration plugin for the WebUI handler.
//!
//! Emits FAST 3 HTML comment markers and data attributes that enable client-side
//! FAST declarative templates to locate and re-hydrate server-rendered dynamic
//! content. FAST 2 compatibility is implemented separately in `fast_v2`.
//!
//! ## FAST 3 Comment Format
//!
//! - **Binding start**: `<!--fe:b-->`
//! - **Binding end**: `<!--fe:/b-->`
//! - **Repeat item start**: `<!--fe:r-->`
//! - **Repeat item end**: `<!--fe:/r-->`
//! - **Attribute bindings**: ` data-fe="COUNT"`
use super::HandlerPlugin;
use crate::{HandlerError, ResponseWriter, Result};
use serde_json::Value;
use std::fmt::Write;
use webui_protocol::FastElementData;

// FAST 3 comment format constants
const BINDING_START_MARKER: &str = "<!--fe:b-->";
const BINDING_END_MARKER: &str = "<!--fe:/b-->";
const REPEAT_START_MARKER: &str = "<!--fe:r-->";
const REPEAT_END_MARKER: &str = "<!--fe:/r-->";
const ATTR_PREFIX: &str = " data-fe=\"";
const ATTR_SUFFIX: &str = "\"";

/// FAST 3 hydration handler plugin.
///
/// Emits HTML comment markers around dynamic bindings so that FAST
/// can re-hydrate server-rendered content on the client side.
///
/// The root scope is disabled (no markers) — hydration only activates in
/// child scopes (components, for-loop items, if-condition bodies).
/// This matches the C++ and JS prototype behavior.
pub struct FastV3HydrationPlugin {
    /// Stack of local binding counters (one per scope).
    /// The bottom of the stack is the root scope (disabled).
    scopes: Vec<usize>,
    /// Reusable buffer for formatting markers without allocation.
    buffer: String,
}

impl FastV3HydrationPlugin {
    /// Create a new FAST 3 hydration plugin.
    /// The initial root scope is disabled — markers only emitted in child scopes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Root scope (index 0) is disabled — only scopes.len() > 1 are active
            scopes: vec![0],
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

    /// Build an attribute binding marker into the reusable buffer.
    fn build_attribute_marker(&mut self, count: u32) {
        self.buffer.clear();
        self.buffer.push_str(ATTR_PREFIX);
        let _ = write!(self.buffer, "{}", count);
        self.buffer.push_str(ATTR_SUFFIX);
    }
}

impl Default for FastV3HydrationPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerPlugin for FastV3HydrationPlugin {
    fn push_scope(&mut self) {
        self.scopes.push(0);
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn on_binding_start(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let _ = self.next_index();
        writer.write(BINDING_START_MARKER)
    }

    fn on_binding_end(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        writer.write(BINDING_END_MARKER)
    }

    fn on_repeat_item_start(
        &mut self,
        _index: usize,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        writer.write(REPEAT_START_MARKER)
    }

    fn on_repeat_item_end(&mut self, _index: usize, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        writer.write(REPEAT_END_MARKER)
    }

    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let decoded = FastElementData::decode(data).map_err(|error| {
            HandlerError::PluginData(format!(
                "FAST hydration plugin expected 4 bytes of element data: {error}"
            ))
        })?;
        if decoded.binding_count > 0 {
            let _ = self.next_index_n(decoded.binding_count);
            self.build_attribute_marker(decoded.binding_count);
            writer.write(&self.buffer)?;
        }
        Ok(())
    }

    /// FAST emits scalar attributes + `data-state` JSON on route component elements.
    /// Components read these via `@attr` and their connection lifecycle.
    fn write_route_component_state(
        &self,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        write_fast_route_component_state(state, writer)
    }
}

fn write_fast_route_component_state(state: &Value, writer: &mut dyn ResponseWriter) -> Result<()> {
    let map = match state.as_object() {
        Some(m) => m,
        None => return Ok(()),
    };

    // Emit scalar values as individual kebab-case attributes.
    for (key, value) in map {
        let val_str = match value {
            Value::String(s) => std::borrow::Cow::Borrowed(s.as_str()),
            Value::Number(n) => std::borrow::Cow::Owned(n.to_string()),
            Value::Bool(true) => std::borrow::Cow::Borrowed("true"),
            Value::Bool(false) => std::borrow::Cow::Borrowed("false"),
            _ => continue,
        };
        let attr_name = webui_protocol::attrs::camel_to_kebab(key);
        writer.write(" ")?;
        writer.write(&attr_name)?;
        writer.write("=\"")?;
        crate::route_renderer::write_escaped_state_attr(writer, val_str.as_ref())?;
        writer.write("\"")?;
    }

    // Emit data-state JSON for complex values (arrays, objects).
    let has_complex = map.values().any(|v| v.is_array() || v.is_object());
    if has_complex {
        let json_str = state.to_string();
        writer.write(" data-state=\"")?;
        crate::route_renderer::write_escaped_state_attr(writer, &json_str)?;
        writer.write("\"")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

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
        let mut plugin = FastV3HydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Root scope should not emit markers
        plugin.on_binding_start("x", &mut writer).unwrap();
        plugin.on_binding_end("x", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_binding_start_format() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
    }

    #[test]
    fn test_binding_end_format() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_end("userName", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:/b-->");
    }

    #[test]
    fn test_binding_sequence_uses_compact_markers() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_start("b", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
    }

    #[test]
    fn test_scopes_emit_compact_markers() {
        let mut plugin = FastV3HydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Push first active scope (root is disabled)
        plugin.push_scope();
        // Active scope emits compact markers.
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        // Push child scope: markers are still emitted.
        plugin.push_scope();
        writer.output.clear();
        plugin.on_binding_start("b", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
        plugin.on_binding_end("b", &mut writer).unwrap();
        // Pop child scope: parent remains active.
        plugin.pop_scope();
        writer.output.clear();
        plugin.on_binding_start("c", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
    }

    #[test]
    fn test_repeat_item_markers() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:r-->");
        writer.output.clear();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:/r-->");
    }

    #[test]
    fn test_attribute_binding_single() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 1u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, r#" data-fe="1""#);
    }

    #[test]
    fn test_attribute_binding_multi() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, r#" data-fe="3""#);
    }

    #[test]
    fn test_attribute_binding_zero_count_no_output() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 0u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_attribute_binding_count_allows_following_binding() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, r#" data-fe="3""#);

        // Next binding still emits the compact sequential marker.
        writer.output.clear();
        plugin.on_binding_start("x", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
    }

    #[test]
    fn test_nested_scopes_emit_compact_markers() {
        let mut plugin = FastV3HydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Push first active scope (root is disabled)
        plugin.push_scope();
        // Active scope emits binding markers.
        plugin.on_binding_start("root", &mut writer).unwrap();
        plugin.on_binding_end("root", &mut writer).unwrap();
        // Component scope
        plugin.push_scope();
        // For-loop binding in component emits compact markers.
        writer.output.clear();
        plugin.on_binding_start("for-1", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
        // For-loop item scope
        plugin.push_scope();
        writer.output.clear();
        plugin.on_binding_start("signal", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
        plugin.on_binding_end("signal", &mut writer).unwrap();
        plugin.pop_scope();
        plugin.on_binding_end("for-1", &mut writer).unwrap();
        plugin.pop_scope();
        // Back to first active scope: compact markers are still emitted.
        writer.output.clear();
        plugin.on_binding_start("root2", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe:b-->");
    }

    #[test]
    fn test_empty_element_data_returns_error() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let result = plugin.on_element_data(&[], &mut writer);
        assert!(
            matches!(result, Err(crate::HandlerError::PluginData(ref msg)) if msg.contains("expected 4 bytes")),
            "invalid payload length should produce a plugin-data error: {result:?}"
        );
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_short_element_data_returns_error() {
        let mut plugin = FastV3HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let result = plugin.on_element_data(&[1, 2], &mut writer);
        assert!(
            matches!(result, Err(crate::HandlerError::PluginData(ref msg)) if msg.contains("expected 4 bytes")),
            "invalid payload length should produce a plugin-data error: {result:?}"
        );
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_write_route_component_state_emits_data_state() {
        let plugin = FastV3HydrationPlugin::new();
        let mut writer = TestWriter::new();
        let state = serde_json::json!({
            "title": "Hello",
            "items": [{"name": "A&B"}]
        });

        plugin
            .write_route_component_state(&state, &mut writer)
            .unwrap();

        assert!(
            writer.output.contains("data-state="),
            "FAST handler plugin should emit data-state: {}",
            writer.output
        );
        assert!(
            writer.output.contains(r#"title="Hello""#),
            "FAST handler plugin should still emit scalar attrs: {}",
            writer.output
        );
    }

    // ── Integration tests (full render cycles with WebUIHandler) ────────

    use std::collections::HashMap;
    use webui_protocol::{
        web_ui_fragment, ConditionExpr, FragmentList, LogicalOperator, WebUIFragment,
        WebUIFragmentAttribute, WebUIProtocol,
    };
    use webui_test_utils::test_json;

    use crate::{RenderOptions, WebUIHandler};

    fn render_with_plugin(
        protocol: &WebUIProtocol,
        state: &serde_json::Value,
        factory: fn() -> Box<dyn HandlerPlugin>,
    ) -> String {
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::with_plugin(factory);
        handler
            .handle(
                protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        writer.output
    }

    fn render_no_plugin(protocol: &WebUIProtocol, state: &serde_json::Value) -> String {
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::new();
        handler
            .handle(
                protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "Alice"});
        let output = render_no_plugin(&protocol, &state);
        assert_eq!(output, "<p>Alice</p>");
        assert!(!output.contains("<!--fe:"));
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "Alice"});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": ["a", "b"]});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        // Root scope disabled — no for-loop binding or repeat item markers
        assert!(!output.contains("<!--fe:r-->"));
        assert!(!output.contains("<!--fe:/r-->"));
        // Signal bindings inside each item ARE emitted (for-loop items push scope)
        assert_eq!(output, "<!--fe:b-->a<!--fe:/b--><!--fe:b-->b<!--fe:/b-->");
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
        let protocol = WebUIProtocol::new(fragments);

        // True case — root scope disabled, no markers; content still rendered
        let state = test_json!({"show": true});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        assert!(output.contains("<p>Visible</p>"));
        assert!(!output.contains("<!--fe:"));

        // False case — no content, no markers
        let state = test_json!({"show": false});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        assert!(!output.contains("<p>Visible</p>"));
        assert!(!output.contains("<!--fe:"));
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"before": "B", "inner": "I", "after": "A"});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        assert_eq!(output, "B<!--fe:b-->I<!--fe:/b-->A");
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"itemId": "42", "itemTitle": "Hello"});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        // Root scope disabled — no plugin data markers
        assert!(!output.contains("data-fe="));
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
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "World", "content": "CONTENT"});

        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));

        // Hydration markers should exist in the output (around component content)
        assert!(
            output.contains("<!--fe:b-->"),
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
            !title_value.contains("fe:"),
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
        let protocol = WebUIProtocol::new(fragments);
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

        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));

        let expected = "\
            <div>\
            <!--fe:b-->My Store<!--fe:/b-->\
            <!--fe:b-->\
            <!--fe:r-->\
            <section data-category=\"electronics\" data-fe=\"1\">\
            <!--fe:b-->Electronics<!--fe:/b-->\
            <!--fe:b-->\
            <ul>\
            <!--fe:b-->\
            <!--fe:r-->\
            <li id=\"item-1\" data-name=\"Laptop\" data-fe=\"2\">\
            <!--fe:b-->Laptop<!--fe:/b-->\
            <!--fe:b--> \
            (<!--fe:b-->On Sale<!--fe:/b-->)\
            <!--fe:/b-->\
            </li>\
            <!--fe:/r-->\
            <!--fe:r-->\
            <li id=\"item-2\" data-name=\"Phone\" data-fe=\"2\">\
            <!--fe:b-->Phone<!--fe:/b-->\
            <!--fe:b-->\
            <!--fe:/b-->\
            </li>\
            <!--fe:/r-->\
            <!--fe:/b-->\
            </ul>\
            <!--fe:/b-->\
            </section>\
            <!--fe:/r-->\
            <!--fe:r-->\
            <section data-category=\"books\" data-fe=\"1\">\
            <!--fe:b-->Books<!--fe:/b-->\
            <!--fe:b-->\
            <!--fe:/b-->\
            </section>\
            <!--fe:/r-->\
            <!--fe:r-->\
            <section data-category=\"toys\" data-fe=\"1\">\
            <!--fe:b-->Toys<!--fe:/b-->\
            <!--fe:b-->\
            <ul>\
            <!--fe:b-->\
            <!--fe:/b-->\
            </ul>\
            <!--fe:/b-->\
            </section>\
            <!--fe:/r-->\
            <!--fe:/b-->\
            </div>";

        assert_eq!(output, expected);
    }

    #[test]
    fn test_hydration_missing_signal_still_emits_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-comp")],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("missing_field", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        // Hydration comments must be emitted even when signal is not found in state
        assert!(
            output.contains("<!--fe:b-->"),
            "Expected binding start marker for missing signal, got: {output}"
        );
        assert!(
            output.contains("<!--fe:/b-->"),
            "Expected binding end marker for missing signal, got: {output}"
        );
        // Start and end markers should be adjacent (no content between them)
        assert!(output.contains("<!--fe:b--><!--fe:/b-->"));
    }

    #[test]
    fn test_hydration_missing_for_collection_still_emits_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-comp")],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<ul>"),
                    WebUIFragment::for_loop("item", "missing_items", "loop-body"),
                    WebUIFragment::raw("</ul>"),
                ],
            },
        );
        fragments.insert(
            "loop-body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        // Hydration comments must be emitted even when collection is missing from state
        assert!(
            output.contains("<!--fe:b-->"),
            "Expected binding start marker for missing collection, got: {output}"
        );
        assert!(
            output.contains("<!--fe:/b-->"),
            "Expected binding end marker for missing collection, got: {output}"
        );
    }

    #[test]
    fn test_hydration_empty_string_signal_still_emits_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-comp")],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": ""});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        assert!(
            output.contains("<!--fe:b-->"),
            "Expected binding start marker for empty string signal, got: {output}"
        );
        assert!(
            output.contains("<!--fe:/b-->"),
            "Expected binding end marker for empty string signal, got: {output}"
        );
        assert!(output.contains("<!--fe:b--><!--fe:/b-->"));
    }

    #[test]
    fn test_hydration_empty_collection_still_emits_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-comp")],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<ul>"),
                    WebUIFragment::for_loop("item", "items", "loop-body"),
                    WebUIFragment::raw("</ul>"),
                ],
            },
        );
        fragments.insert(
            "loop-body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": []});
        let output =
            render_with_plugin(&protocol, &state, || Box::new(FastV3HydrationPlugin::new()));
        assert!(
            output.contains("<!--fe:b-->"),
            "Expected binding start marker for empty collection, got: {output}"
        );
        assert!(
            output.contains("<!--fe:/b-->"),
            "Expected binding end marker for empty collection, got: {output}"
        );
    }

    #[test]
    fn test_for_if_hooks_delegate_to_binding_hooks() {
        // on_for_start/on_for_end should produce the same output as
        // on_binding_start/on_binding_end because the trait defaults delegate.
        {
            let mut plugin_for = FastV3HydrationPlugin::new();
            plugin_for.push_scope();
            let mut writer_for = TestWriter::new();
            plugin_for.on_for_start("items", &mut writer_for).unwrap();
            plugin_for.on_for_end("items", &mut writer_for).unwrap();

            let mut plugin_bind = FastV3HydrationPlugin::new();
            plugin_bind.push_scope();
            let mut writer_bind = TestWriter::new();
            plugin_bind
                .on_binding_start("items", &mut writer_bind)
                .unwrap();
            plugin_bind
                .on_binding_end("items", &mut writer_bind)
                .unwrap();

            assert_eq!(
                writer_for.output, writer_bind.output,
                "on_for_* should delegate to on_binding_*"
            );
        }

        // on_if_start/on_if_end should produce the same output as
        // on_binding_start/on_binding_end.
        {
            let mut plugin_if = FastV3HydrationPlugin::new();
            plugin_if.push_scope();
            let mut writer_if = TestWriter::new();
            plugin_if.on_if_start("visible", &mut writer_if).unwrap();
            plugin_if.on_if_end("visible", &mut writer_if).unwrap();

            let mut plugin_bind = FastV3HydrationPlugin::new();
            plugin_bind.push_scope();
            let mut writer_bind = TestWriter::new();
            plugin_bind
                .on_binding_start("visible", &mut writer_bind)
                .unwrap();
            plugin_bind
                .on_binding_end("visible", &mut writer_bind)
                .unwrap();

            assert_eq!(
                writer_if.output, writer_bind.output,
                "on_if_* should delegate to on_binding_*"
            );
        }
    }
}
