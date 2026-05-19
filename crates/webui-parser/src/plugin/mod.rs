// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Parser plugin trait and built-in plugin implementations.
//!
//! The plugin system is framework-aware but phase-local — parser plugins
//! classify framework-owned attributes, capture processed component templates,
//! and emit per-element hydration metadata for the handler.

pub mod fast;
pub(crate) mod fast_host_attrs;
pub mod fast_v2;
pub mod fast_v3;
pub mod webui;

use crate::component_registry::Component;
use crate::Result;

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

/// A single attribute declared on the root `<template>` element of a
/// component's source HTML, in the order encountered.
///
/// Surfaces structural metadata to parser plugins without committing the
/// WebUI core to any framework-specific filtering or normalization. The
/// `raw_text` field is the verbatim source spelling of the attribute
/// **without** any leading whitespace (e.g. `autofocus` or
/// `tabindex="0"`); the parser is responsible for inserting separator
/// whitespace when splicing into output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRootAttribute {
    /// Attribute name as written in source (case preserved).
    pub name: String,
    /// Attribute value with quotes stripped, or `None` for a valueless
    /// boolean attribute such as `autofocus`.
    pub value: Option<String>,
    /// Verbatim source spelling of the attribute without leading
    /// whitespace, suitable for splicing back into an HTML opening tag.
    pub raw_text: String,
}

/// A parser plugin that can customize template parsing behavior.
///
/// Plugins receive callbacks at key points during HTML parsing:
/// - **Fragment lifecycle**: `start_fragment` before a fragment parse begins
/// - **Component registration**: `register_component_template` when a component template is finalized
/// - **Template root inspection**: `on_template_root_attributes` once per unique component, before any usage-site emission
/// - **Attribute classification**: `classify_attribute` for framework-specific attrs
/// - **Element completion**: `finish_element` after attributes are processed
/// - **Host-tag mutation**: `host_element_attributes` to inject extra attributes onto a component usage-site host opening tag
/// - **Template-tag mutation**: `template_element_attributes` to inject extra attributes onto the inner Shadow DOM `<template>` wrapper
///
/// WebUI calls these hooks during parsing; plugins decide what (if anything) to do.
pub trait ParserPlugin {
    /// Called before parsing begins for a fragment.
    ///
    /// Plugins can use this to reset fragment-local counters while preserving
    /// global build-level state such as tracked component templates.
    fn start_fragment(&mut self, _fragment_id: &str) {}

    /// Called once per unique component with the structured attributes
    /// declared on the root `<template>` element of its source HTML.
    /// Receives an empty slice when the component HTML does not begin
    /// with a `<template>` element.
    ///
    /// Plugins may stash framework-specific filtered subsets of these
    /// attributes for later injection via
    /// [`host_element_attributes`](Self::host_element_attributes) and
    /// [`template_element_attributes`](Self::template_element_attributes).
    ///
    /// Called **before** [`register_component_template`](Self::register_component_template)
    /// and before the first usage-site host opening tag is emitted for
    /// the component.
    fn on_template_root_attributes(
        &mut self,
        _tag_name: &str,
        _attributes: &[TemplateRootAttribute],
    ) {
    }

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

    /// Append additional attributes onto a component usage-site host
    /// opening tag. Called after the parser has processed all author-
    /// supplied attributes for the host element and before the closing
    /// `>` is emitted.
    ///
    /// Returns the verbatim attribute-list fragment to splice into the
    /// opening tag, **without** a leading separator space — the parser
    /// inserts one space between the author attributes and the appended
    /// text. Returning `None` (or an empty string) appends nothing.
    ///
    /// `author_attr_names` is the source-preserved list of attribute
    /// names the author wrote on the host. The plugin owns any conflict
    /// resolution policy.
    ///
    /// The returned text is inserted verbatim. The plugin is responsible
    /// for HTML attribute escaping and producing a well-formed fragment
    /// (no closing `>`, no tag text).
    ///
    /// Default returns `None`.
    fn host_element_attributes(
        &mut self,
        _tag_name: &str,
        _author_attr_names: &[&str],
    ) -> Option<String> {
        None
    }

    /// Append additional attributes onto the inner Shadow DOM
    /// `<template>` wrapper emitted for a component's content. Called
    /// when re-wrapping a component template under the Shadow DOM
    /// strategy.
    ///
    /// Same contract as [`host_element_attributes`](Self::host_element_attributes):
    /// return verbatim text without a leading separator space; `None`
    /// appends nothing. Only invoked when the parser is configured for
    /// Shadow DOM output.
    ///
    /// Default returns `None`.
    fn template_element_attributes(&mut self, _tag_name: &str) -> Option<String> {
        None
    }

    /// Consume the plugin and return any build artifacts it captured.
    fn into_artifacts(self: Box<Self>) -> ParserPluginArtifacts {
        ParserPluginArtifacts::None
    }
}
