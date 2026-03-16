// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Handler plugin trait and built-in plugin implementations.
//!
//! The plugin system is framework-agnostic — WebUI provides lifecycle hooks
//! and plugins decide what (if anything) to write. Different frameworks
//! (FAST, LIT, etc.) implement `HandlerPlugin` with their own marker formats.

mod fast;

pub use fast::FastHydrationPlugin;

use crate::{ResponseWriter, Result};

/// A handler plugin that can inject additional content during rendering.
///
/// Plugins receive callbacks at key points in the rendering lifecycle:
/// - **Scope management**: `push_scope` / `pop_scope` for component and loop boundaries
/// - **Binding lifecycle**: `on_binding_start` / `on_binding_end` around signals, for-loops, if-conditions
/// - **Repeat items**: `on_repeat_item_start` / `on_repeat_item_end` per for-loop item
/// - **Plugin data**: `on_plugin_data` for opaque data from parser plugins
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

    /// Called when a plugin-specific protocol fragment is encountered.
    /// The data is opaque bytes from the parser plugin — interpretation is plugin-defined.
    fn on_plugin_data(&mut self, data: &[u8], writer: &mut dyn ResponseWriter) -> Result<()>;

    /// Called after all fragments have been rendered.
    fn on_render_complete(
        &mut self,
        _protocol: &webui_protocol::WebUIProtocol,
        _rendered_components: &std::collections::HashSet<String>,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
    }
}
