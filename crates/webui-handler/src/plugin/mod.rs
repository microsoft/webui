// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Handler plugin trait and built-in plugin implementations.
//!
//! Handler plugins write framework-specific hydration markers while shared
//! completion work, such as component template emission, stays in handler core.

pub mod fast;
pub mod webui;

use crate::{ResponseWriter, Result};
use std::collections::HashSet;
use webui_protocol::WebUIProtocol;

/// A handler plugin that can inject additional content during rendering.
///
/// Plugins receive callbacks at key points in the rendering lifecycle:
/// - **Scope management**: `push_scope` / `pop_scope` for component and loop boundaries
/// - **Binding lifecycle**: `on_binding_start` / `on_binding_end` around signals, for-loops, if-conditions
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

    /// Called before rendering a binding (signal, for-loop, or if-condition).
    fn on_binding_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called after rendering a binding.
    fn on_binding_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()>;

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
}

/// Emit client component templates for only the components rendered in this response.
pub(crate) fn emit_rendered_component_templates(
    protocol: &WebUIProtocol,
    rendered_components: &HashSet<String>,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    for name in rendered_components {
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
