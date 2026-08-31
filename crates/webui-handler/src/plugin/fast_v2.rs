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

/// Deprecated FAST 2 hydration handler plugin.
///
/// Emits the legacy FAST 2 marker format used by the `fast` and `fast-v2`
/// plugin names. New FAST 3 applications should use the separate
/// `fast_v3::FastV3HydrationPlugin` implementation through `fast-v3` instead.
pub struct FastV2HydrationPlugin {
    /// Stack of local binding counters (one per scope).
    /// The bottom of the stack is the root scope (disabled).
    scopes: Vec<usize>,
    /// Stack of binding indices for matching start/end pairs.
    binding_stack: Vec<usize>,
    /// Reusable buffer for formatting markers without allocation.
    buffer: String,
}

impl FastV2HydrationPlugin {
    /// Create a new deprecated FAST 2 hydration plugin.
    /// The initial root scope is disabled — markers only emit in child scopes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Root scope (index 0) is disabled — only scopes.len() > 1 are active.
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
        self.scopes.push(0);
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn on_binding_start(
        &mut self,
        name: &str,
        _raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }
        let index = self.next_index();
        self.binding_stack.push(index);
        self.build_binding_comment(V2_BINDING_START_PREFIX, index, name);
        writer.write(&self.buffer)
    }

    fn on_binding_end(
        &mut self,
        name: &str,
        _raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
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
        let (decoded, reset_child_scope) = FastElementData::decode_v2(data).map_err(|error| {
            HandlerError::PluginData(format!(
                "FAST v2 hydration plugin received invalid element data: {error}"
            ))
        })?;
        if decoded.binding_count > 0 {
            let binding_index = self.next_index_n(decoded.binding_count);
            self.build_attribute_marker(binding_index, decoded.binding_count);
            writer.write(&self.buffer)?;
        }
        if reset_child_scope {
            if let Some(counter) = self.scopes.last_mut() {
                *counter = 0;
            }
        }
        Ok(())
    }

    /// FAST emits scalar attributes on route component elements.
    /// Components read these via `@attr` and their connection lifecycle.
    fn write_route_component_state(
        &self,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        super::fast::write_route_component_state(state, writer)
    }
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
    fn test_fast_v2_binding_marker_format() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin
            .on_binding_start("userName", false, &mut writer)
            .unwrap();
        plugin
            .on_binding_end("userName", false, &mut writer)
            .unwrap();
        assert_eq!(
            writer.output,
            "<!--fe-b$$start$$0$$userName$$fe-b--><!--fe-b$$end$$0$$userName$$fe-b-->"
        );
    }

    #[test]
    fn test_fast_v2_binding_sequence_uses_indexes() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("a", false, &mut writer).unwrap();
        plugin.on_binding_end("a", false, &mut writer).unwrap();
        writer.output.clear();
        plugin.on_binding_start("b", false, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$1$$b$$fe-b-->");
    }

    #[test]
    fn test_fast_v2_repeat_marker_format() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
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
        single.push_scope();
        let mut writer = TestWriter::new();
        let one = 1u32.to_le_bytes();
        single.on_element_data(&one, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-b-0");

        let mut multi = FastV2HydrationPlugin::new();
        multi.push_scope();
        writer.output.clear();
        let three = 3u32.to_le_bytes();
        multi.on_element_data(&three, &mut writer).unwrap();
        assert_eq!(writer.output, " data-fe-c-0-3");
    }

    #[test]
    fn test_fast_v2_attribute_count_advances_binding_index() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let three = 3u32.to_le_bytes();
        plugin.on_element_data(&three, &mut writer).unwrap();

        writer.output.clear();
        plugin.on_binding_start("next", false, &mut writer).unwrap();
        assert_eq!(writer.output, "<!--fe-b$$start$$3$$next$$fe-b-->");
    }

    #[test]
    fn test_fast_v2_root_host_bindings_reset_child_index() {
        let mut plugin = FastV2HydrationPlugin::new();
        plugin.push_scope();
        let mut writer = TestWriter::new();
        let root = FastElementData { binding_count: 2 }.encode_v2(true);
        plugin.on_element_data(&root, &mut writer).unwrap();
        plugin
            .on_element_data(&1u32.to_le_bytes(), &mut writer)
            .unwrap();
        assert_eq!(writer.output, " data-fe-c-0-2 data-fe-b-0");
    }

    #[test]
    fn test_fast_v2_parser_and_handler_keep_child_index_local() {
        use crate::{RenderOptions, WebUIHandler};
        use webui_parser::{
            plugin::fast_v2::FastV2ParserPlugin, ComponentRegistration, HtmlParser,
        };
        use webui_protocol::WebUIProtocol;

        let mut parser = HtmlParser::with_plugin(Box::new(FastV2ParserPlugin::new()));
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "binding-card",
                r#"<f-template name="binding-card" shadowrootmode="open"><template @click="{click($e)}" @keydown="{keydown($e)}"><slot f-ref="{slot}"></slot></template></f-template>"#,
                None,
                true,
            ))
            .unwrap();
        parser
            .parse("index.html", "<binding-card></binding-card>")
            .unwrap();
        let mut protocol = WebUIProtocol::new(parser.into_fragment_records());
        protocol
            .components
            .entry("binding-card".to_string())
            .or_default()
            .uses_shadow_dom = true;
        protocol.populate_style_closures(&["index.html"]);
        let handler = WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()));
        let mut writer = TestWriter::new();
        handler
            .handle(
                &protocol,
                &serde_json::json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        assert!(
            writer.output.contains(concat!(
                r#"<template shadowrootmode="open" data-fe-c-0-2>"#,
                r#"<slot data-fe-b-0></slot></template>"#
            )),
            "FAST 2 child markers should exclude consumed host bindings: {}",
            writer.output
        );
    }

    #[test]
    fn test_fast_v2_root_scope_disabled() {
        let mut plugin = FastV2HydrationPlugin::new();
        let mut writer = TestWriter::new();
        plugin.on_binding_start("x", false, &mut writer).unwrap();
        plugin.on_binding_end("x", false, &mut writer).unwrap();
        plugin.on_repeat_item_start(0, &mut writer).unwrap();
        plugin.on_repeat_item_end(0, &mut writer).unwrap();
        let data = 3u32.to_le_bytes();
        plugin.on_element_data(&data, &mut writer).unwrap();
        assert_eq!(writer.output, "");
    }
}
