// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

pub mod plugin;
pub mod route_handler;
pub mod route_matcher;
pub(crate) mod route_renderer;

use plugin::HandlerPlugin;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use webui_expressions::{evaluate_with_resolver, ExpressionError};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment, WebUIProtocol};
use webui_state::find_value_by_dotted_path;

/// Error types for the WebUI handler.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Rendering error: {0}")]
    Rendering(String),

    #[error("Rendering invariant error: {0}")]
    Invariant(String),

    #[error("Missing fragment: {0}")]
    MissingFragment(String),

    #[error("Missing data field: {0}")]
    MissingData(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Protocol error: {0}")]
    Protocol(#[from] webui_protocol::ProtocolError),

    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Writer error: {0}")]
    Writer(String),

    #[error("Plugin data error: {0}")]
    PluginData(String),
}

pub type Result<T> = std::result::Result<T, HandlerError>;

/// Interface for writing rendered output
pub trait ResponseWriter {
    /// Write content to the output
    fn write(&mut self, content: &str) -> Result<()>;

    /// Finalize the output
    fn end(&mut self) -> Result<()>;
}

/// Options controlling how the handler renders a protocol.
///
/// The handler performs server-side route matching: matched routes are rendered
/// visible with content; non-matched routes are rendered hidden and empty.
pub struct RenderOptions<'a> {
    /// The fragment ID to start rendering from (e.g., `"index.html"`).
    pub entry_id: &'a str,
    /// The URL path to match routes against (e.g., `"/contacts/42"`).
    pub request_path: &'a str,
    /// Optional CSP nonce for inline `<script>` tags.
    /// When set, all inline scripts include `nonce="VALUE"` and a
    /// `<meta name="webui-nonce">` tag is emitted for the client router.
    pub nonce: Option<&'a str>,
}

impl<'a> RenderOptions<'a> {
    /// Create render options for the given entry fragment and request path.
    #[must_use]
    pub fn new(entry_id: &'a str, request_path: &'a str) -> Self {
        Self {
            entry_id,
            request_path,
            nonce: None,
        }
    }

    /// Set the CSP nonce for inline scripts.
    #[must_use]
    pub fn with_nonce(mut self, nonce: &'a str) -> Self {
        self.nonce = Some(nonce);
        self
    }
}

/// The main WebUI handler that processes protocols and renders them.
///
/// The handler is stateless: plugin instances are created per-render from
/// the stored factory function, allowing concurrent renders with `&self`.
pub struct WebUIHandler {
    plugin_factory: Option<fn() -> Box<dyn HandlerPlugin>>,
}

/// Context object for processing WebUI fragments
struct WebUIProcessContext<'a> {
    protocol: &'a WebUIProtocol,
    state: &'a Value,
    #[allow(dead_code)]
    depth: usize,
    writer: &'a mut dyn ResponseWriter,
    local_vars: HashMap<String, Value>,
    /// Accumulates component attribute values between attrStart and the component fragment.
    component_attrs: HashMap<String, Value>,
    /// URL path for server-side route matching.
    request_path: String,
    /// Base path for resolving relative route paths (`./`).
    /// Updated as the handler descends into nested matched routes.
    route_base: String,
    /// Component names visited during rendering (for selective f-template emission
    /// and CSS module dedup — only the first render of each component emits
    /// its `<style type="module">` tag).
    rendered_components: HashSet<String>,
    /// Per-render plugin instance created from the handler's factory.
    plugin: Option<Box<dyn HandlerPlugin>>,
    /// Current position in the route tree for outlet-based rendering.
    /// Contains the children of the currently matched route fragment.
    route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    /// Entry fragment ID — used to compute the initial inventory at head_end.
    entry_id: String,
    /// CSP nonce for inline `<script>` tags (None = no nonce attribute).
    nonce: Option<String>,
}

/// Get the component attribute name, stripping `:` prefix and converting to camelCase.
///
/// Uses `webui_protocol::attrs::attribute_to_camel` which handles irregular
/// attributes (multi-word ARIA and global HTML attributes like `readonly`,
/// `tabindex`) via the shared lookup table.
fn component_attr_name(name: &str) -> String {
    let stripped = name.strip_prefix(':').unwrap_or(name);
    webui_protocol::attrs::attribute_to_camel(stripped)
}

impl WebUIHandler {
    /// Create a new WebUI handler with no plugin.
    pub fn new() -> Self {
        Self {
            plugin_factory: None,
        }
    }

    /// Create a new WebUI handler with a plugin factory.
    ///
    /// Each render call creates a fresh plugin instance from the factory,
    /// enabling concurrent renders with `&self`.
    pub fn with_plugin(factory: fn() -> Box<dyn HandlerPlugin>) -> Self {
        Self {
            plugin_factory: Some(factory),
        }
    }

