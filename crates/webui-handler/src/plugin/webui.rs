// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI handler plugin that emits lightweight hydration markers.
//!
//! Emits comment markers around for-loop blocks (`<!--wr-->` / `<!--/wr-->`),
//! before each repeat item (`<!--wi-->`), and around if-condition blocks
//! (`<!--wc-->` / `<!--/wc-->`). These markers enable zero-DOM-mutation
//! in-place hydration on the client: the framework reuses the SSR comment
//! nodes as runtime anchors instead of creating temporary wrappers.

use super::HandlerPlugin;
use crate::{ResponseWriter, Result};

const REPEAT_START: &str = "<!--wr-->";
const REPEAT_END: &str = "<!--/wr-->";
const REPEAT_ITEM: &str = "<!--wi-->";
const COND_START: &str = "<!--wc-->";
const COND_END: &str = "<!--/wc-->";

/// WebUI handler plugin that emits hydration markers.
///
/// Emits lightweight HTML comment markers around structural boundaries
/// (for-loops and if-conditions) so the client can hydrate in-place
/// without reparenting DOM nodes.
pub struct WebUIHydrationPlugin;

impl WebUIHydrationPlugin {
    /// Create a new WebUI handler plugin.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebUIHydrationPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerPlugin for WebUIHydrationPlugin {
    fn push_scope(&mut self) {}

    fn pop_scope(&mut self) {}

    fn on_binding_start(&mut self, _name: &str, _writer: &mut dyn ResponseWriter) -> Result<()> {
        Ok(())
    }

    fn on_binding_end(&mut self, _name: &str, _writer: &mut dyn ResponseWriter) -> Result<()> {
        Ok(())
    }

    fn on_for_start(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        writer.write(REPEAT_START)
    }

    fn on_for_end(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        writer.write(REPEAT_END)
    }

    fn on_if_start(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        writer.write(COND_START)
    }

    fn on_if_end(&mut self, _name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        writer.write(COND_END)
    }

    fn on_repeat_item_start(
        &mut self,
        _index: usize,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        writer.write(REPEAT_ITEM)
    }

    // No end marker needed — the next <!--wi--> or <!--/wr--> serves as the boundary.
    fn on_repeat_item_end(
        &mut self,
        _index: usize,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
    }

    fn on_element_data(&mut self, _data: &[u8], _writer: &mut dyn ResponseWriter) -> Result<()> {
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
    fn test_default_creates_instance() {
        let _plugin = WebUIHydrationPlugin::default();
    }

    #[test]
    fn test_signal_binding_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("x", &mut writer).unwrap();
        plugin.on_binding_end("x", &mut writer).unwrap();
        assert_eq!(
            writer.output, "",
            "signal binding hooks must not emit output"
        );
    }

    #[test]
    fn test_for_loop_emits_repeat_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_for_start("items", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<div>A</div>").unwrap();
        plugin.pop_scope();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        plugin.on_repeat_item_start(1, &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<div>B</div>").unwrap();
        plugin.pop_scope();
        plugin.on_repeat_item_end(1, &mut writer).unwrap();
        plugin.on_for_end("items", &mut writer).unwrap();
        assert_eq!(
            writer.output,
            "<!--wr--><!--wi--><div>A</div><!--wi--><div>B</div><!--/wr-->"
        );
    }

    #[test]
    fn test_empty_for_loop_emits_boundary_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_for_start("items", &mut writer).unwrap();
        plugin.on_for_end("items", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--wr--><!--/wr-->");
    }

    #[test]
    fn test_if_true_emits_cond_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_if_start("show", &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<p>visible</p>").unwrap();
        plugin.pop_scope();
        plugin.on_if_end("show", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--wc--><p>visible</p><!--/wc-->");
    }

    #[test]
    fn test_if_false_emits_empty_cond_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_if_start("show", &mut writer).unwrap();
        // condition is false — no content rendered
        plugin.on_if_end("show", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--wc--><!--/wc-->");
    }

    #[test]
    fn test_nested_repeat_inside_conditional() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_if_start("show", &mut writer).unwrap();
        plugin.push_scope();
        plugin.on_for_start("items", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<li>X</li>").unwrap();
        plugin.pop_scope();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        plugin.on_for_end("items", &mut writer).unwrap();
        plugin.pop_scope();
        plugin.on_if_end("show", &mut writer).unwrap();
        assert_eq!(
            writer.output,
            "<!--wc--><!--wr--><!--wi--><li>X</li><!--/wr--><!--/wc-->"
        );
    }

    #[test]
    fn test_element_data_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "", "element data must not emit output");
    }

