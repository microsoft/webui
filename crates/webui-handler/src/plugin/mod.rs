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

/// Context passed to plugin-specific SSR bootstrap extension hooks.
pub struct BootstrapExtensionContext<'a> {
    /// Full protocol for plugins that need additional component metadata.
    pub protocol: &'a WebUIProtocol,
    /// Route-reachable component tags for this render.
    pub components: &'a HashSet<String>,
    /// Split WebUI template payloads collected for this render.
    pub payloads: &'a [WebUiTemplatePayload<'a>],
    /// CSP nonce for executable scripts, when configured.
    pub nonce: Option<&'a str>,
}

/// A handler plugin that can inject additional content during rendering.
///
/// Plugins receive callbacks at key points in the rendering lifecycle:
/// - **Scope management**: `push_scope` / `pop_scope` for component and loop boundaries
/// - **Binding lifecycle**: binding hooks around escaped and raw signals
/// - **For-loop lifecycle**: `on_for_start` / `on_for_end` around for-loop blocks
/// - **If-condition lifecycle**: `on_if_start` / `on_if_end` around if-condition blocks
/// - **Repeat items**: `on_repeat_item_start` / `on_repeat_item_end` per for-loop item
/// - **Element data**: `on_element_data` for parser-produced hydration metadata
/// - **Route state**: `write_route_component_state` for framework-specific opening-tag attributes
///
/// WebUI does not interpret what plugins write — it just calls the hooks.
/// Each framework defines its own marker format.
///
/// # Threading
///
/// Plugins must be [`Send`] because the same handler can create an owned
/// [`StreamingSession`](crate::StreamingSession). That session parks its live
/// per-render plugin between calls and may move to another host thread. Rust
/// cannot safely add `Send` after a factory result has been erased to
/// `Box<dyn HandlerPlugin>`, so the guarantee is established here.
///
/// Plugins do not need to be [`Sync`]. Every render receives a fresh instance,
/// and WebUI never invokes one instance concurrently. Sendable interior
/// mutability such as [`Cell`](std::cell::Cell) and
/// [`RefCell`](std::cell::RefCell) remains supported; thread-affine state such
/// as [`Rc`](std::rc::Rc) must be replaced with owned data, [`Arc`](std::sync::Arc),
/// or another sendable handle.
pub trait HandlerPlugin: Send {
    /// Enter a new scope (component or for-loop item boundary).
    /// Typically resets per-scope counters.
    fn push_scope(&mut self);

    /// Exit the current scope, restoring the parent scope state.
    fn pop_scope(&mut self);

    /// Called before rendering a signal binding.
    ///
    /// `raw` is true when the signal owns a replaceable HTML sibling range.
    fn on_binding_start(
        &mut self,
        name: &str,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;

    /// Called after rendering a signal binding.
    ///
    /// `raw` is true when the signal owns a replaceable HTML sibling range.
    fn on_binding_end(
        &mut self,
        name: &str,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;

    /// Called before rendering a for-loop block.
    /// Defaults to [`on_binding_start`](HandlerPlugin::on_binding_start).
    fn on_for_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_start(name, false, writer)
    }

    /// Called after rendering a for-loop block.
    /// Defaults to [`on_binding_end`](HandlerPlugin::on_binding_end).
    fn on_for_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_end(name, false, writer)
    }

    /// Called before rendering an if-condition block.
    /// Defaults to [`on_binding_start`](HandlerPlugin::on_binding_start).
    fn on_if_start(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_start(name, false, writer)
    }