    /// Process a WebUI protocol with the provided state and write the output to the given writer.
    ///
    /// `options.entry_id` selects the fragment to start rendering from.
    /// `options.request_path` controls server-side route matching.
    pub fn handle(
        &self,
        protocol: &WebUIProtocol,
        state: &Value,
        options: &RenderOptions<'_>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if !protocol.fragments.contains_key(options.entry_id) {
            return Err(HandlerError::MissingFragment(options.entry_id.to_string()));
        }

        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            request_path: options.request_path.to_string(),
            route_base: "/".to_string(),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|f| f()),
            route_children: Vec::new(),
            entry_id: options.entry_id.to_string(),
            nonce: options.nonce.map(String::from),
        };
        self.process_fragment_id(options.entry_id, &mut context)?;

        writer.end()?;

        Ok(())
    }

    /// Like `handle()`, but pushes a component scope so the plugin emits
    /// binding markers. Use this when rendering a component outside the
    /// normal page render flow (e.g., re-rendering a route component with
    /// modified state).
    pub fn handle_as_component(
        &self,
        protocol: &WebUIProtocol,
        state: &Value,
        entry_id: &str,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if !protocol.fragments.contains_key(entry_id) {
            return Err(HandlerError::MissingFragment(entry_id.to_string()));
        }

        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            request_path: String::new(),
            route_base: "/".to_string(),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|f| f()),
            route_children: Vec::new(),
            entry_id: entry_id.to_string(),
            nonce: None,
        };

        if let Some(p) = &mut context.plugin {
            p.push_scope();
        }

        self.process_fragment_id(entry_id, &mut context)?;

        if let Some(p) = &mut context.plugin {
            p.pop_scope();
        }

        writer.end()?;

        Ok(())
    }

    /// Process a fragment by its ID.
    ///
    /// The `context` parameter contains scope-local variables that are accessible during rendering,
    /// such as loop iteration variables. This is separate from the global `state`.
    fn process_fragment_id(
        &self,
        fragment_id: &str,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if let Some(fragment_list) = context.protocol.fragments.get(fragment_id) {
            self.process_fragment(&fragment_list.fragments, context)
        } else {
            Err(HandlerError::MissingFragment(fragment_id.to_string()))
        }
    }

    /// Process a vector of fragments.
    ///
    /// The `context` maintains scope-specific variables that can be accessed by fragments
    /// during rendering, while `state` contains the global application state.
    fn process_fragment(
        &self,
        fragments: &[WebUIFragment],
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Pre-scan: find the best matching route among sibling routes by specificity.
        // This ensures `/contacts/add` (2 literals) beats `/contacts/:id` (1 literal).
        // Resolves relative paths (`./`) using the current route_base.
        let best_route = route_renderer::find_best_route_match(
            fragments,
            &context.request_path,
            &context.route_base,
        );

        for item in fragments {
            match item.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => {
                    context.writer.write(&raw.value)?;
                }
                Some(Fragment::Component(component)) => {
                    self.process_component(component, context)?;
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    self.process_for_loop(for_loop, context)?;
                }
                Some(Fragment::Signal(signal)) => {
                    self.process_signal(signal, context)?;
                }
                Some(Fragment::IfCond(if_cond)) => {
                    self.process_if(if_cond, context)?;
                }
                Some(Fragment::Attribute(attr)) => {
                    self.process_attribute(attr, context)?;
                }
                Some(Fragment::Plugin(plugin_frag)) => {
                    if let Some(p) = &mut context.plugin {
                        p.on_element_data(&plugin_frag.data, context.writer)?;
                    }
                }
                Some(Fragment::Route(route_frag)) => {
                    self.process_route(route_frag, &best_route, context)?;
                }
                Some(Fragment::Outlet(_)) => {
                    self.process_outlet(context)?;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Process an `<outlet />` directive.
    ///
    /// Matches children from the currently active route's `children` field
    /// against the request path, renders the matched child `<webui-route>`
    /// elements directly at this position (no wrapper element).
    fn process_outlet(&self, context: &mut WebUIProcessContext) -> Result<()> {
        let mut children = std::mem::take(&mut context.route_children);
        if children.is_empty() {
            return Ok(());
        }

        // Find the best matching child route
        let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
        for (idx, child) in children.iter().enumerate() {
            let resolved = route_matcher::resolve_route_path(&child.path, &context.route_base);
            if let Some(m) =
                route_matcher::match_single_route(&resolved, &context.request_path, child.exact)
            {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);
                if is_better {
                    best = Some((idx, m));
                }
            }
        }

        // Extract grandchildren from the matched child to avoid cloning.
        // We swap out the children vec so we can move it into context without
        // cloning, then swap an empty vec back for the sibling rendering pass.
        let grandchildren = if let Some((idx, _)) = &best {
            std::mem::take(&mut children[*idx].children)
        } else {
            Vec::new()
        };

        if let Some((idx, ref rm)) = best {
            let matched_child = &children[idx];
            let comp = &matched_child.fragment_id;

            if !comp.is_empty() {
                let saved_route_base = context.route_base.clone();
                let saved_route_children = std::mem::take(&mut context.route_children);

                if rm.consumed_segments > 0 {
                    context.route_base = route_matcher::compute_route_base(
                        &context.request_path,
                        rm.consumed_segments,
                    );
                }

                context.route_children = grandchildren;

                // Emit matched <webui-route>
                context.writer.write("<webui-route")?;
                if !matched_child.path.is_empty() {
                    context.writer.write(" path=\"")?;
                    context.writer.write(&matched_child.path)?;
                    context.writer.write("\"")?;
                }
                context.writer.write(" component=\"")?;
                context.writer.write(comp)?;
                context.writer.write("\"")?;
                if matched_child.exact {
                    context.writer.write(" exact")?;
                }
                if !matched_child.allowed_query.is_empty() {
                    context.writer.write(" query=\"")?;
                    context.writer.write(&matched_child.allowed_query)?;
                    context.writer.write("\"")?;
                }
                context.writer.write(" active>")?;

                context.writer.write("<")?;
                context.writer.write(comp)?;
                if let Some(p) = &context.plugin {
                    p.write_route_component_state(context.state, context.writer)?;
                }
                context.writer.write(">")?;

                self.process_component(
                    &webui_protocol::WebUIFragmentComponent {
                        fragment_id: comp.clone(),
                    },
                    context,
                )?;

                context.writer.write("</")?;
                context.writer.write(comp)?;
                context.writer.write(">")?;
                context.writer.write("</webui-route>")?;

                context.route_base = saved_route_base;
                context.route_children = saved_route_children;
            }
        }

        // Render non-matched siblings as hidden
        for (idx, child) in children.iter().enumerate() {
            let is_matched = best.as_ref().is_some_and(|(bi, _)| *bi == idx);
            if !is_matched && !child.fragment_id.is_empty() {
                context.writer.write("<webui-route")?;
                if !child.path.is_empty() {
                    context.writer.write(" path=\"")?;
                    context.writer.write(&child.path)?;
                    context.writer.write("\"")?;
                }
                context.writer.write(" component=\"")?;
                context.writer.write(&child.fragment_id)?;
                context.writer.write("\"")?;
                if child.exact {
                    context.writer.write(" exact")?;
                }
                if !child.allowed_query.is_empty() {
                    context.writer.write(" query=\"")?;
                    context.writer.write(&child.allowed_query)?;
                    context.writer.write("\"")?;
                }
                context
                    .writer
                    .write(" style=\"display:none\"></webui-route>")?;
            }
        }

        Ok(())
    }

    /// Emit a `<style type="module">` tag for a component's CSS module definition.
    ///
    /// Only emits on first render of this component (deduped by `rendered_components`).
    /// Placed in the component's light DOM so the browser can register the CSS module:
    /// `<my-comp><style type="module" specifier="my-comp">CSS</style><template ...>`.
    ///
    /// This keeps SSR output lean — only components actually rendered on the current
    /// route get their style definitions. Components on other routes receive their
    /// definitions via `templateStyles` during SPA partial navigation.
    fn emit_css_module(
        &self,
        component: &webui_protocol::WebUIFragmentComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if !context.rendered_components.contains(&component.fragment_id) {
            if let Some(css) = context
                .protocol
                .components
                .get(&component.fragment_id)
                .map(|c| c.css.as_str())
                .filter(|s| !s.is_empty())
            {
                context
                    .writer
                    .write("<style type=\"module\" specifier=\"")?;
                context.writer.write(&component.fragment_id)?;
                context.writer.write("\">")?;
                context.writer.write(css)?;
                context.writer.write("</style>")?;
            }
        }
        Ok(())
    }

    /// Process a route fragment — renders `<webui-route>` with matched/hidden state.
    fn process_route(
        &self,
        route_frag: &webui_protocol::WebUiFragmentRoute,
        best_route: &Option<(String, route_matcher::RouteMatch)>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let is_matched = best_route
            .as_ref()
            .is_some_and(|(best_key, _)| *best_key == route_frag.fragment_id);

        context.writer.write("<webui-route")?;
        if !route_frag.path.is_empty() {
            context.writer.write(" path=\"")?;
            context.writer.write(&route_frag.path)?;
            context.writer.write("\"")?;
        }
        if !route_frag.fragment_id.is_empty() {
            context.writer.write(" component=\"")?;
            context.writer.write(&route_frag.fragment_id)?;
            context.writer.write("\"")?;
        }
        if route_frag.exact {
            context.writer.write(" exact")?;
        }
        if !route_frag.allowed_query.is_empty() {
            context.writer.write(" query=\"")?;
            context.writer.write(&route_frag.allowed_query)?;
            context.writer.write("\"")?;
        }

        if is_matched {
            context.writer.write(" active>")?;

            if !route_frag.fragment_id.is_empty() {
                let saved_route_base = context.route_base.clone();
                let saved_route_children = std::mem::take(&mut context.route_children);
                if let Some((_, ref rm)) = best_route {
                    context.route_base = route_matcher::compute_route_base(
                        &context.request_path,
                        rm.consumed_segments,
                    );
                }

                context.route_children = route_frag.children.clone();

                let comp = webui_protocol::WebUIFragmentComponent {
                    fragment_id: route_frag.fragment_id.clone(),
                };

                context.writer.write("<")?;
                context.writer.write(&route_frag.fragment_id)?;
                if let Some(p) = &context.plugin {
                    p.write_route_component_state(context.state, context.writer)?;
                }
                context.writer.write(">")?;

                self.process_component(&comp, context)?;

                context.writer.write("</")?;
                context.writer.write(&route_frag.fragment_id)?;
                context.writer.write(">")?;

                context.route_base = saved_route_base;
                context.route_children = saved_route_children;
            }
        } else {
            context.writer.write(" style=\"display:none\">")?;
        }

        context.writer.write("</webui-route>")?;
        Ok(())
    }

    /// Process a component fragment.
    fn process_component(
        &self,
        component: &webui_protocol::WebUIFragmentComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Emit CSS module into the component's light DOM on first encounter.
        // Only rendered components get their <style type="module"> definition
        // during SSR. Components on other routes will receive theirs via the
        // templateStyles array in the SPA partial response instead.
        // Produces: <my-comp><style type="module" specifier="my-comp">CSS</style><template ...>
        if !context.rendered_components.contains(&component.fragment_id) {
            self.emit_css_module(component, context)?;
        }

        // Track this component as rendered (for selective f-template emission)
        context
            .rendered_components
            .insert(component.fragment_id.clone());

        // Save parent scope
        let saved_local_vars = std::mem::take(&mut context.local_vars);
        let saved_component_attrs = std::mem::take(&mut context.component_attrs);

        // Component gets accumulated attrs as its local vars
        context.local_vars = saved_component_attrs;

        if let Some(p) = &mut context.plugin {
            p.push_scope();
        }

        self.process_fragment_id(&component.fragment_id, context)?;

        if let Some(p) = &mut context.plugin {
            p.pop_scope();
        }

        // Restore parent scope
        context.local_vars = saved_local_vars;
        context.component_attrs = HashMap::new();

        Ok(())
    }

    /// Resolve a dotted path value, checking local variables first, then global state.
    fn resolve_value(&self, path: &str, context: &WebUIProcessContext) -> Option<Value> {
        // Check local vars first
        if let Some(first_part) = path.split('.').next() {
            if let Some(local_value) = context.local_vars.get(first_part) {
                if first_part.len() == path.len() {
                    return Some(local_value.clone());
                }
                let remaining = &path[first_part.len() + 1..];
                if let Some(v) = find_value_by_dotted_path(remaining, local_value) {
                    return Some(v);
                }
            }
        }
        // Fall back to global state
        find_value_by_dotted_path(path, context.state)
    }

    /// Evaluate a condition expression against the current context.
    ///
    /// Uses a resolver closure that checks local variables first, then falls
    /// back to global state — avoiding a full clone of the state tree.
    /// Returns false if the condition references a missing value.
    fn evaluate_condition(
        &self,
        condition: &webui_protocol::ConditionExpr,
        context: &WebUIProcessContext,
    ) -> Result<bool> {
        let local_vars = &context.local_vars;
        let state = context.state;
        match evaluate_with_resolver(condition, |path| {
            if let Some(first_part) = path.split('.').next() {
                if let Some(local_value) = local_vars.get(first_part) {
                    if first_part.len() == path.len() {
                        return Some(local_value.clone());
                    }
                    let remaining = &path[first_part.len() + 1..];
                    if let Some(v) = find_value_by_dotted_path(remaining, local_value) {
                        return Some(v);
                    }
                }
            }
            find_value_by_dotted_path(path, state)
        }) {
            Ok(result) => Ok(result),
            Err(ExpressionError::MissingValue(_)) => Ok(false),
            Err(e) => Err(HandlerError::Evaluation(e.to_string())),
        }
    }

    /// Process a for loop fragment.
    ///
    /// Creates a new context for each iteration that includes the current loop item.
    /// This allows nested templates to access both the loop variable and any parent context.
    /// Example: `for item in items` makes "item" available in the loop body.
    fn process_for_loop(
        &self,
        for_loop: &webui_protocol::WebUIFragmentFor,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let collection_name = &for_loop.collection;

        // If the collection is missing, treat it as empty (0 iterations) — matches NodeJS behavior.
        // Hydration comments are always emitted regardless of collection presence.
        let items = match self.resolve_value(collection_name, context) {
            Some(Value::Array(arr)) => arr,
            Some(_) => {
                return Err(HandlerError::TypeError(format!(
                    "Collection '{}' is not an array",
                    collection_name
                )))
            }
            None => Vec::new(),
        };

        if let Some(p) = &mut context.plugin {
            p.on_for_start(&for_loop.fragment_id, context.writer)?;
        }

        let item_name = &for_loop.item;
        for (i, item) in items.into_iter().enumerate() {
            if let Some(p) = &mut context.plugin {
                p.on_repeat_item_start(i, context.writer)?;
                p.push_scope();
            }

            // Save only the overwritten key instead of cloning the entire HashMap.
            let saved_value = context.local_vars.insert(item_name.clone(), item);
            self.process_fragment_id(&for_loop.fragment_id, context)?;
            match saved_value {
                Some(v) => {
                    context.local_vars.insert(item_name.clone(), v);
                }
                None => {
                    context.local_vars.remove(item_name.as_str());
                }
            }

            if let Some(p) = &mut context.plugin {
                p.pop_scope();
                p.on_repeat_item_end(i, context.writer)?;
            }
        }

        if let Some(p) = &mut context.plugin {
            p.on_for_end(&for_loop.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process a signal fragment.
    ///
    /// Looks up the value in the context first (for local variables), then in the global state.
    /// This prioritization allows local variables (like loop items) to override global state.
    /// If the value is not found in either scope, an empty string is returned.
    fn process_signal(
        &self,
        signal: &webui_protocol::WebUIFragmentSignal,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Hook: emit nonce meta and CSS <link> tags before </head>
        if signal.raw && signal.value == "head_end" {
            if let Some(ref nonce) = context.nonce {
                context
                    .writer
                    .write("<meta name=\"webui-nonce\" content=\"")?;
                context.writer.write(&html_escape::encode_safe(nonce))?;
                context.writer.write("\">")?;
            }

            // Emit CSS <link> tags in <head> for Link-strategy components.
            // Only Link-strategy components have css_href set.
            // Style-strategy embeds CSS inside the shadow DOM template.
            // Module-strategy emits <style type="module"> inline in each
            // component's light DOM during rendering (via emit_css_module).
            let (needed_components, _) = crate::route_handler::get_needed_components_for_request(
                context.protocol,
                &context.entry_id,
                &context.request_path,
                "",
            );
            for name in &needed_components {
                if let Some(href) = context
                    .protocol
                    .components
                    .get(name)
                    .map(|c| &c.css_href)
                    .filter(|h| !h.is_empty())
                {
                    context.writer.write("<link rel=\"stylesheet\" href=\"")?;
                    context.writer.write(href)?;
                    context.writer.write("\">")?;
                }
            }
        }

        // Hook: emit component templates before body_end when hydration is enabled.
        if signal.raw && signal.value == "body_end" && context.plugin.is_some() {
            // Build the component → index map for the inventory bitfield.
            let comp_index = crate::route_handler::build_component_index(context.protocol);

            // Emit inventory meta tag based on actually rendered components.
            // Placed here (not head_end) because rendered_components is only
            // complete after the full SSR pass. The router reads it via
            // document.querySelector regardless of position.
            let (_, inventory_hex) = crate::route_handler::filter_needed_components(
                &context.rendered_components,
                "",
                &comp_index,
            );
            context
                .writer
                .write("<meta name=\"webui-inventory\" content=\"")?;
            context.writer.write(&inventory_hex)?;
            context.writer.write("\">")?;

            // Emit templates for all REACHABLE components on the current route,
            // not just those rendered in this SSR pass. Components inside false
            // <if> blocks or empty <for> loops are reachable via client-side
            // state changes and need their templates available without a server
            // round-trip. The graph walker follows conditional and loop branches
            // unconditionally, but only descends into the matched route chain —
            // components on other routes are delivered via SPA partial navigation.
            let (reachable_names, _) = crate::route_handler::get_needed_components_for_request(
                context.protocol,
                &context.entry_id,
                &context.request_path,
                "",
            );
            let reachable: std::collections::HashSet<String> =
                reachable_names.into_iter().collect();

            // Emit CSS module definitions for reachable-but-unrendered components.
            // Rendered components already got their <style type="module"> inline
            // during the render pass (via emit_css_module). Unrendered components
            // need their definitions here so the framework can adopt them when
            // the <if> condition flips true client-side.
            for name in &reachable {
                if !context.rendered_components.contains(name) {
                    if let Some(css) = context
                        .protocol
                        .components
                        .get(name)
                        .map(|c| c.css.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        context
                            .writer
                            .write("<style type=\"module\" specifier=\"")?;
                        context.writer.write(name)?;
                        context.writer.write("\">")?;
                        context.writer.write(css)?;
                        context.writer.write("</style>")?;
                    }
                }
            }

            if let Some(ref p) = context.plugin {
                p.emit_templates(
                    context.protocol,
                    &reachable,
                    context.nonce.as_deref(),
                    context.writer,
                )?;
            }

            // Emit initial state as JSON for client-side hydration.
            // Like Preact/Next.js, we pass the same state used for SSR
            // so the client doesn't need to reconstruct it from the DOM.
            if let Some(ref nonce) = context.nonce {
                context.writer.write("<script nonce=\"")?;
                context.writer.write(nonce)?;
                context.writer.write("\">window.__webui_state=")?;
            } else {
                context.writer.write("<script>window.__webui_state=")?;
            }
            let state_json = serde_json::to_string(context.state).unwrap_or_default();
            // Escape </script> inside JSON to prevent premature tag closure
            let safe_json = state_json.replace("</", "<\\/");
            context.writer.write(&safe_json)?;
            context.writer.write("</script>\n")?;
        }

        if let Some(p) = &mut context.plugin {
            p.on_binding_start(&signal.value, context.writer)?;
        }

        if let Some(value) = self.resolve_value(&signal.value, context) {
            self.write_signal_value(&value, signal.raw, context.writer)?;
        }

        if let Some(p) = &mut context.plugin {
            p.on_binding_end(&signal.value, context.writer)?;
        }
        Ok(())
    }

    /// Write a signal value directly to the writer, avoiding intermediate String allocation.
    /// For HTML-escaped output, writes the Cow from `encode_safe` directly.
    fn write_signal_value(
        &self,
        value: &Value,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if raw {
            match value {
                Value::String(s) => writer.write(s),
                _ => writer.write(&value.to_string()),
            }
        } else {
            match value {
                Value::String(s) => writer.write(&html_escape::encode_safe(s)),
                _ => {
                    let s = value.to_string();
                    writer.write(&html_escape::encode_safe(&s))
                }
            }
        }
    }

    /// Process an if condition fragment.
    fn process_if(
        &self,
        if_cond: &webui_protocol::WebUIFragmentIf,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(|| HandlerError::Rendering("If fragment missing condition".to_string()))?;
        let condition_met = self.evaluate_condition(condition, context)?;

        if let Some(p) = &mut context.plugin {
            p.on_if_start(&if_cond.fragment_id, context.writer)?;
        }

        if condition_met {
            if let Some(p) = &mut context.plugin {
                p.push_scope();
            }

            self.process_fragment_id(&if_cond.fragment_id, context)?;

            if let Some(p) = &mut context.plugin {
                p.pop_scope();
            }
        }

        if let Some(p) = &mut context.plugin {
            p.on_if_end(&if_cond.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process an attribute fragment by rendering the attribute name/value pair.
    fn process_attribute(
        &self,
        attr: &webui_protocol::WebUIFragmentAttribute,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Initialize component attribute accumulator on attrStart
        if attr.attr_start {
            context.component_attrs = HashMap::new();
        }

        // Boolean attribute with condition tree
        if let Some(condition) = &attr.condition_tree {
            let condition_met = self.evaluate_condition(condition, context)?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::Bool(condition_met));
            }

            if condition_met {
                context.writer.write(" ")?;
                context.writer.write(&attr.name)?;
            }
            return Ok(());
        }

        // Template attribute (mixed static + dynamic)
        if !attr.template.is_empty() {
            let raw_value = self.render_template_attr_value(&attr.template, context)?;
            let escaped = html_escape::encode_safe(&raw_value);
            write_attr(context.writer, &attr.name, &escaped)?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::String(raw_value));
            }
            return Ok(());
        }

        // Simple attribute
        if !attr.value.is_empty() {
            if attr.raw_value {
                // Static attribute — value is the literal string
                write_attr(context.writer, &attr.name, &attr.value)?;
                if !attr.attr_skip {
                    let name = component_attr_name(&attr.name);
                    context
                        .component_attrs
                        .insert(name, Value::String(attr.value.clone()));
                }
            } else if attr.complex {
                // Complex attribute — resolve value, don't render to HTML, store as state
                if let Some(value) = self.resolve_value(&attr.value, context) {
                    if !attr.attr_skip {
                        let stripped = attr.name.strip_prefix(':').unwrap_or(&attr.name);
                        let name = component_attr_name(stripped);
                        context.component_attrs.insert(name, value);
                    }
                }
            } else {
                // Dynamic attribute — resolve and render
                let value = self.resolve_value(&attr.value, context);
                // Always emit the attribute so FAST hydration binding
                // markers (data-fe-b-N) match the DOM node structure.
                match &value {
                    Some(Value::String(s)) => {
                        write_attr(context.writer, &attr.name, &html_escape::encode_safe(s))?;
                    }
                    Some(Value::Null) | None => {
                        write_attr(context.writer, &attr.name, "")?;
                    }
                    Some(other) => {
                        let s = other.to_string();
                        write_attr(context.writer, &attr.name, &s)?;
                    }
                }

                if !attr.attr_skip {
                    let name = component_attr_name(&attr.name);
                    context
                        .component_attrs
                        .insert(name, value.unwrap_or(Value::String(String::new())));
                }
            }
        }

        Ok(())
    }

    /// Render a template attribute's fragments into a raw (unescaped) string.
    fn render_template_attr_value(
        &self,
        template_id: &str,
        context: &WebUIProcessContext,
    ) -> Result<String> {
        let fragments = context
            .protocol
            .fragments
            .get(template_id)
            .ok_or_else(|| HandlerError::MissingFragment(template_id.to_string()))?;
        let mut raw_value = String::new();
        for frag in &fragments.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => raw_value.push_str(&raw.value),
                Some(Fragment::Signal(signal)) => {
                    if let Some(value) = self.resolve_value(&signal.value, context) {
                        match &value {
                            Value::String(s) => raw_value.push_str(s),
                            _ => raw_value.push_str(&value.to_string()),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(raw_value)
    }

    /// Render the UI based on the protocol and state.
    ///
    /// Like `handle()` but does not call `writer.end()`.
    pub fn render(
        &self,
        protocol: &WebUIProtocol,
        state: &Value,
        options: &RenderOptions<'_>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            request_path: options.request_path.to_string(),
            route_base: "/".to_string(),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|f| f()),
            route_children: Vec::new(),
            entry_id: options.entry_id.to_string(),
            nonce: options.nonce.map(String::from),
        };

        self.process_fragment_id(options.entry_id, &mut context)?;

        Ok(())
    }
}

impl Default for WebUIHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Write ` name="value"` to the writer without allocating a format string.
fn write_attr(writer: &mut dyn ResponseWriter, name: &str, value: &str) -> Result<()> {
    writer.write(" ")?;
    writer.write(name)?;
    writer.write("=\"")?;
    writer.write(value)?;
    writer.write("\"")
}

/// Process a WebUI protocol with the provided state and write the output to the given writer.
/// This is the main entry point for the WebUI handler.
pub fn handle(
    protocol: &WebUIProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    let handler = WebUIHandler::new();
    handler.handle(protocol, state, options, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use webui_protocol::{
        web_ui_fragment, ComparisonOperator, ConditionExpr, FragmentList, LogicalOperator,
        WebUIFragmentAttribute,
    };
    use webui_test_utils::test_json;

    // A simple test writer implementation
    struct TestWriter {
        content: RefCell<String>,
        ended: RefCell<bool>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                content: RefCell::new(String::new()),
                ended: RefCell::new(false),
            }
        }

        fn get_content(&self) -> String {
            self.content.borrow().clone()
        }

        fn is_ended(&self) -> bool {
            *self.ended.borrow()
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.content.borrow_mut().push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            *self.ended.borrow_mut() = true;
            Ok(())
        }
    }

    #[test]
    fn test_handle_raw() {
        // Create a simple protocol
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Hello, WebUI!")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle raw protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_signal() {
        // Create a protocol with a signal
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "WebUI"});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle signal protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_for_loop() {
        // Create a protocol with a for loop
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("People: "),
                    WebUIFragment::for_loop("person", "people", "person-item"),
                ],
            },
        );

        fragments.insert(
            "person-item".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("person.name", false),
                    WebUIFragment::raw(", "),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "people": [
                {"name": "Alice"},
                {"name": "Bob"},
                {"name": "Charlie"}
            ]
        });

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle for loop protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "People: Alice, Bob, Charlie, ");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_if_condition() {
        // Create a protocol with an if condition
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Status: "),
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("isActive"),
                        "active-content",
                    ),
                    WebUIFragment::raw("End"),
                ],
            },
        );

        fragments.insert(
            "active-content".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Active")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);

        // Test with isActive = true
        let state_true = test_json!({"isActive": true});
        let mut writer_true = TestWriter::new();
        assert!(
            handle(
                &protocol,
                &state_true,
                &RenderOptions::new("index.html", "/"),
                &mut writer_true
            )
            .is_ok(),
            "Failed to handle if condition (true case)"
        );
        assert_eq!(writer_true.get_content(), "Status: ActiveEnd");
        assert!(writer_true.is_ended());

        // Test with isActive = false
        let state_false = test_json!({"isActive": false});
        let mut writer_false = TestWriter::new();
        assert!(
            handle(
                &protocol,
                &state_false,
                &RenderOptions::new("index.html", "/"),
                &mut writer_false
            )
            .is_ok(),
            "Failed to handle if condition (false case)"
        );
        assert_eq!(writer_false.get_content(), "Status: End");
        assert!(writer_false.is_ended());
    }

    #[test]
    fn test_handle_component() {
        // Create a protocol with a component
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Component: "),
                    WebUIFragment::component("my-component"),
                ],
            },
        );

        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Component Content</div>")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle component protocol"
        );

        // Check the output
        assert_eq!(
            writer.get_content(),
            "Component: <div>Component Content</div>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_missing_fragment() {
        // Create a protocol with a missing fragment reference
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("missing-component")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        let result = handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );

        // Expect an error
        assert!(result.is_err());
        if let Err(HandlerError::MissingFragment(fragment_id)) = result {
            assert_eq!(fragment_id, "missing-component");
        } else {
            panic!("Expected MissingFragment error");
        }
    }

    #[test]
    fn test_missing_signal_renders_empty() {
        // A signal referencing a field absent from state should render as empty
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("missing_field", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        let mut writer = TestWriter::new();

        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Missing signal should not produce an error"
        );

        assert_eq!(writer.get_content(), "Hello, !");
        assert!(writer.is_ended());
    }

    // ── Boolean attribute rendering tests ─────────────────────────────

    #[test]
    fn test_boolean_attr_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    #[test]
    fn test_boolean_attr_missing() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\">");
    }

    #[test]
    fn test_boolean_attr_multiple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("disabled"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"checked": true, "disabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\" checked>");
    }

    // ── Simple attribute rendering tests ──────────────────────────────

    #[test]
    fn test_attribute_with_value() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute("value", "inputValue"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"inputValue": "Hello"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input value=\"Hello\">");
    }

    #[test]
    fn test_attribute_with_falsy_numeric() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div name=\"test\""),
                    WebUIFragment::attribute("handle", "number"),
                    WebUIFragment::raw("></div>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"number": 0});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div name=\"test\" handle=\"0\"></div>"
        );
    }

    // ── Template attribute rendering tests ────────────────────────────

    #[test]
    fn test_mixed_attribute_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute_template("value", "attr-1"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        fragments.insert(
            "attr-1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "world"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input value=\"hello world\">");
    }

    // ── Raw signal rendering test ─────────────────────────────────────

    #[test]
    fn test_raw_signal_not_escaped() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("html", false),
                    WebUIFragment::signal("html", true),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"html": "<strong>hi</strong>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "&lt;strong&gt;hi&lt;&#x2F;strong&gt;<strong>hi</strong>"
        );
    }

    // ── Nested for loop tests ─────────────────────────────────────────

    #[test]
    fn test_nested_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outer"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outer".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "inner"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "inner".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Inner</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "A"}, {"name": "B"}]},
                {"innerItems": [{"name": "C"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Inner</span><span>Inner</span></div><div><span>Inner</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_signals() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Item1</span><span>Item2</span></div><div><span>Item3</span><span>Item4</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_global_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::signal("globalOuter", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::signal("globalInner", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalOuter": "GO",
            "globalInner": "GI",
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div>GO<span>Item1GI</span><span>Item2GI</span></div><div>GO<span>Item3GI</span><span>Item4GI</span></div></div>"
        );
    }

    // ── For + If state scoping tests ──────────────────────────────────

    #[test]
    fn test_if_in_for_uses_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.visible"),
                    "visible-tpl",
                )],
            },
        );
        fragments.insert(
            "visible-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item.name", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Show", "visible": true}, {"name": "Hide", "visible": false}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "Show");
    }

    #[test]
    fn test_for_if_local_overrides_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.flag"),
                    "show-tpl",
                )],
            },
        );
        fragments.insert(
            "show-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        // Global flag is true, but local item.flag is false for second item
        let state = test_json!({"flag": true, "items": [{"flag": true}, {"flag": false}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "yes");
    }

    // ── Component attribute state tests ───────────────────────────────

    #[test]
    fn test_component_attr_state_simple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                attr_start: true,
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"Attribute Title\"><span>Attribute Title</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_state_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "title-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "title-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "<world>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"hello &lt;world&gt;\"><span>hello &lt;world&gt;</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_camel_case() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "dt-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "dt-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("prefix "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "a&b"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp data-title=\"prefix a&amp;b\"><span>prefix a&amp;b</span></my-comp>"
        );
    }

    #[test]
    fn test_component_complex_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp><span>1</span><p>true</p></my-comp>"
        );
    }

    #[test]
    fn test_component_no_parent_pollution() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "var".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent"),
                    WebUIFragment::raw("</parent>"),
                ],
            },
        );
        fragments.insert(
            "parent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Before: "),
                    WebUIFragment::signal("var", false),
                    WebUIFragment::raw("<child foo"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "replaced".into(),
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child"),
                    WebUIFragment::raw("Label</child>After: "),
                    WebUIFragment::signal("var", false),
                ],
            },
        );
        fragments.insert("child".to_string(), FragmentList { fragments: vec![] });
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"var": "original"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent var=\"original\">Before: original<child foo var=\"replaced\">Label</child>After: original</parent>"
        );
    }

    #[test]
    fn test_component_boolean_attr_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("disabled"),
                    "show",
                )],
            },
        );
        fragments.insert(
            "show".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("disabled!")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp disabled>disabled!</my-comp>"
        );
    }

    // ===== HTML Escape Tests (ported from utils.test.js escapeHtml) =====

    /// Helper: render a signal value through the handler and return the escaped output.
    fn render_signal(value: &str) -> String {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("v", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"v": value});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        writer.get_content()
    }

    #[test]
    fn test_escape_ampersand() {
        assert_eq!(render_signal("&"), "&amp;");
    }

    #[test]
    fn test_escape_less_than() {
        assert_eq!(render_signal("<"), "&lt;");
    }

    #[test]
    fn test_escape_greater_than() {
        assert_eq!(render_signal(">"), "&gt;");
    }

    #[test]
    fn test_escape_double_quote() {
        assert_eq!(render_signal("\""), "&quot;");
    }

    #[test]
    fn test_escape_single_quote() {
        // html_escape::encode_safe escapes ' as &#x27;
        let result = render_signal("'");
        assert!(
            result == "&#39;" || result == "&#x27;" || result == "'",
            "Expected escaped single quote, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_multiple_special_chars() {
        let result = render_signal("<script>alert('xss');</script>");
        assert!(
            result.contains("&lt;") && result.contains("&gt;"),
            "Expected escaped HTML, got: {}",
            result
        );
        assert!(
            !result.contains("<script>"),
            "Should not contain raw <script> tag"
        );
    }

    #[test]
    fn test_escape_no_special_chars() {
        assert_eq!(render_signal("Hello World"), "Hello World");
    }

    #[test]
    fn test_escape_empty_string() {
        assert_eq!(render_signal(""), "");
    }

    #[test]
    fn test_escape_special_at_beginning() {
        let result = render_signal("<Hello");
        assert!(
            result.starts_with("&lt;"),
            "Expected &lt; at start, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_at_end() {
        let result = render_signal("Hello>");
        assert!(
            result.ends_with("&gt;"),
            "Expected &gt; at end, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_in_middle() {
        let result = render_signal("Hel&lo");
        assert!(
            result.contains("&amp;"),
            "Expected &amp; in middle, got: {}",
            result
        );
    }

    // ── GROUP 5: Boolean Attribute Edge Cases ─────────────────────────

    #[test]
    fn test_boolean_attr_truthy_values() {
        // checked: 1
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": 1});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: "yes"
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": "yes"});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: {} (empty object is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": {}});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            // Empty object is falsy in this expression evaluator
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: "false" (string "false" is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": "false"});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
    }

    #[test]
    fn test_boolean_attr_falsy_values() {
        // checked: 0
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": 0});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: ""
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": ""});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: false
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": false});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // no checked key at all
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
    }

    #[test]
    fn test_boolean_attr_expression_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"itemCount": 5});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_expression_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"itemCount": 3});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    // ── GROUP 6: Mixed Attributes ─────────────────────────────────────

    #[test]
    fn test_nested_component_attr_capture() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello "),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Child of "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"who": "<world>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Hello &lt;world&gt;\"><child-component title=\"Child of Hello &lt;world&gt;\"><span>Child of Hello &lt;world&gt;</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_grandchild_attr_propagation() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "p-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "p-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("P:"), WebUIFragment::signal("p", false)],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "c-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "c-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("C("),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw(")-"),
                    WebUIFragment::signal("cExtra", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"p": "<p>", "cExtra": "x&y"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"P:&lt;p&gt;\"><child-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><grandchild-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><span>C(P:&lt;p&gt;)-x&amp;y</span></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_for_loop_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Parent:"),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "child-loop")],
            },
        );
        fragments.insert(
            "child-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hi "),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"who": "Bob", "items": [{"name": "A<1>"}, {"name": "B&2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent:Bob\"><child-component title=\"Hi A&lt;1&gt; &#x2F; Parent:Bob\"><span>Hi A&lt;1&gt; &#x2F; Parent:Bob</span></child-component><child-component title=\"Hi B&amp;2 &#x2F; Parent:Bob\"><span>Hi B&amp;2 &#x2F; Parent:Bob</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_multiple_template_attrs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "attr-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "attr-data-title".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                template: "attr-aria-label".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("T:"), WebUIFragment::signal("t", false)],
            },
        );
        fragments.insert(
            "attr-data-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("D:"), WebUIFragment::signal("d", false)],
            },
        );
        fragments.insert(
            "attr-aria-label".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("A:"), WebUIFragment::signal("a", false)],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"t": "<t&1>", "d": "d<2>", "a": "a&3"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"T:&lt;t&amp;1&gt;\" data-title=\"D:d&lt;2&gt;\" aria-label=\"A:a&amp;3\"><span>T:&lt;t&amp;1&gt;|D:d&lt;2&gt;|A:a&amp;3</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_local_and_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "loop")],
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title", "items": [{"title": "Local Title"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_boolean_attr_first_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "label".into(),
                                value: "Component Label".into(),
                                raw_value: true,
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Disabled</div>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled label=\"Component Label\"><div>Disabled</div><span>Component Label</span></my-component>"
        );
    }

    #[test]
    fn test_hyphenated_attr_camelcase() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "key-hyphen".into(),
                                value: "Local Value".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("keyHyphen", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"keyHyphen": "Global Value"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component key-hyphen=\"Local Value\"><span>Local Value</span></my-component>"
        );
    }

    #[test]
    fn test_skipped_component_attrs() {
        // Skipped attributes: class, style, role, data-*, aria-*
        // Plus framework-specific prefixes/names that the parser marks with attr_skip.
        // These render on the HTML element but are NOT passed into component attribute state.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<test-component"),
                    // Skipped: class
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "class".into(),
                                value: "skippedClass".into(),
                                attr_start: true,
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: style
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "style".into(),
                                value: "skippedStyle".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: role
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "role".into(),
                                value: "skippedRole".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: data-testid (data-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-testid".into(),
                                value: "skippedDataTestid".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: aria-label (aria-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                value: "skippedAriaLabel".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // NOT skipped: title
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("test-component"),
                    WebUIFragment::raw("</test-component>"),
                ],
            },
        );
        fragments.insert(
            "test-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("class", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("style", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("role", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("dataTestid", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "title": "Hello",
            "skippedClass": "my-class",
            "skippedStyle": "color:red",
            "skippedRole": "button",
            "skippedDataTestid": "test-id",
            "skippedAriaLabel": "label-text"
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        // Skipped attrs render on the element but their values are NOT accessible inside the component.
        // The component's signals for skipped attrs resolve to empty strings.
        // Only "title" (non-skipped) is accessible.
        assert_eq!(
            writer.get_content(),
            "<test-component class=\"my-class\" style=\"color:red\" role=\"button\" data-testid=\"test-id\" aria-label=\"label-text\" title=\"Hello\"><span>Hello-----</span></test-component>"
        );
    }

    // ── GROUP 7: Attribute Inheritance ─────────────────────────────────

    #[test]
    fn test_attr_inherit_parent_to_child() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h1><child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h2>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h2>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><h1>Parent Title</h1><child-component title=\"Parent Title\"><h2>Parent Title</h2></child-component></parent-component>"
        );
    }

    #[test]
    fn test_attr_inherit_deep() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Child Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h3>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h3>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><child-component title=\"Child Title\"><grandchild-component title=\"Child Title\"><h3>Child Title</h3></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_complex_attr_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>1</span><p>true</p></my-component>"
        );
    }

    #[test]
    fn test_complex_attr_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "item",
                    "list.items",
                    "listTemplate",
                )],
            },
        );
        fragments.insert(
            "listTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "item".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::component("item_component"),
                ],
            },
        );
        fragments.insert(
            "item_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"list": {"items": [{"name": "Alice"}, {"name": "Bob"}]}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<span>Alice</span><span>Bob</span>");
    }

    #[test]
    fn test_complex_attr_nested_for() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "outer",
                    "data.outer",
                    "outerTemplate",
                )],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "middle",
                    "outer.middle",
                    "middleTemplate",
                )],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "inner",
                    "middle.inner",
                    "innerTemplate",
                )],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<card"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":outer".into(),
                                value: "outer".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":middle".into(),
                                value: "middle".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":inner".into(),
                                value: "inner".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("card_component"),
                    WebUIFragment::raw("</card>"),
                ],
            },
        );
        fragments.insert(
            "card_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("outer.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("middle.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("inner.label", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"data": {"outer": [
            {"label": "Outer1", "middle": [{"label": "Middle1", "inner": [{"label": "Inner1A"}, {"label": "Inner1B"}]}]},
            {"label": "Outer2", "middle": [{"label": "Middle2", "inner": [{"label": "Inner2A"}]}]}
        ]}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<card><p>Outer1 / Middle1 / Inner1A</p></card><card><p>Outer1 / Middle1 / Inner1B</p></card><card><p>Outer2 / Middle2 / Inner2A</p></card>"
        );
    }

    // ── GROUP 8: Boolean Component State ──────────────────────────────

    #[test]
    fn test_bool_component_state_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled><span>Disabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>Enabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_forward() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "parentDisabledTemplate",
                    ),
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("disabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "parentDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Parent Disabled</div>")],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "childDisabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "childEnabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "childDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Disabled</div>")],
            },
        );
        fragments.insert(
            "childEnabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Enabled</div>")],
            },
        );

        // Test case 1: isDisabled = true
        {
            let protocol = WebUIProtocol::new(fragments.clone());
            let state = test_json!({"isDisabled": true});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component disabled><div>Parent Disabled</div><child-component disabled><div>Child Disabled</div></child-component></parent-component>"
            );
        }

        // Test case 2: isDisabled = false
        {
            let protocol = WebUIProtocol::new(fragments.clone());
            let state = test_json!({"isDisabled": false});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component><child-component><div>Child Enabled</div></child-component></parent-component>"
            );
        }
    }

    // ── GROUP 9: Hydration (SKIP) ─────────────────────────────────────

    // TODO: test_hydration – requires FastHydrationPlugin integration; see plugin/fast.rs

    // ── Component tests ──────────────────────────────────────────────

    #[test]
    fn test_component_with_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Custom Element</div>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element><div>Custom Element</div></custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_component_with_slots() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element appearance=\"subtle\">"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("Hello World</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<slot></slot>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element appearance=\"subtle\"><slot></slot>Hello World</custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_multiple_nested_components() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateRepeat"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "custom-button".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<slot></slot>")],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-child>"),
                    WebUIFragment::component("custom-child"),
                    WebUIFragment::raw("</custom-child><slot></slot>"),
                ],
            },
        );
        fragments.insert(
            "custom-child".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>Hello World!</h1>")],
            },
        );
        fragments.insert(
            "templateRepeat".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("<custom-button>"),
                    WebUIFragment::component("custom-button"),
                    WebUIFragment::raw("Ok</custom-button></custom-element>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Item1"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><custom-element><custom-child><h1>Hello World!</h1></custom-child><slot></slot><custom-button><slot></slot>Ok</custom-button></custom-element></div>"
        );
        assert!(writer.is_ended());
    }

    // ── Conditional tests ────────────────────────────────────────────

    #[test]
    fn test_if_with_binary_expression() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::predicate("x", ComparisonOperator::GreaterThan, "5"),
                        "if-1",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>If 1</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        // True case: x = 10 > 5
        let state_true = test_json!({"x": 10});
        let mut writer_true = TestWriter::new();
        handle(
            &protocol,
            &state_true,
            &RenderOptions::new("index.html", "/"),
            &mut writer_true,
        )
        .unwrap();
        assert_eq!(writer_true.get_content(), "<div><span>If 1</span></div>");

        // False case: x = 1 <= 5
        let state_false = test_json!({"x": 1});
        let mut writer_false = TestWriter::new();
        handle(
            &protocol,
            &state_false,
            &RenderOptions::new("index.html", "/"),
            &mut writer_false,
        )
        .unwrap();
        assert_eq!(writer_false.get_content(), "<div></div>");
    }

    #[test]
    fn test_for_if_overlapping_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "flag": false,
            "items": [
                {"label": "A", "flag": true},
                {"label": "B", "flag": false},
                {"label": "C", "flag": true}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>A</span></div><div></div><div><span>C</span></div></div>"
        );
    }

    #[test]
    fn test_for_if_global_flag_no_effect() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"flag": true},
            "items": [
                {"label": "A", "flag": false},
                {"label": "B", "flag": true},
                {"label": "C", "flag": false}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div></div><div><span>B</span></div><div></div></div>"
        );
    }

    // ── Recursive template test ──────────────────────────────────────

    #[test]
    fn test_recursive_template_refs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "static")],
            },
        );
        fragments.insert(
            "static".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div expanded=\""),
                    WebUIFragment::signal("item.expanded", false),
                    WebUIFragment::raw("\" class=\""),
                    WebUIFragment::signal("testScenario", false),
                    WebUIFragment::raw("\"><span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                    WebUIFragment::for_loop("item", "item.children", "static"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "testScenario": "RecursiveTemplatesWithGlobalState",
            "items": [
                {"name": "A", "expanded": "false", "children": []},
                {"name": "B", "expanded": "true", "children": [
                    {"name": "C", "expanded": "false"},
                    {"name": "D", "expanded": "false"}
                ]},
                {"name": "E", "expanded": "false"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>A</span></div><div expanded=\"true\" class=\"RecursiveTemplatesWithGlobalState\"><span>B</span><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>C</span></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>D</span></div></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>E</span></div>"
        );
    }

    // ── Advanced state management tests ──────────────────────────────

    #[test]
    fn test_component_in_for_no_local_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span></span></component-tag><component-tag><span></span></component-tag></div>"
        );
    }

    #[test]
    fn test_nested_for_hierarchical_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "outerItems": [
                {"outerLabel": "O1", "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<p>Prefix: O1: I1</p><p>Prefix: O1: I2</p></section><section>Prefix: O2<p>Prefix: O2: I3</p></section></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_only() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_component_no_item_moniker() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_for_nonqualified_uses_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "GlobalName", "items": [{"name": "LocalName1"}, {"name": "LocalName2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>GlobalName</span><span>GlobalName</span></div>"
        );
    }

    #[test]
    fn test_nested_for_if_interleaved() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.include"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "globalSuffix": "Suffix",
            "outerItems": [
                {"outerLabel": "O1", "include": true, "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "include": false, "innerItems": [{"innerLabel": "Iignored"}]},
                {"outerLabel": "O3", "include": true, "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<div><p>Suffix: I1</p><p>Suffix: I2</p></div></section><section>Prefix: O2</section><section>Prefix: O3<div><p>Suffix: I3</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_outer_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop(
                        "middleItem",
                        "outerItem.middleItems",
                        "middleTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.active"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("middleItem.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "GP-",
            "outerItems": [
                {"label": "O1", "active": true, "middleItems": [{"value": "M1"}, {"value": "M2"}]},
                {"label": "O2", "active": false, "middleItems": [{"value": "M3"}]},
                {"label": "O3", "active": true, "middleItems": [{"value": "M4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>GP-O1<div><p>M1</p></div><div><p>M2</p></div></section><section>GP-O2<div></div></section><section>GP-O3<div><p>M4</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_inner_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<article>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("innerItem.show"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</article>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("innerItem.detail", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"label": "Outer1", "innerItems": [{"detail": "Detail1", "show": true}, {"detail": "Detail2", "show": false}]},
                {"label": "Outer2", "innerItems": [{"detail": "Detail3", "show": true}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Outer1<article><p>Detail1</p></article><article></article></section><section>Outer2<article><p>Detail3</p></article></section></div>"
        );
    }

    #[test]
    fn test_for_merge_local_global_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>Local1-LOCAL-Only1-other</span><span>Local2-GLOBAL-Only2-other</span></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_moniker_shadow() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-GLOBAL--other</span></component-tag><component-tag><span>-GLOBAL--other</span></component-tag></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_local_flag() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"flag": true, "value": "X"}, {"flag": false, "value": "Y"}]}]},
            "inner_item": {"flag": false}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p></section></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_global_fallback() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"value": "X"}, {"value": "Y"}]}]},
            "inner_item": {"flag": true}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p><p>Y</p></section></div>"
        );
    }

    #[test]
    fn test_if_mixed_for_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outer.outerLabel", false),
                    WebUIFragment::for_loop("inner", "outer.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::compound(
                        ConditionExpr::identifier("outer.active"),
                        LogicalOperator::And,
                        ConditionExpr::predicate(
                            "inner.value",
                            ComparisonOperator::GreaterThan,
                            "globalLimit",
                        ),
                    ),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalLimit": 10,
            "list": {"outerItems": [
                {"outerLabel": "O1", "active": true, "innerItems": [{"value": 15}, {"value": 8}]},
                {"outerLabel": "O2", "active": false, "innerItems": [{"value": 20}]},
                {"outerLabel": "O3", "active": true, "innerItems": [{"value": 5}]}
            ]}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>O1<p>15</p></section><section>O2</section><section>O3</section></div>"
        );
    }

    // ── Route-aware rendering tests ─────────────────────────────────────

    fn make_route_protocol() -> WebUIProtocol {
        use webui_protocol::WebUiFragmentRoute;

        let mut fragments = HashMap::new();

        // Entry page with two routes
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>Shell</h1>"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/".into(),
                        fragment_id: "dash-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/contacts/:id".into(),
                        fragment_id: "detail-page".into(),
                        exact: true,
                        ..Default::default()
                    }),
                ],
            },
        );

        // Dashboard page component
        fragments.insert(
            "dash-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Dashboard</p>")],
            },
        );

        // Detail page component
        fragments.insert(
            "detail-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Detail</p>")],
            },
        );

        WebUIProtocol::new(fragments)
    }

    fn make_nested_route_protocol() -> WebUIProtocol {
        use webui_protocol::WebUiFragmentRoute;

        let mut fragments = HashMap::new();

        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "sections/:id".into(),
                        fragment_id: "section-comp".into(),
                        exact: false,
                        children: vec![WebUiFragmentRoute {
                            path: "topics/:topicId".into(),
                            fragment_id: "topic-comp".into(),
                            exact: true,
                            children: vec![],
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
            },
        );

        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>Shell</h1>"),
                    WebUIFragment::outlet(),
                ],
            },
        );

        fragments.insert(
            "section-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h2>Section</h2>"),
                    WebUIFragment::outlet(),
                ],
            },
        );

        fragments.insert(
            "topic-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Topic content</p>")],
            },
        );

        WebUIProtocol::new(fragments)
    }

    #[test]
    fn test_route_renders_shell_always() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Shell content always renders regardless of route matching
        assert!(html.contains("<h1>Shell</h1>"), "shell should render");
        // Dashboard matches "/" so it should be active
        assert!(html.contains(" active>"), "matched route should be active");
        // Detail should be hidden and empty
        assert!(
            html.contains("style=\"display:none\""),
            "non-matched routes should be hidden"
        );
    }

    #[test]
    fn test_route_matched_renders_visible() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Dashboard route should be visible (active, no display:none)
        assert!(
            html.contains("<webui-route path=\"/\""),
            "dashboard route should exist"
        );
        assert!(
            html.contains("active>") && html.contains("<dash-page>"),
            "matched route should be active with component tag: {html}"
        );
        assert!(
            html.contains("<p>Dashboard</p>"),
            "matched route should have content"
        );
    }

    #[test]
    fn test_route_non_matched_renders_hidden_empty() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Detail route should be hidden and empty (no content rendered)
        assert!(
            html.contains("<webui-route path=\"/contacts/:id\""),
            "detail route element should exist"
        );
        // The non-matched route should have display:none and no inner content
        let detail_start = html.find("path=\"/contacts/:id\"").expect("detail route");
        let after_detail = &html[detail_start..];
        assert!(
            after_detail.contains("style=\"display:none\">")
                && !after_detail.starts_with(&format!("path=\"/contacts/:id\"{}detail-page>", "")),
            "non-matched route should be hidden: {after_detail}"
        );
        // Should NOT contain the component's rendered content
        let detail_end = after_detail.find("</webui-route>").expect("closing tag");
        let detail_body = &after_detail[..detail_end];
        assert!(
            !detail_body.contains("<detail-page>"),
            "non-matched route should not render component content: {detail_body}"
        );
    }

    #[test]
    fn test_route_parameterized_match() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/contacts/42"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Detail route matches /contacts/42
        assert!(
            html.contains("active>") && html.contains("<detail-page>"),
            "detail route should be active: {html}"
        );
        assert!(html.contains("<p>Detail</p>"), "detail should have content");
        // Dashboard should be hidden + empty
        let dash_start = html
            .find("component=\"dash-page\"")
            .expect("dashboard route");
        let after_dash = &html[dash_start..];
        assert!(
            after_dash.contains("style=\"display:none\">"),
            "dashboard should be hidden when detail matches: {after_dash}"
        );
        let dash_end = after_dash.find("</webui-route>").expect("closing tag");
        let dash_body = &after_dash[..dash_end];
        assert!(
            !dash_body.contains("<dash-page>"),
            "dashboard should not render component content: {dash_body}"
        );
    }

    #[test]
    fn test_route_no_match_all_hidden_empty() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/nonexistent"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Shell content should still render
        assert!(html.contains("<h1>Shell</h1>"));
        // All routes should be hidden + empty (nothing matched)
        assert!(
            !html.contains("<p>Dashboard</p>"),
            "no route content when nothing matches"
        );
        assert!(
            !html.contains("<p>Detail</p>"),
            "no route content when nothing matches"
        );
    }

    #[test]
    fn test_route_component_attr_emitted() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();
        // component attribute should be emitted on webui-route
        assert!(
            html.contains("component=\"dash-page\""),
            "component attr should be on webui-route: {html}"
        );
        assert!(
            html.contains("component=\"detail-page\""),
            "component attr should be on webui-route: {html}"
        );
    }

    #[test]
    fn test_no_plugin_no_state_attributes() {
        let protocol = make_route_protocol();
        let state = test_json!({
            "title": "Fish & Chips",
            "cartOpen": true,
            "items": [{"name": "A&B"}]
        });
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        // Without a plugin, no state attributes at all
        assert!(
            !html.contains("data-state"),
            "no data-state without plugin: {html}"
        );
        assert!(
            !html.contains(r#"title="Fish"#),
            "no scalar attrs without plugin: {html}"
        );
    }

    #[test]
    fn test_nested_routes_render_webui_route_as_light_dom() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root route should be active: {html}"
        );
        // webui-route should NOT have shadow DOM — it's a light DOM structural element
        assert!(
            !html.contains("<template shadowrootmode"),
            "webui-route should be light DOM (no shadow template): {html}"
        );
    }

    #[test]
    fn test_nested_routes_render_outlet_as_light_dom() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        // No <webui-outlet> wrapper — routes render directly at outlet position
        assert!(
            !html.contains("<webui-outlet>"),
            "should not contain webui-outlet wrapper: {html}"
        );
        // Route elements should be in the output directly
        assert!(
            html.contains("<webui-route"),
            "should contain webui-route elements: {html}"
        );
    }

    #[test]
    fn test_nested_routes_match_child_at_outlet() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"section-comp\"") && html.contains("active>"),
            "section route should be active: {html}"
        );
        assert!(
            html.contains("<h2>Section</h2>"),
            "section content should be present: {html}"
        );
    }

    #[test]
    fn test_nested_routes_three_levels_deep() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend/topics/react"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root active: {html}"
        );
        assert!(
            html.contains("component=\"section-comp\"") && html.contains("active>"),
            "section active: {html}"
        );
        assert!(
            html.contains("component=\"topic-comp\"") && html.contains("exact active>"),
            "topic active: {html}"
        );
        assert!(
            html.contains("<p>Topic content</p>"),
            "leaf content present: {html}"
        );
    }

    #[test]
    fn test_nested_routes_nonmatched_siblings_hidden() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains(r#"component="topic-comp" exact style="display:none">"#),
            "topic should be hidden: {html}"
        );
    }

    #[test]
    fn test_nested_routes_root_only() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root active at /: {html}"
        );
        assert!(
            html.contains("<h1>Shell</h1>"),
            "shell renders at /: {html}"
        );
        assert!(
            html.contains(r#"component="section-comp" style="display:none">"#),
            "section hidden at /: {html}"
        );
    }

    // ── CSS Module dedup tests ───────────────────────────────────────

    #[test]
    fn test_css_module_emitted_once_inline_in_component() {
        // CSS module definition emitted once in the component's light DOM
        // on first render, not in <head> and not on second instance.
        let template = r#"<p><slot></slot></p>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body><div>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("A".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("B</div>".to_string()),
                    WebUIFragment::signal("body_end", true),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .css = "p{color:red}".to_string();
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        // CSS module should appear exactly once
        let count = html
            .matches(r#"<style type="module" specifier="my-card">"#)
            .count();
        assert_eq!(
            count, 1,
            "CSS module should be emitted once, got {count} in: {html}"
        );

        // Template content should appear twice (once per component instance)
        let tmpl_count = html.matches(r#"<p><slot></slot></p>"#).count();
        assert_eq!(
            tmpl_count, 2,
            "Template should render twice, got {tmpl_count} in: {html}"
        );

        // CSS module should be in <body> (inline), not in <head>
        let css_pos = html
            .find(r#"<style type="module""#)
            .expect("CSS module missing");
        let body_pos = html.find("<body>").expect("<body> missing");
        assert!(
            css_pos > body_pos,
            "CSS module should be inline in component, not in <head>: {html}"
        );
    }

    #[test]
    fn test_component_without_css_renders_normally() {
        // Components without CSS module prefix pass through unchanged
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(r#"<p>hello</p>"#.to_string())],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        assert!(
            html.contains("<p>hello</p>"),
            "Non-module component should render normally: {html}"
        );
    }

    #[test]
    fn test_non_module_strategy_no_css_in_head() {
        // When component_css is empty (Link/Style strategies), no
        // <style type="module"> tags should appear in <head>.
        let template = r#"<p>hello</p>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        // No component css populated — simulates Link/Style strategy
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            !html.contains(r#"<style type="module""#),
            "Non-module strategy should not emit CSS modules in <head>: {html}"
        );
        assert!(
            html.contains("<p>hello</p>"),
            "Component should still render: {html}"
        );
    }

    #[test]
    fn test_style_strategy_embeds_inline_style_in_shadow_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card></body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><style>.card{color:red}</style><div>card</div></template>"
                        .to_string(),
                )],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains("<style>.card{color:red}</style>"),
            "Style strategy should embed inline CSS in shadow template: {html}"
        );
        assert!(
            !html.contains(r#"<style type="module""#),
            "Style strategy should not emit module CSS in <head>: {html}"
        );
    }

    #[test]
    fn test_link_strategy_emits_link_tag_in_head() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card>".to_string()),
                    WebUIFragment::signal("body_end", true),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>card</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        let comp = protocol
            .components
            .entry("my-card".to_string())
            .or_default();
        comp.css_href = "/my-card.css".to_string();
        comp.template = "(function(){})();".to_string();

        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        let head_end = html.find("</head>").expect("</head> missing");
        let link_pos = html.find(r#"<link rel="stylesheet" href="/my-card.css">"#);
        assert!(
            link_pos.is_some_and(|p| p < head_end),
            "Link strategy should emit <link> in <head>: {html}"
        );
        assert!(
            !html.contains(r#"<style type="module""#),
            "Link strategy should not emit module CSS: {html}"
        );
    }

    #[test]
    fn test_css_module_emitted_in_component_light_dom() {
        // CSS module <style> tags are emitted inline in the component's light DOM,
        // not in <head>. This keeps SSR output lean — only rendered components
        // get their style definitions.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card>".to_string()),
                    WebUIFragment::signal("body_end", true),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(r#"<p>hi</p>"#.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .css = "p{color:red}".to_string();
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        // CSS module must be INSIDE the component tag (light DOM)
        let tag_open = html.find("<my-card>").expect("<my-card> missing");
        let css_pos = html
            .find(r#"<style type="module""#)
            .expect("CSS module missing");
        let tag_close = html.rfind("</my-card>").expect("</my-card> missing");
        assert!(
            css_pos > tag_open && css_pos < tag_close,
            "CSS module should be inside component light DOM: {html}"
        );

        // <head> should NOT contain module styles
        let head_end = html.find("</head>").expect("</head> missing");
        assert!(
            css_pos > head_end,
            "CSS module should not be in <head>: {html}"
        );
    }

    #[test]
    fn test_css_module_emitted_for_route_components() {
        // Route components get CSS modules emitted inline in their light DOM.
        let template = r#"<h1>Dashboard</h1>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::route("/", "dash-page"),
                    WebUIFragment::signal("body_end", true),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "dash-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        let comp = protocol
            .components
            .entry("dash-page".to_string())
            .or_default();
        comp.css = "h1{font-size:2rem}".to_string();
        comp.template = "(function(){})();".to_string();
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains(
                r#"<style type="module" specifier="dash-page">h1{font-size:2rem}</style>"#
            ),
            "Route component should have CSS module: {html}"
        );
        assert!(
            html.contains("<h1>Dashboard</h1>"),
            "Route component should render content: {html}"
        );
    }

    #[test]
    fn test_head_css_link_skipped_for_components_without_css() {
        // Regression: components without CSS files must not get <link> tags
        // in <head>, otherwise the browser requests a 404.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::component("has-css"),
                    WebUIFragment::component("no-css"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "has-css".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>styled</p>".to_string())],
            },
        );
        fragments.insert(
            "no-css".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>plain</p>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        // Only has-css has an external stylesheet (Link strategy)
        protocol
            .components
            .entry("has-css".to_string())
            .or_default()
            .css_href = "/has-css.css".to_string();

        let state = test_json!({});
        let mut writer = TestWriter::new();

        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let html = writer.get_content();
        assert!(
            html.contains(r#"<link rel="stylesheet" href="/has-css.css">"#),
            "Component with CSS should get a <link> in <head>: {html}"
        );
        assert!(
            !html.contains("no-css.css"),
            "Component without CSS must NOT get a <link> in <head>: {html}"
        );
    }

    #[test]
    fn test_reachable_unrendered_components_get_templates_and_css_but_not_inventory() {
        // Simulates a page where app-shell renders cart-panel, but cart-panel
        // contains an <if> block with product-card inside. When the condition
        // is false (empty cart), product-card is NOT rendered — but it IS
        // reachable from the fragment graph. Its template IIFE and CSS module
        // definition must be in the output so the client can mount it when
        // the <if> flips true. However, its bit must NOT be set in the
        // inventory — the inventory tracks what was actually rendered.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    WebUIFragment::signal("head_end", true),
                    WebUIFragment::raw("</head><body><app-shell>".to_string()),
                    WebUIFragment::component("app-shell"),
                    WebUIFragment::raw("</app-shell>".to_string()),
                    WebUIFragment::signal("body_end", true),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        // app-shell contains a cart panel
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>Shell</div>".to_string()),
                    WebUIFragment::component("cart-panel"),
                ],
            },
        );
        // cart-panel has an <if> block containing product-card
        fragments.insert(
            "cart-panel".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<aside>".to_string()),
                    WebUIFragment::if_cond(ConditionExpr::identifier("hasItems"), "cart-items"),
                    WebUIFragment::raw("</aside>".to_string()),
                ],
            },
        );
        // cart-items (if block body) contains product-card
        fragments.insert(
            "cart-items".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("product-card")],
            },
        );
        fragments.insert(
            "product-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Card</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        for name in ["app-shell", "cart-panel", "product-card"] {
            let comp = protocol.components.entry(name.to_string()).or_default();
            comp.template = format!(
                "(function(){{var w=window.__webui_templates||(window.__webui_templates={{}});w['{name}']={{h:'<div/>'}};}})();\n"
            );
            comp.css = format!(".{name}{{display:block}}");
        }

        // Render with hasItems=false — product-card should NOT be rendered
        let state = test_json!({ "hasItems": false });
        let mut writer = TestWriter::new();

        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let html = writer.get_content();

        // product-card template IS in the output — it's a known component
        // whose template must be available for client-side <if> activation.
        assert!(
            html.contains("w['product-card']"),
            "product-card template should be emitted even when unrendered: {html}"
        );

        // product-card CSS module IS in the output — reachable components need
        // their stylesheet definitions for client-side <if> activation.
        assert!(
            html.contains(r#"specifier="product-card""#),
            "reachable product-card CSS module should be emitted: {html}"
        );

        // app-shell and cart-panel SHOULD be in the output (they were rendered)
        assert!(
            html.contains("w['app-shell']"),
            "rendered app-shell template should be emitted: {html}"
        );
        assert!(
            html.contains("w['cart-panel']"),
            "rendered cart-panel template should be emitted: {html}"
        );
    }

    #[test]
    fn test_component_attr_name_aria() {
        // component_attr_name correctly maps ARIA attributes via the shared table
        assert_eq!(component_attr_name("aria-describedby"), "ariaDescribedBy");
        assert_eq!(component_attr_name("aria-labelledby"), "ariaLabelledBy");
        assert_eq!(
            component_attr_name("aria-activedescendant"),
            "ariaActiveDescendant"
        );
        assert_eq!(component_attr_name("aria-label"), "ariaLabel");
        assert_eq!(component_attr_name("aria-hidden"), "ariaHidden");
    }

    #[test]
    fn test_component_attr_name_html_global() {
        assert_eq!(component_attr_name("readonly"), "readOnly");
        assert_eq!(component_attr_name("tabindex"), "tabIndex");
        assert_eq!(component_attr_name("accesskey"), "accessKey");
        assert_eq!(component_attr_name("contenteditable"), "contentEditable");
        assert_eq!(component_attr_name("crossorigin"), "crossOrigin");
        assert_eq!(component_attr_name("inputmode"), "inputMode");
        assert_eq!(component_attr_name("maxlength"), "maxLength");
        assert_eq!(component_attr_name("minlength"), "minLength");
        assert_eq!(component_attr_name("novalidate"), "noValidate");
        assert_eq!(component_attr_name("formaction"), "formAction");
        assert_eq!(component_attr_name("ismap"), "isMap");
        assert_eq!(component_attr_name("usemap"), "useMap");
    }

    #[test]
    fn test_component_attr_name_strips_colon() {
        assert_eq!(component_attr_name(":readonly"), "readOnly");
        assert_eq!(component_attr_name(":aria-describedby"), "ariaDescribedBy");
        assert_eq!(component_attr_name(":data-title"), "dataTitle");
    }

    #[test]
    fn test_component_attr_name_regular() {
        assert_eq!(component_attr_name("data-title"), "dataTitle");
        assert_eq!(component_attr_name("key-hyphen"), "keyHyphen");
        assert_eq!(component_attr_name("simple"), "simple");
    }
}
