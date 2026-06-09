// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Deprecated FAST 2 hydration plugin for the WebUI handler.
//!
//! Emits the legacy FAST 2 marker format used by the `fast` and `fast-v2`
//! plugin names. FAST 3 hydration is implemented separately in `fast_v3`.
//!
//! ## FAST 2 Comment Format
//!
//! - **Binding start**: `<!--fe-b$$start$$INDEX$$NAME$$fe-b-->`
//! - **Binding end**: `<!--fe-b$$end$$INDEX$$NAME$$fe-b-->`
//! - **Repeat item start**: `<!--fe-repeat$$start$$INDEX$$fe-repeat-->`
//! - **Repeat item end**: `<!--fe-repeat$$end$$INDEX$$fe-repeat-->`
//! - **Single attribute binding**: ` data-fe-b-INDEX`
//! - **Multiple attribute bindings**: ` data-fe-c-INDEX-COUNT`
use super::HandlerPlugin;
use crate::{HandlerError, ResponseWriter, Result};
use serde_json::Value;
use std::fmt::Write;
use webui_protocol::FastElementData;

// FAST 2 comment format constants
const V2_BINDING_START_PREFIX: &str = "<!--fe-b$$start$$";
const V2_BINDING_END_PREFIX: &str = "<!--fe-b$$end$$";
const V2_BINDING_SUFFIX: &str = "$$fe-b-->";
const V2_SEPARATOR: &str = "$$";
const V2_REPEAT_START_PREFIX: &str = "<!--fe-repeat$$start$$";
const V2_REPEAT_END_PREFIX: &str = "<!--fe-repeat$$end$$";
const V2_REPEAT_SUFFIX: &str = "$$fe-repeat-->";
const V2_ATTR_SINGLE_PREFIX: &str = " data-fe-b-";
const V2_ATTR_MULTI_PREFIX: &str = " data-fe-c-";

#[derive(Clone, Copy)]
struct HydrationScope {
    binding_count: usize,
    in_component: bool,
}

impl HydrationScope {
    const fn root() -> Self {
        Self {
            binding_count: 0,
            in_component: false,
        }
    }

    const fn child(in_component: bool) -> Self {
        Self {
            binding_count: 0,
            in_component,
        }
    }
}

/// Deprecated FAST 2 hydration handler plugin.
///
/// Emits the legacy FAST 2 marker format used by the `fast` and `fast-v2`
/// plugin names. New FAST 3 applications should use the separate
/// `fast_v3::FastV3HydrationPlugin` implementation through `fast-v3` instead.
pub struct FastV2HydrationPlugin {
    /// Stack of local binding counters (one per scope).
    /// The bottom of the stack is the root scope (disabled).
    scopes: Vec<HydrationScope>,
    /// Stack of binding indices for matching start/end pairs.
    binding_stack: Vec<usize>,
    /// Reusable buffer for formatting markers without allocation.
    buffer: String,
}

impl FastV2HydrationPlugin {
    /// Create a new deprecated FAST 2 hydration plugin.
    /// The initial root scope is disabled — markers only emit inside components.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Root scope (index 0) is disabled.
            scopes: vec![HydrationScope::root()],
            binding_stack: Vec::with_capacity(8),
            buffer: String::with_capacity(64),
        }
    }

    /// Whether the current scope is inside a custom element component.
    fn is_active(&self) -> bool {
        matches!(self.scopes.last(), Some(scope) if scope.in_component)
    }

    /// Get the next binding index in the current scope, advancing the counter.
    fn next_index(&mut self) -> usize {
        if let Some(scope) = self.scopes.last_mut() {
            let index = scope.binding_count;
            scope.binding_count += 1;
            index
        } else {
            0
        }
    }

    /// Get the next binding index, advancing the counter by `count`.
    fn next_index_n(&mut self, count: u32) -> usize {
        if let Some(scope) = self.scopes.last_mut() {
            let index = scope.binding_count;
            scope.binding_count += count as usize;
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
        self.buffer.push_str(V2_SEPARATOR);
        self.buffer.push_str(name);
        self.buffer.push_str(V2_BINDING_SUFFIX);
    }

    /// Build a repeat comment into the reusable buffer.
    fn build_repeat_comment(&mut self, prefix: &str, index: usize) {
        self.buffer.clear();
        self.buffer.push_str(prefix);
        let _ = write!(self.buffer, "{}", index);
        self.buffer.push_str(V2_REPEAT_SUFFIX);
    }

    /// Build an attribute binding marker into the reusable buffer.
    fn build_attribute_marker(&mut self, binding_index: usize, count: u32) {
        self.buffer.clear();
        if count == 1 {
            self.buffer.push_str(V2_ATTR_SINGLE_PREFIX);
            let _ = write!(self.buffer, "{}", binding_index);
        } else {
            self.buffer.push_str(V2_ATTR_MULTI_PREFIX);
            let _ = write!(self.buffer, "{}-{}", binding_index, count);
        }
    }
}