    /// Called after rendering an if-condition block.
    /// Defaults to [`on_binding_end`](HandlerPlugin::on_binding_end).
    fn on_if_end(&mut self, name: &str, writer: &mut dyn ResponseWriter) -> Result<()> {
        self.on_binding_end(name, false, writer)
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

    /// Slice-based counterpart to [`HandlerPlugin::emit_templates`].
    ///
    /// The streaming checkpoint path captures the exact component tags rendered
    /// since the previous checkpoint as a borrowed `&[&str]`, avoiding the owned
    /// `HashSet<String>` the ordinary body-end path builds. The default forwards
    /// to [`emit_component_templates_slice`] (verbatim FAST `<f-template>`
    /// emission).
    fn emit_templates_slice(
        &self,
        protocol: &WebUIProtocol,
        tags: &[&str],
        _nonce: Option<&str>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        emit_component_templates_slice(protocol, tags, writer)
    }

    /// Slice-based counterpart to [`HandlerPlugin::collect_template_payloads`].
    ///
    /// Consumes a borrowed `&[&str]` of component tags so the streaming path can
    /// project per-checkpoint templates without materializing an owned set. The
    /// default returns `None`.
    fn collect_template_payloads_slice<'a>(
        &self,
        _protocol: &'a WebUIProtocol,
        _tags: &[&str],
    ) -> Option<Vec<WebUiTemplatePayload<'a>>> {
        None
    }

    /// Emit plugin-specific executable SSR bootstrap code for the streaming
    /// path, given only the already-collected template payloads.
    ///
    /// Unlike [`HandlerPlugin::emit_bootstrap_extension`], this takes no
    /// `HashSet<String>` component set — the streaming checkpoint has already
    /// projected the exact per-checkpoint payloads. The default is a no-op.
    fn emit_bootstrap_extension_payloads(
        &self,
        _payloads: &[WebUiTemplatePayload<'_>],
        _nonce: Option<&str>,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
    }

    /// Emit plugin-specific executable SSR bootstrap code, if needed.
    ///
    /// The handler emits shared metadata as inert `#webui-data`; client
    /// packages parse that data lazily. Plugins can still emit executable
    /// side-channel data here, such as WebUI framework `templateFns` closures.
    /// The default is a no-op for FAST plugins, which use `<f-template>` tags.
    fn emit_bootstrap_extension(
        &self,
        _context: BootstrapExtensionContext<'_>,
        _writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        Ok(())
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

/// Slice-based counterpart to [`emit_component_templates`].
///
/// Writes each non-empty template verbatim for the borrowed component tags,
/// used by the streaming checkpoint fallback path.
pub(crate) fn emit_component_templates_slice(
    protocol: &WebUIProtocol,
    tags: &[&str],
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    for &name in tags {
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::HandlerPlugin;
    use crate::{ResponseWriter, Result};

    #[derive(Default)]
    struct SendOnlyPlugin {
        scope_depth: Cell<usize>,
        binding_calls: Cell<usize>,
        raw_binding_calls: Cell<usize>,
    }

    struct TestWriter;

    impl ResponseWriter for TestWriter {
        fn write(&mut self, _content: &str) -> Result<()> {
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl HandlerPlugin for SendOnlyPlugin {
        fn push_scope(&mut self) {
            self.scope_depth
                .set(self.scope_depth.get().saturating_add(1));
        }

        fn pop_scope(&mut self) {
            self.scope_depth
                .set(self.scope_depth.get().saturating_sub(1));
        }

        fn on_binding_start(
            &mut self,
            _name: &str,
            raw: bool,
            _writer: &mut dyn ResponseWriter,
        ) -> Result<()> {
            self.binding_calls
                .set(self.binding_calls.get().saturating_add(1));
            if raw {
                self.raw_binding_calls
                    .set(self.raw_binding_calls.get().saturating_add(1));
            }
            Ok(())
        }

        fn on_binding_end(
            &mut self,
            _name: &str,
            raw: bool,
            _writer: &mut dyn ResponseWriter,
        ) -> Result<()> {
            self.binding_calls
                .set(self.binding_calls.get().saturating_add(1));
            if raw {
                self.raw_binding_calls
                    .set(self.raw_binding_calls.get().saturating_add(1));
            }
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

        fn on_element_data(
            &mut self,
            _data: &[u8],
            _writer: &mut dyn ResponseWriter,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handler_plugin_is_send_but_does_not_require_sync() {
        fn assert_send<T: Send + ?Sized>() {}
        fn assert_plugin<T: HandlerPlugin>() {}

        assert_send::<dyn HandlerPlugin>();
        assert_plugin::<SendOnlyPlugin>();
    }

    #[test]
    fn binding_hooks_receive_raw_range_flag() {
        let mut plugin = SendOnlyPlugin::default();
        let mut writer = TestWriter;

        plugin
            .on_binding_start("trusted_html", true, &mut writer)
            .unwrap();
        plugin
            .on_binding_end("trusted_html", true, &mut writer)
            .unwrap();

        assert_eq!(plugin.binding_calls.get(), 2);
        assert_eq!(plugin.raw_binding_calls.get(), 2);
    }
}
