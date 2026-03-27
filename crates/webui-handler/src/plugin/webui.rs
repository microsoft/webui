// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI Framework handler plugin.
//!
//! # Overview
//!
//! Injects HTML comment markers and data attributes into server-rendered
//! output that enable the client-side WebUI Framework to locate and
//! re-hydrate dynamic content. The plugin operates within the handler's
//! scope-based lifecycle — each component and for-loop item gets its own
//! scope with an independent binding counter starting from 0.
//!
//! # Comment format
//!
//! | Marker type       | Format                                    | Purpose                          |
//! |-------------------|-------------------------------------------|----------------------------------|
//! | Binding start     | `<!--w-b:start:INDEX:NAME-->`             | Wraps a text binding             |
//! | Binding end       | `<!--w-b:end:INDEX:NAME-->`               | Closes the binding pair          |
//! | Repeat item start | `<!--w-r:start:INDEX-->`                  | Wraps one iteration of a loop    |
//! | Repeat item end   | `<!--w-r:end:INDEX-->`                    | Closes the iteration             |
//! | Attribute single  | ` data-w-b-INDEX`                         | One dynamic attribute binding    |
//! | Attribute multi   | ` data-w-c-INDEX-COUNT`                   | Multiple attribute bindings      |
//! | Event             | ` data-ev="COUNT"`                        | Wires COUNT events on one element |
//!
//! # Scoping
//!
//! - The **root scope** (depth 0) is disabled — no markers are emitted.
//!   This prevents markers from appearing in the page-level HTML outside
//!   any component.
//! - Each `push_scope` opens a new child scope with counter reset to 0.
//! - Each `pop_scope` returns to the parent scope's counter position.
//!
//! # Plugin data decoding
//!
//! The parser plugin emits 12-byte payloads per element:
//!
//! ```text
//! [binding_count: u32 LE | event_start_idx: u32 LE | event_count: u32 LE]
//! ```
//!
//! The handler decodes this in
//! [`on_element_data`](WebUIHydrationPlugin::on_element_data)
//! and emits `data-w-*` markers for bindings plus a single `data-ev="COUNT"`
//! marker per element. A 4-byte payload (bindings only) is also supported for
//! backward compatibility.

use super::HandlerPlugin;
use crate::{HandlerError, ResponseWriter, Result};
use std::fmt::Write;
use webui_protocol::{FastElementData, WebUIElementData};

// Comment format constants
const BINDING_START_PREFIX: &str = "<!--w-b:start:";
const BINDING_END_PREFIX: &str = "<!--w-b:end:";
const BINDING_SUFFIX: &str = "-->";
const SEPARATOR: &str = ":";
const REPEAT_START_PREFIX: &str = "<!--w-r:start:";
const REPEAT_END_PREFIX: &str = "<!--w-r:end:";
const REPEAT_SUFFIX: &str = "-->";
const ATTR_SINGLE_PREFIX: &str = " data-w-b-";
const ATTR_MULTI_PREFIX: &str = " data-w-c-";
const EVENT_PREFIX: &str = " data-ev=\"";
const EVENT_SUFFIX: &str = "\"";

/// WebUI Framework handler plugin.
///
/// Emits HTML comment markers around dynamic bindings so that the WebUI
/// Framework client runtime can re-hydrate server-rendered content.
///
/// The root scope is disabled (no markers) — hydration only activates in
/// child scopes (components, for-loop items, if-condition bodies).
pub struct WebUIHydrationPlugin {
    /// Stack of local binding counters (one per scope).
    /// The bottom of the stack is the root scope (disabled).
    scopes: Vec<usize>,
    /// Stack of binding indices for matching start/end pairs.
    binding_stack: Vec<usize>,
    /// Reusable buffer for formatting markers without allocation.
    buffer: String,
}

