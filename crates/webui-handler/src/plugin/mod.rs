// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Handler plugin trait and built-in plugin implementations.
//!
//! Handler plugins write framework-specific hydration markers while shared
//! completion work, such as component template emission, stays in handler core.

pub mod fast;
pub mod fast_v2;
pub mod fast_v3;
pub mod webui;

use crate::{ResponseWriter, Result};
use std::collections::HashSet;
use webui_protocol::WebUIProtocol;

/// Split WebUI component template payload used by SSR bootstrap emission.
pub struct WebUiTemplatePayload<'a> {
    /// Component custom-element tag name.
    pub tag_name: &'a str,
    /// JSON-safe template metadata object.
    pub template_json: &'a str,
    /// Component-local JavaScript condition closure array.
    pub template_functions: &'a str,
}

/// A handler plugin that can inject additional content during rendering.
///
/// Plugins receive callbacks at key points in the rendering lifecycle:
/// - **Scope management**: `push_scope` / `pop_scope` for component and loop boundaries
/// - **Binding lifecycle**: `on_binding_start` / `on_binding_end` around signals
/// - **For-loop lifecycle**: `on_for_start` / `on_for_end` around for-loop blocks
/// - **If-condition lifecycle**: `on_if_start` / `on_if_end` around if-condition blocks
/// - **Repeat items**: `on_repeat_item_start` / `on_repeat_item_end` per for-loop item
/// - **Element data**: `on_element_data` for parser-produced hydration metadata
/// - **Route state**: `write_route_component_state` for framework-specific opening-tag attributes
///
/// WebUI does not interpret what plugins write — it just calls the hooks.
/// Each framework defines its own marker format.
pub trait HandlerPlugin {
    /// Enter a new scope (component or for-loop item boundary).
    /// Typically resets per-scope counters.
    fn push_scope(&mut self);

    /// Exit the current scope, restoring the parent scope state.
    fn pop_scope(&mut self);

    /// Called before rendering a signal binding.
    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called after rendering a signal binding.
    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called before rendering a for-loop block.
    /// Defaults to [`on_binding_start`](HandlerPlugin::on_binding_start).
    fn on_for_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_start(name, writer)
    }

    /// Called after rendering a for-loop block.
    /// Defaults to [`on_binding_end`](HandlerPlugin::on_binding_end).
    fn on_for_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_end(name, writer)
    }

    /// Called before rendering an if-condition block.
    /// Defaults to [`on_binding_start`](HandlerPlugin::on_binding_start).
    fn on_if_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_start(name, writer)
    }

    /// Called after rendering an if-condition block.
    /// Defaults to [`on_binding_end`](HandlerPlugin::on_binding_end).
    fn on_if_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_end(name, writer)
    }

    /// Called before rendering a repeat item in a for loop.
    fn on_repeat_item_start(&mut self, index: usize, writer: &mut dyn ResponseWriter)
        -> Result<()>;

    /// Called after rendering a repeat item.
    fn on_repeat_item_end(&mut self, index: usize, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called when parser-produced element metadata is encountered.
    fn on_element_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called when emitting a matched route component's opening tag.
    /// Plugins can write framework-specific attributes before the closing `>`.
    /// The default is a no-op.
    fn write_route_component_state(
        &self,
        _state: &serde_json::Value,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
    }

    /// Emit component templates collected during SSR.  The default emits
    /// each template as-is (suitable for FAST `<f-template>` tags).  The
    /// WebUI split-payload path uses [`HandlerPlugin::collect_template_payloads`]
    /// instead.
    fn emit_templates(
        &self,
        protocol: &WebUIProtocol,
        components: &HashSet<String>,
        _nonce: Option<&str>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        emit_component_templates(protocol, components, writer)
    }

    /// Return split WebUI template payloads for the given components.
    ///
    /// The WebUI plugin overrides this so `lib.rs` can emit JSON metadata in an
    /// inert data block and only emit condition closures as executable JS.
    /// Returns `None` when templates are non-WebUI payloads (e.g. FAST
    /// `<f-template>` tags).
    fn collect_template_payloads<'a>(
        &self,
        _protocol: &'a WebUIProtocol,
        _components: &HashSet<String>,
    ) -> Option<Vec<WebUiTemplatePayload<'a>>> {
        None
    }
}

/// Default template emission: write each non-empty template verbatim.
/// Used by FAST parser plugins for `<f-template>` tags.
pub(crate) fn emit_component_templates(
    protocol: &WebUIProtocol,
    components: &HashSet<String>,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    for name in components {
        if let Some(template) = protocol
            .components
            .get(name)
            .map(|component| component.template.as_str())
            .filter(|template| !template.is_empty())
        {
            writer.write(template)?;
        }
    }
    Ok(())
}
