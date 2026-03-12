//! Parser plugin trait and built-in plugin implementations.
//!
//! The plugin system is framework-agnostic — WebUI provides parsing hooks
//! and plugins decide what to do. Different frameworks (FAST, LIT, etc.)
//! implement `ParserPlugin` with their own component discovery and
//! hydration data emission logic.

pub mod fast;

pub use fast::generate_f_template;
pub use fast::FastParserPlugin;

use crate::component_registry::Component;
use crate::Result;
use std::any::Any;

/// A parser plugin that can customize template parsing behavior.
///
/// Plugins receive callbacks at key points during HTML parsing:
/// - **Component discovery**: `on_parse_component` when a custom element is found
/// - **Attribute filtering**: `should_skip_attribute` to skip framework-specific attrs
/// - **Body end injection**: `on_body_end` to inject framework content before `</body>`
/// - **Element completion**: `on_element_parsed` after attributes are processed
///
/// WebUI calls these hooks during parsing; plugins decide what (if anything) to do.
pub trait ParserPlugin: Any {
    /// Called when a component element is encountered during parsing.
    /// Use to track components for later template generation.
    fn on_parse_component(&mut self, tag_name: &str, component: &Component) -> Result<()>;

    /// Return `true` if this attribute should be skipped during parsing.
    /// Framework-specific client-side bindings (e.g., `@click`, `f-ref`)
    /// should be skipped since they are only meaningful at runtime.
    fn should_skip_attribute(&self, attr_name: &str) -> bool;

    /// Called when the `</body>` closing tag is encountered.
    /// Returns optional raw HTML content to inject before `</body>`.
    /// FAST uses this for `<f-template>` wrappers.
    fn on_body_end(&mut self) -> Option<String>;

    /// Called after all attributes on an element have been processed.
    /// `binding_attribute_count` is the number of dynamic attribute bindings found.
    /// Returns optional opaque bytes to emit as a `Plugin` protocol fragment.
    fn on_element_parsed(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;

    /// Downcast to `Any` for plugin-specific access.
    fn as_any(&self) -> &dyn Any;

    /// Downcast to mutable `Any` for plugin-specific access.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
