// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Lightweight no-op WebUI handler plugin.
//!
//! Hydration now uses compiled template paths instead of comment markers or
//! data attributes. This plugin satisfies the [`HandlerPlugin`] trait with
//! no-op implementations — no markers are emitted, no scope tracking is
//! performed, and element data is silently ignored.

use super::HandlerPlugin;
use crate::{ResponseWriter, Result};

/// No-op WebUI handler plugin.
///
/// All trait methods return immediately without writing any output.
/// Retained for API compatibility with callers that construct a plugin
/// instance via [`WebUIHydrationPlugin::new`].
pub struct WebUIHydrationPlugin;

impl WebUIHydrationPlugin {
    /// Create a new (no-op) WebUI handler plugin.
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

    fn on_repeat_item_start(
        &mut self,
        _index: usize,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
    }

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
    fn test_binding_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("x", &mut writer).unwrap();
        plugin.on_binding_end("x", &mut writer).unwrap();
        assert_eq!(writer.output, "", "binding hooks must not emit output");
    }

    #[test]
    fn test_binding_in_child_scope_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        plugin.on_binding_end("userName", &mut writer).unwrap();
        plugin.pop_scope();
        assert_eq!(
            writer.output, "",
            "binding hooks must not emit output even in child scopes"
        );
    }

    #[test]
    fn test_repeat_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        assert_eq!(writer.output, "", "repeat hooks must not emit output");
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
    fn test_element_data_arbitrary_bytes_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.push_scope();
        // Any byte sequence is accepted — the no-op ignores the payload.
        plugin.on_element_data(&[0u8; 12], &mut writer).unwrap();
        plugin.on_element_data(&[0u8; 2], &mut writer).unwrap();
        plugin.on_element_data(&[], &mut writer).unwrap();
        assert_eq!(
            writer.output, "",
            "element data must not emit output regardless of payload"
        );
    }

    #[test]
    fn test_scope_push_pop_is_noop() {
        let mut plugin = WebUIHydrationPlugin::new();
        // Should not panic or affect subsequent calls.
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
    fn test_full_lifecycle_emits_no_output() {
        let mut plugin = WebUIHydrationPlugin::new();
        let mut writer = TestWriter::new();

        // Simulate a complete component render lifecycle.
        plugin.push_scope();
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        plugin.push_scope();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_binding_start("b", &mut writer).unwrap();
        plugin
            .on_element_data(&1u32.to_le_bytes(), &mut writer)
            .unwrap();
        plugin.on_binding_end("b", &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        plugin.pop_scope();
        plugin.pop_scope();

        assert_eq!(writer.output, "", "full lifecycle must produce zero output");
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

        crate::plugin::emit_rendered_component_templates(&protocol, &rendered, None, &mut writer)
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
}