    #[test]
    fn test_scope_push_pop_is_noop() {
        let mut plugin = WebUIHydrationPlugin::new();
        plugin.push_scope();
        plugin.push_scope();
        plugin.pop_scope();
        plugin.pop_scope();
    }

    #[test]
    fn test_route_component_state_emits_no_output() {
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
        assert_eq!(
            writer.output, "",
            "route component state must not emit output"
        );
    }

    #[test]
    fn test_full_lifecycle_with_markers() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();

        // Simulate a component with a signal, repeat, and conditional.
        plugin.push_scope();
        // Signal binding — no markers
        plugin.on_binding_start("a", &mut writer).unwrap();
        writer.write("hello").unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        // For-loop — markers
        plugin.on_for_start("list", &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<x-item>1</x-item>").unwrap();
        plugin.pop_scope();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        plugin.on_for_end("list", &mut writer).unwrap();
        // Conditional — markers
        plugin.on_if_start("flag", &mut writer).unwrap();
        plugin.push_scope();
        writer.write("<p>yes</p>").unwrap();
        plugin.pop_scope();
        plugin.on_if_end("flag", &mut writer).unwrap();
        plugin.pop_scope();

        assert_eq!(
            writer.output,
            "hello<!--wr--><!--wi--><x-item>1</x-item><!--/wr--><!--wc--><p>yes</p><!--/wc-->"
        );
    }

    #[test]
    fn test_on_render_complete_only_emits_rendered_components() {
        let mut writer = TestWriter::new();

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

        let mut rendered = std::collections::HashSet::new();
        rendered.insert("comp-a".to_string());

        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, None, &mut writer)
            .unwrap();

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
    fn test_on_render_complete_empty_rendered_set() {
        let mut writer = TestWriter::new();
        let mut protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        protocol
            .components
            .entry("comp-a".to_string())
            .or_default()
            .template = "<script>/*a*/</script>".to_string();
        let rendered = std::collections::HashSet::new();
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, None, &mut writer)
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
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, None, &mut writer)
            .unwrap();
        assert_eq!(writer.output, "", "empty template should not be emitted");
    }

    #[test]
    fn test_on_render_complete_unknown_component() {
        let mut writer = TestWriter::new();
        let protocol = webui_protocol::WebUIProtocol::new(std::collections::HashMap::new());
        let mut rendered = std::collections::HashSet::new();
        rendered.insert("nonexistent-comp".to_string());
        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, None, &mut writer)
            .unwrap();
        assert_eq!(
            writer.output, "",
            "unknown component should not cause error"
        );
    }

    // ── Integration test (full render cycle with WebUIHandler) ──────────

    use std::collections::HashMap;
    use webui_protocol::{ConditionExpr, FragmentList, WebUIFragment, WebUIProtocol};
    use webui_test_utils::test_json;

    use crate::{RenderOptions, WebUIHandler};

    fn render_with_webui_plugin(protocol: &WebUIProtocol, state: &serde_json::Value) -> String {
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
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
    fn test_handler_emits_hydration_markers_for_loop_and_if() {
        // Build a protocol with a for-loop (2 items) and an if-condition.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::for_loop("item", "items", "for-body"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("show"), "if-body"),
                ],
            },
        );
        fragments.insert(
            "for-body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item", false)],
            },
        );
        fragments.insert(
            "if-body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>yes</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": ["a", "b"], "show": true});
        let output = render_with_webui_plugin(&protocol, &state);

        // For-loop markers
        assert!(
            output.contains("<!--wr-->"),
            "Expected repeat-start marker, got: {output}"
        );
        assert!(
            output.contains("<!--/wr-->"),
            "Expected repeat-end marker, got: {output}"
        );
        assert!(
            output.contains("<!--wi-->"),
            "Expected repeat-item marker, got: {output}"
        );
        // Each item should produce a <!--wi--> marker
        assert_eq!(
            output.matches("<!--wi-->").count(),
            2,
            "Expected 2 repeat-item markers, got: {output}"
        );

        // If-condition markers
        assert!(
            output.contains("<!--wc-->"),
            "Expected cond-start marker, got: {output}"
        );
        assert!(
            output.contains("<!--/wc-->"),
            "Expected cond-end marker, got: {output}"
        );

        // Content is rendered
        assert!(
            output.contains("a"),
            "Expected for-loop items in output, got: {output}"
        );
        assert!(
            output.contains("b"),
            "Expected for-loop items in output, got: {output}"
        );
        assert!(
            output.contains("<span>yes</span>"),
            "Expected if-condition body, got: {output}"
        );
    }
}