impl WebUIHydrationPlugin {
    /// Create a new WebUI Framework hydration plugin.
    /// The initial root scope is disabled — markers only emitted in child scopes.
    #[must_use]
    pub fn new() -> Self {
        Self {
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

    /// Build an event marker into the reusable buffer.
    fn build_event_marker(&mut self, count: u32) {
        self.buffer.clear();
        self.buffer.push_str(EVENT_PREFIX);
        let _ = write!(self.buffer, "{}", count);
        self.buffer.push_str(EVENT_SUFFIX);
    }
}

impl Default for WebUIHydrationPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerPlugin for WebUIHydrationPlugin {
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
        let index = self.binding_stack.pop().ok_or_else(|| {
            crate::HandlerError::Invariant(format!(
                "binding end for '{name}' had no matching start marker; regenerate the protocol and ensure the parser and handler use the same WebUI plugin version"
            ))
        })?;
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

    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let decoded = match data.len() {
            4 => {
                let fast = FastElementData::decode(data).map_err(|error| {
                    HandlerError::PluginData(format!(
                        "WebUI hydration plugin expected 4 or 12 bytes of element data: {error}"
                    ))
                })?;
                WebUIElementData {
                    binding_count: fast.binding_count,
                    event_start: 0,
                    event_count: 0,
                }
            }
            12 => WebUIElementData::decode(data).map_err(|error| {
                HandlerError::PluginData(format!(
                    "WebUI hydration plugin expected 4 or 12 bytes of element data: {error}"
                ))
            })?,
            _ => {
                return Err(HandlerError::PluginData(format!(
                    "WebUI hydration plugin expected 4 or 12 bytes, received {}. Regenerate the protocol with a matching parser/handler pair.",
                    data.len()
                )));
            }
        };

        if decoded.binding_count > 0 {
            let binding_index = self.next_index_n(decoded.binding_count);
            self.build_attribute_marker(binding_index, decoded.binding_count);
            writer.write(&self.buffer)?;
        }
        // Emit one data-ev marker per element so browser parsing stays stable.
        if decoded.event_count > 0 {
            self.build_event_marker(decoded.event_count);
            writer.write(&self.buffer)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WebUIHandler;
    use std::collections::HashMap;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};
    use webui_test_utils::test_json;

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

    fn render_component_with_webui_plugin(
        protocol: &WebUIProtocol,
        state: &serde_json::Value,
        entry_id: &str,
    ) -> String {
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        handler
            .handle_as_component(protocol, state, entry_id, &mut writer)
            .unwrap();
        writer.output
    }

    #[test]
    fn test_root_scope_disabled() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("x", &mut writer).unwrap();
        plugin.on_binding_end("x", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_binding_markers_in_child_scope() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        plugin.on_binding_end("userName", &mut writer).unwrap();
        assert_eq!(
            writer.output,
            "<!--w-b:start:0:userName--><!--w-b:end:0:userName-->"
        );
    }

    #[test]
    fn test_repeat_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--w-r:start:0--><!--w-r:end:0-->");
    }

    #[test]
    fn test_attribute_single_binding() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let data = 1u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-w-b-0");
    }

