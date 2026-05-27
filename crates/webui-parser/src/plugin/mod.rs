// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser plugin trait and built-in plugin implementations.
//!
//! The plugin system is framework-aware but phase-local — parser plugins
//! classify framework-owned attributes, capture processed component templates,
//! and emit per-element hydration metadata for the handler.

pub mod fast;
pub mod fast_v2;
pub mod fast_v3;
pub mod webui;

use crate::component_registry::Component;
use crate::{ParserOptions, Result};

/// Parser-owned decision about how an attribute should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeAction {
    /// Let the parser process the attribute normally.
    Keep,
    /// Skip the attribute without incrementing the element binding count.
    Skip,
    /// Skip the attribute and count it as a dynamic binding.
    SkipAndCountBinding,
}

/// Build artifacts extracted from a parser plugin after parsing completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserPluginArtifacts {
    /// The plugin did not produce any post-parse artifacts.
    None,
    /// Client component templates keyed by tag name.
    ComponentTemplates(Vec<(String, String)>),
}

/// A parser plugin that can customize template parsing behavior.
///
/// Plugins receive callbacks at key points during HTML parsing:
/// - **Fragment lifecycle**: `start_fragment` before a fragment parse begins
/// - **Component registration**: `register_component_template` when a component template is finalized
/// - **Attribute classification**: `classify_attribute` for framework-specific attrs
/// - **Element completion**: `finish_element` after attributes are processed
///
/// WebUI calls these hooks during parsing; plugins decide what (if anything) to do.
pub trait ParserPlugin {
    /// Called before parsing begins for a fragment.
    ///
    /// Plugins can use this to reset fragment-local counters while preserving
    /// global build-level state such as tracked component templates.
    fn start_fragment(&mut self, _fragment_id: &str) {}

    /// Called when parser output options change.
    fn configure(&mut self, _options: &ParserOptions) {}

    /// Called when a component template has been fully processed for the active
    /// CSS strategy and wrapped for shadow DOM rendering.
    fn register_component_template(
        &mut self,
        tag_name: &str,
        component: &Component,
        processed_template: &str,
    ) -> Result<()>;

    /// Decide how a framework-owned attribute should be handled.
    fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction;

    /// Called after all attributes on an element have been processed.
    /// `binding_attribute_count` is the number of dynamic attribute bindings found.
    /// Returns optional opaque bytes to emit as a `Plugin` protocol fragment.
    fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>>;

    /// Consume the plugin and return any build artifacts it captured.
    fn into_artifacts(self: Box<Self>) -> ParserPluginArtifacts {
        ParserPluginArtifacts::None
    }
}