impl Default for FastV2HydrationPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerPlugin for FastV2HydrationPlugin {
    fn push_scope(&mut self) {
        self.scopes.push(HydrationScope::child(self.is_active()));
    }

    fn push_component_scope(&mut self) {
        self.scopes.push(HydrationScope::child(true));
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let index = self.next_index();
        self.binding_stack.push(index);
        self.build_binding_comment(V2_BINDING_START_PREFIX, index, name);
        writer.write(&self.buffer)
    }

    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let index = self.binding_stack.pop().unwrap_or(0);
        self.build_binding_comment(V2_BINDING_END_PREFIX, index, name);
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
        self.build_repeat_comment(V2_REPEAT_START_PREFIX, index);
        writer.write(&self.buffer)
    }

    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        self.build_repeat_comment(V2_REPEAT_END_PREFIX, index);
        writer.write(&self.buffer)
    }

    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let decoded = FastElementData::decode(data).map_err(|error| {
            HandlerError::PluginData(format!(
                "FAST v2 hydration plugin expected 4 bytes of element data: {error}"
            ))
        })?;
        if decoded.binding_count > 0 {
            let binding_index = self.next_index_n(decoded.binding_count);
            self.build_attribute_marker(binding_index, decoded.binding_count);
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
    use crate::{RenderOptions, WebUIHandler};
    use std::collections::HashMap;
    use webui_protocol::{ConditionExpr, FragmentList, WebUIFragment, WebUIProtocol};
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

    #[test]
    fn test_fast_v2_binding_marker_format() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_component_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("userName", &mut writer).unwrap();
        plugin.on_binding_end("userName", &mut writer).unwrap();
        assert_eq!(
            writer.output,
            "<!--fe-b$$start$$0$$userName$$fe-b--><!--fe-b$$end$$0$$userName$$fe-b-->"
        );
    }

    #[test]
    fn test_fast_v2_binding_sequence_uses_indexes() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_component_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("a", &mut writer).unwrap();
        plugin.on_binding_end("a", &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_start("b", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$1$$b$$fe-b-->");
    }

    #[test]
    fn test_fast_v2_repeat_marker_format() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_component_scope();
        let mut writer = TestWriter::new();
        plugin.on_repeat_item_start(2, &mut writer).unwrap();
        plugin.on_repeat_item_end(2, &mut writer).unwrap();
        assert_eq!(
            writer.output,
            "<!--fe-repeat$$start$$2$$fe-repeat--><!--fe-repeat$$end$$2$$fe-repeat-->"
        );
    }

    #[test]
    fn test_fast_v2_attribute_marker_formats() {
        let mut single = FastV2HydrationPlugin::new();
        single.push_component_scope();
        let mut writer = TestWriter::new();
        let one = 1u32.to_le_bytes();
        single.on_element_data(&one, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-b-0");

        let mut multi = FastV2HydrationPlugin::new();
        multi.push_component_scope();
        writer.output.clear();
        let three = 3u32.to_le_bytes();
        multi.on_element_data(&three, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-c-0-3");
    }

    #[test]
    fn test_fast_v2_attribute_count_advances_binding_index() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_component_scope();
        let mut writer = TestWriter::new();
        let three = 3u32.to_le_bytes();
        plugin.on_element_data(&three, &mut writer).unwrap();

        writer.output.clear();
        plugin.on_binding_start("next", &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$3$$next$$fe-b-->");
    }

    #[test]
    fn test_fast_v2_root_scope_disabled() {
        let mut plugin = FastV2HydrationPlugin::new();
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
    fn test_fast_v2_structural_scope_disabled_outside_component() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
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
    fn test_fast_v2_write_route_component_state_emits_data_state() {
        let plugin = FastV2HydrationPlugin::new();
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
            "FAST v2 handler plugin should emit data-state: {}",
            writer.output
        );
        assert!(
            writer.output.contains(r#"title="Hello""#),
            "FAST v2 handler plugin should still emit scalar attrs: {}",
            writer.output
        );
    }

    fn render_with_fast_v2(protocol: &WebUIProtocol, state: &serde_json::Value) -> String {
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()));
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

    fn assert_no_fast_v2_markers(output: &str) {
        assert!(
            !output.contains("<!--fe-b"),
            "entry light DOM must not contain FAST v2 binding markers: {output}"
        );
        assert!(
            !output.contains("<!--fe-repeat"),
            "entry light DOM must not contain FAST v2 repeat markers: {output}"
        );
        assert!(
            !output.contains("data-fe-"),
            "entry light DOM must not contain FAST v2 attribute markers: {output}"
        );
    }

    #[test]
    fn test_fast_v2_full_render_root_for_skips_hydration_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<main>"),
                    WebUIFragment::for_loop("item", "items", "row"),
                    WebUIFragment::raw("</main>"),
                ],
            },
        );
        fragments.insert(
            "row".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p"),
                    WebUIFragment::attribute("class", "item.name"),
                    WebUIFragment::plugin(1u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "A"}, {"name": "B"}]});
        let output = render_with_fast_v2(&protocol, &state);

        assert_eq!(
            output,
            r#"<main><p class="A">A</p><p class="B">B</p></main>"#
        );
        assert_no_fast_v2_markers(&output);
    }

    #[test]
    fn test_fast_v2_full_render_root_if_skips_hydration_markers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("show"), "visible"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "visible".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span"),
                    WebUIFragment::attribute("title", "title"),
                    WebUIFragment::plugin(1u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"show": true, "title": "Hello"});
        let output = render_with_fast_v2(&protocol, &state);

        assert_eq!(
            output,
            r#"<section><span title="Hello">Hello</span></section>"#
        );
        assert_no_fast_v2_markers(&output);
    }

    #[test]
    fn test_fast_v2_full_render_component_for_keeps_hydration_markers() {
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
                    WebUIFragment::for_loop("item", "items", "row"),
                    WebUIFragment::raw("</ul>"),
                ],
            },
        );
        fragments.insert(
            "row".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<li"),
                    WebUIFragment::attribute("data-name", "item"),
                    WebUIFragment::plugin(1u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("item", false),
                    WebUIFragment::raw("</li>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": ["A", "B"]});
        let output = render_with_fast_v2(&protocol, &state);

        assert!(output.contains("<!--fe-b$$start$$0$$row$$fe-b-->"));
        assert!(output.contains("<!--fe-repeat$$start$$0$$fe-repeat-->"));
        assert!(output.contains(r#"<li data-name="A" data-fe-b-0>"#));
        assert!(output.contains("<!--fe-b$$start$$1$$item$$fe-b-->A"));
        assert!(output.contains("<!--fe-repeat$$end$$1$$fe-repeat-->"));
        assert!(output.contains("<!--fe-b$$end$$0$$row$$fe-b-->"));
    }

    #[test]
    fn test_fast_v2_full_render_component_if_keeps_hydration_markers() {
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
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("show"),
                    "if-body",
                )],
            },
        );
        fragments.insert(
            "if-body".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span"),
                    WebUIFragment::attribute("title", "title"),
                    WebUIFragment::plugin(1u32.to_le_bytes().to_vec()),
                    WebUIFragment::raw(">"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"show": true, "title": "Hello"});
        let output = render_with_fast_v2(&protocol, &state);

        assert!(output.contains("<!--fe-b$$start$$0$$if-body$$fe-b-->"));
        assert!(output.contains(r#"<span title="Hello" data-fe-b-0>"#));
        assert!(output.contains("<!--fe-b$$start$$1$$title$$fe-b-->Hello"));
        assert!(output.contains("<!--fe-b$$end$$0$$if-body$$fe-b-->"));
    }
}