    #[test]
    fn test_attribute_multi_binding() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-w-c-0-3");
    }

    #[test]
    fn test_scope_reset_on_push_pop() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        plugin.push_scope();
        plugin.on_binding_start("b", &mut writer).unwrap();
        plugin.on_binding_end("b", &mut writer).unwrap();
        plugin.pop_scope();
        plugin.on_binding_start("c", &mut writer).unwrap();
        plugin.on_binding_end("c", &mut writer).unwrap();
        // After pop, counter continues from where the outer scope was
        assert!(writer.output.contains("<!--w-b:start:0:a-->"));
        assert!(writer.output.contains("<!--w-b:start:0:b-->"));
        assert!(writer.output.contains("<!--w-b:start:1:c-->"));
    }

    #[test]
    fn test_on_render_complete_only_emits_rendered_components() {
        let mut writer = TestWriter::new();

        // Build a protocol with 3 components — only 1 was rendered
        let mut protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        protocol
            .components
            .entry("comp-a".to_string())
            .or_default()
            .template = "<script>/*comp-a*/</script>".to_string();
        protocol
            .components
            .entry("comp-b".to_string())
            .or_default()
            .template = "<script>/*comp-b*/</script>".to_string();
        protocol
            .components
            .entry("comp-c".to_string())
            .or_default()
            .template = "<script>/*comp-c*/</script>".to_string();

        // Only comp-a was rendered
        let mut rendered = std::collections::HashSet::new();
        rendered.insert("comp-a".to_string());

        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, &mut writer)
            .unwrap();

        // Should contain only comp-a's template
        assert!(
            writer.output.contains("comp-a"),
            "rendered component should be emitted: {}",
            writer.output
        );
        assert!(
            !writer.output.contains("comp-b"),
            "non-rendered component should NOT be emitted: {}",
            writer.output
        );
        assert!(
            !writer.output.contains("comp-c"),
            "non-rendered component should NOT be emitted: {}",
            writer.output
        );
    }

    #[test]
    fn test_route_component_is_noop() {
        let plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        let state = serde_json::json!({
            "title": "Hello",
            "cartOpen": true,
            "items": [{"name": "A&B"}],
        });

        plugin
            .write_route_component_state(&state, &mut writer)
            .unwrap();

        assert_eq!(writer.output, "");
    }

    #[test]
    fn test_hydration_preserves_inline_spaces_around_entity_between_bindings() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<nav>"),
                    WebUIFragment::signal("sectionName", false),
                    WebUIFragment::raw(" &gt; "),
                    WebUIFragment::signal("topicName", false),
                    WebUIFragment::raw("</nav>"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "sectionName": "Backend",
            "topicName": "Rust"
        });

        let html = render_component_with_webui_plugin(&protocol, &state, "index.html");

        assert_eq!(
            html,
            "<nav><!--w-b:start:0:sectionName-->Backend<!--w-b:end:0:sectionName--> &gt; <!--w-b:start:1:topicName-->Rust<!--w-b:end:1:topicName--></nav>"
        );
    }

    #[test]
    fn test_hydration_for_loop_marker_format() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<ul>"),
                    WebUIFragment::for_loop("item", "items", "item-tpl"),
                    WebUIFragment::raw("</ul>"),
                ],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<li>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</li>"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "items": [
                {"name": "Alpha"},
                {"name": "Beta"}
            ]
        });

        let html = render_component_with_webui_plugin(&protocol, &state, "index.html");

        assert!(
            html.contains("<!--w-b:start:0:item-tpl-->"),
            "should have outer binding start for for-loop: {html}"
        );
        assert!(
            html.contains("<!--w-b:end:0:item-tpl-->"),
            "should have outer binding end for for-loop: {html}"
        );
        assert!(
            html.contains("<!--w-r:start:0-->"),
            "should have first repeat marker: {html}"
        );
        assert!(
            html.contains("<!--w-r:end:1-->"),
            "should have second repeat end marker: {html}"
        );
        assert!(
            html.contains("<!--w-b:start:0:item.name-->Alpha<!--w-b:end:0:item.name-->"),
            "should have item binding markers: {html}"
        );
    }

    #[test]
    fn test_data_ev_single_event() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        // Encode: [binding_count=0, event_start=0, event_count=1]
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-ev=\"1\"");
    }

    #[test]
    fn test_data_ev_multiple_events() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes()); // binding_count
        data.extend_from_slice(&5u32.to_le_bytes()); // event_start
        data.extend_from_slice(&3u32.to_le_bytes()); // event_count
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, " data-ev=\"3\"");
    }

    #[test]
    fn test_data_ev_with_bindings() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes()); // binding_count=2
        data.extend_from_slice(&0u32.to_le_bytes()); // event_start=0
        data.extend_from_slice(&1u32.to_le_bytes()); // event_count=1
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert!(
            writer.output.contains("data-w-c-0-2"),
            "binding marker: {}",
            writer.output
        );
        assert!(
            writer.output.contains("data-ev=\"1\""),
            "event marker: {}",
            writer.output
        );
    }

    #[test]
    fn test_data_ev_zero_events_no_marker() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes()); // binding_count=0
        data.extend_from_slice(&0u32.to_le_bytes()); // event_start=0
        data.extend_from_slice(&0u32.to_le_bytes()); // event_count=0
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(
            writer.output, "",
            "no markers for zero bindings and zero events"
        );
    }

    #[test]
    fn test_data_ev_root_scope_suppressed() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        // Don't push a child scope — stay at root
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(
            writer.output, "",
            "root scope should suppress all markers including data-ev"
        );
    }

    #[test]
    fn test_binding_index_advances_with_plugin_data() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        // First element: 2 bindings
        let data1 = 2u32.to_le_bytes();
        plugin.on_element_data(&data1, &mut writer).unwrap();
        // Second element: 1 binding
        let data2 = 1u32.to_le_bytes();
        plugin.on_element_data(&data2, &mut writer).unwrap();
        assert!(
            writer.output.contains("data-w-c-0-2"),
            "first element bindings at 0: {}",
            writer.output
        );
        assert!(
            writer.output.contains("data-w-b-2"),
            "second element binding at 2: {}",
            writer.output
        );
    }

    #[test]
    fn test_on_render_complete_empty_rendered_set() {
        let mut writer = TestWriter::new();
        let mut protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        protocol
            .components
            .entry("comp-a".to_string())
            .or_default()
            .template = "<script>/*a*/</script>".to_string();
        let rendered = std::collections::HashSet::new();
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, &mut writer)
            .unwrap();
        assert_eq!(writer.output, "", "empty rendered set should emit nothing");
    }

    #[test]
    fn test_on_render_complete_skips_empty_template() {
        let mut writer = TestWriter::new();
        let mut protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        protocol
            .components
            .entry("comp-a".to_string())
            .or_default()
            .template = String::new();
        let mut rendered = std::collections::HashSet::new();
        rendered.insert("comp-a".to_string());
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, &mut writer)
            .unwrap();
        assert_eq!(writer.output, "", "empty template should not be emitted");
    }

    #[test]
    fn test_on_render_complete_unknown_component() {
        let mut writer = TestWriter::new();
        let protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        let mut rendered = std::collections::HashSet::new();
        rendered.insert("nonexistent-comp".to_string());
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, &mut writer)
            .unwrap();
        assert_eq!(
            writer.output, "",
            "unknown component should not cause error"
        );
    }

    #[test]
    fn test_invalid_plugin_data_length_returns_error() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let data = [0u8, 0u8]; // only 2 bytes
        let result = plugin.on_element_data(&data, &mut writer);
        assert!(
            matches!(result, Err(crate::HandlerError::PluginData(ref msg)) if msg.contains("expected 4 or 12 bytes")),
            "invalid payload length should produce a plugin-data error: {result:?}"
        );
        assert_eq!(
            writer.output, "",
            "invalid payload must not write partial output"
        );
    }

    #[test]
    fn test_plugin_data_4_bytes_no_events() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let data = 1u32.to_le_bytes(); // only 4 bytes — binding_count=1, no event data
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(
            writer.output, " data-w-b-0",
            "4-byte payload should emit binding marker only"
        );
    }

    #[test]
    fn test_binding_end_without_start_returns_error() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();

        let result = plugin.on_binding_end("missing", &mut writer);

        assert!(
            matches!(result, Err(crate::HandlerError::Invariant(ref msg)) if msg.contains("missing")),
            "binding underflow should produce an invariant error: {result:?}"
        );
        assert_eq!(writer.output, "", "failed binding end must not emit output");
    }

    #[test]
    fn test_invalid_plugin_data_length_rejects_partial_payloads() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();

        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // 8 bytes total is still invalid

        let result = plugin.on_element_data(&data, &mut writer);

        assert!(
            matches!(result, Err(crate::HandlerError::PluginData(ref msg)) if msg.contains("expected 4 or 12 bytes")),
            "partial 12-byte payload should be rejected: {result:?}"
        );
        assert_eq!(
            writer.output, "",
            "invalid payload must not write partial output"
        );
    }
}
