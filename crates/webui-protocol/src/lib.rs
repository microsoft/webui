// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI Protocol implementation.
//!
//! This crate defines the protocol used by the WebUI framework for cross-platform
//! representation of UI components and templates. Types are generated directly
//! from `proto/webui.proto` using prost for optimal runtime performance —
//! no conversion layer between domain types and protobuf types.

use prost::Message;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use thiserror::Error;

/// Plugin-specific protocol helpers for framework hydration metadata.
pub mod plugin;

/// Bundler-neutral state-projection manifest contract.
#[cfg(feature = "projection-manifest")]
pub mod projection_manifest;

/// Attribute-name ↔ property-name mapping for irregular HTML attributes.
pub mod attrs;

/// Generated protobuf types from `proto/webui.proto`.
pub mod proto {
    include!("gen_webui.rs");
}

// Re-export all generated types at the crate root.
pub use plugin::FastElementData;
pub use plugin::WebUIElementData;
pub use proto::*;

// Type aliases preserving the `WebUI` naming convention.
// prost generates `WebUi*` from the proto `WebUI*` messages.
pub type WebUIProtocol = WebUiProtocol;
pub type WebUIFragment = WebUiFragment;
pub type WebUIFragmentRaw = WebUiFragmentRaw;
pub type WebUIFragmentComponent = WebUiFragmentComponent;
pub type WebUIFragmentFor = WebUiFragmentFor;
pub type WebUIFragmentSignal = WebUiFragmentSignal;
pub type WebUIFragmentIf = WebUiFragmentIf;
pub type WebUIFragmentAttribute = WebUiFragmentAttribute;
pub type WebUIFragmentPlugin = WebUiFragmentPlugin;
pub type WebUIFragmentRoute = WebUiFragmentRoute;
pub type WebUIFragmentOutlet = WebUiFragmentOutlet;
pub type ComponentData = proto::ComponentData;
pub type ComponentStyleClosure = proto::ComponentStyleClosure;
pub type StreamingBoundaryList = proto::StreamingBoundaryList;

/// A mapping of unique fragment identifiers to their corresponding fragment lists.
pub type WebUIFragmentRecords = HashMap<String, FragmentList>;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Protocol validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

// ── Display implementations ─────────────────────────────────────────────

impl fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComparisonOperator::GreaterThan => write!(f, ">"),
            ComparisonOperator::LessThan => write!(f, "<"),
            ComparisonOperator::Equal => write!(f, "=="),
            ComparisonOperator::NotEqual => write!(f, "!="),
            ComparisonOperator::GreaterThanOrEqual => write!(f, ">="),
            ComparisonOperator::LessThanOrEqual => write!(f, "<="),
            ComparisonOperator::Unspecified => write!(f, "?"),
        }
    }
}

impl fmt::Display for LogicalOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicalOperator::And => write!(f, "&&"),
            LogicalOperator::Or => write!(f, "||"),
            LogicalOperator::Unspecified => write!(f, "?"),
        }
    }
}

impl fmt::Display for ConditionExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.expr {
            Some(condition_expr::Expr::Identifier(id)) => write!(f, "{}", id.value),
            Some(condition_expr::Expr::Predicate(pred)) => {
                let op = ComparisonOperator::try_from(pred.operator)
                    .unwrap_or(ComparisonOperator::Unspecified);
                write!(f, "{} {} {}", pred.left, op, pred.right)
            }
            Some(condition_expr::Expr::Not(not)) => match &not.condition {
                Some(inner) => write!(f, "!({})", inner),
                None => write!(f, "!(?)"),
            },
            Some(condition_expr::Expr::Compound(compound)) => {
                let op =
                    LogicalOperator::try_from(compound.op).unwrap_or(LogicalOperator::Unspecified);
                let left_str = compound
                    .left
                    .as_ref()
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let right_str = compound
                    .right
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "?".to_string());
                write!(f, "({} {} {})", left_str, op, right_str)
            }
            None => write!(f, "<empty>"),
        }
    }
}

// ── Convenience constructors ────────────────────────────────────────────

impl WebUiFragment {
    /// Create a raw (static content) fragment.
    pub fn raw(value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Raw(WebUiFragmentRaw {
                value: value.into(),
            })),
        }
    }

    /// Create a component fragment.
    pub fn component(fragment_id: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Component(
                WebUiFragmentComponent {
                    fragment_id: fragment_id.into(),
                },
            )),
        }
    }

    /// Create a for-loop fragment.
    pub fn for_loop(
        item: impl Into<String>,
        collection: impl Into<String>,
        fragment_id: impl Into<String>,
    ) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::ForLoop(WebUiFragmentFor {
                item: item.into(),
                collection: collection.into(),
                fragment_id: fragment_id.into(),
            })),
        }
    }

    /// Create a signal fragment.
    pub fn signal(value: impl Into<String>, raw: bool) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Signal(WebUiFragmentSignal {
                value: value.into(),
                raw,
            })),
        }
    }

    /// Create an if-condition fragment.
    pub fn if_cond(condition: ConditionExpr, fragment_id: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::IfCond(WebUiFragmentIf {
                condition: Some(condition),
                fragment_id: fragment_id.into(),
            })),
        }
    }

    /// Create a simple dynamic attribute fragment (value is a single signal name).
    pub fn attribute(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    value: value.into(),
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a template attribute fragment (mixed static + dynamic content).
    pub fn attribute_template(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    template: template.into(),
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a complex attribute fragment (:-prefixed).
    pub fn attribute_complex(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    value: value.into(),
                    complex: true,
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a boolean attribute fragment (?-prefixed) with a condition tree.
    pub fn attribute_boolean(name: impl Into<String>, condition_tree: ConditionExpr) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Attribute(
                WebUiFragmentAttribute {
                    name: name.into(),
                    condition_tree: Some(condition_tree),
                    ..Default::default()
                },
            )),
        }
    }

    /// Create a plugin data fragment with opaque bytes.
    /// The data is passed through to the handler plugin without interpretation.
    pub fn plugin(data: Vec<u8>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Plugin(WebUiFragmentPlugin {
                data,
            })),
        }
    }

    /// Create a route fragment linking a URL path template to a fragment.
    pub fn route(path: impl Into<String>, fragment_id: impl Into<String>) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Route(WebUiFragmentRoute {
                path: path.into(),
                fragment_id: fragment_id.into(),
                keep_alive: false,
                ..Default::default()
            })),
        }
    }

    /// Create a route fragment from a pre-built `WebUiFragmentRoute`.
    pub fn route_from(route: WebUiFragmentRoute) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Route(route)),
        }
    }

    /// Create an outlet fragment.
    pub fn outlet() -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Outlet(WebUiFragmentOutlet {})),
        }
    }
}

impl ConditionExpr {
    /// Create an identifier condition.
    pub fn identifier(value: impl Into<String>) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Identifier(IdentifierCondition {
                value: value.into(),
            })),
        }
    }

    /// Create a predicate condition.
    pub fn predicate(
        left: impl Into<String>,
        operator: ComparisonOperator,
        right: impl Into<String>,
    ) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Predicate(Predicate {
                left: left.into(),
                operator: operator as i32,
                right: right.into(),
            })),
        }
    }

    /// Create a negation condition.
    pub fn negated(inner: ConditionExpr) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Not(Box::new(NotCondition {
                condition: Some(Box::new(inner)),
            }))),
        }
    }

    /// Create a compound condition.
    pub fn compound(left: ConditionExpr, op: LogicalOperator, right: ConditionExpr) -> Self {
        Self {
            expr: Some(condition_expr::Expr::Compound(Box::new(
                CompoundCondition {
                    left: Some(Box::new(left)),
                    op: op as i32,
                    right: Some(Box::new(right)),
                },
            ))),
        }
    }
}

// ── Constructors ────────────────────────────────────────────────────────

type RouteChildrenByComponent<'a> = HashMap<&'a str, Vec<&'a [WebUiFragmentRoute]>>;

enum StyleClosureOp<'a> {
    Fragment {
        fragment_id: &'a str,
        component_owner: Option<&'a str>,
    },
    Component(&'a str),
    Route(&'a WebUiFragmentRoute),
}

impl WebUiProtocol {
    /// Return a component's build-resolved DOM strategy.
    #[must_use]
    pub fn effective_component_dom_strategy(&self, tag_name: &str) -> DomStrategy {
        self.components.get(tag_name).map_or_else(
            || self.dom_strategy(),
            ComponentData::effective_dom_strategy,
        )
    }

    /// Return the stored style resource for a component.
    ///
    /// Link protocols identify a resource by `css_href`; Style and Module
    /// protocols retain the compiled CSS bytes. An empty field means that the
    /// component has no paired style resource.
    #[must_use]
    pub fn component_style_resource(&self, tag_name: &str) -> Option<&str> {
        let component = self.components.get(tag_name)?;
        let resource = match self.css_strategy() {
            CssStrategy::Link => component.css_href.as_str(),
            CssStrategy::Style | CssStrategy::Module => component.css.as_str(),
        };
        (!resource.is_empty()).then_some(resource)
    }

    /// Return a precomputed style closure for `root`.
    #[must_use]
    pub fn style_closure(&self, root: &str) -> Option<&[String]> {
        self.style_closures
            .get(root)
            .map(|closure| closure.component_tags.as_slice())
    }

    /// Precompute ordered style-resource closures for entry fragments and every
    /// compiled component root.
    ///
    /// Traversal is iterative, follows source order, stops at effective Shadow
    /// children, and deduplicates resources at first discovery.
    pub fn populate_style_closures(&mut self, entry_fragments: &[&str]) {
        let route_children = self.route_children_by_component();
        let mut roots: Vec<String> = entry_fragments
            .iter()
            .map(|root| (*root).to_string())
            .collect();
        roots.extend(
            self.components
                .keys()
                .filter(|tag| self.fragments.contains_key(*tag))
                .cloned(),
        );
        roots.sort_unstable();
        roots.dedup();

        self.style_closures = roots
            .into_iter()
            .map(|root| {
                let closure = self.build_style_closure(&root, &route_children);
                (root, closure)
            })
            .collect();
    }

    fn route_definitions_by_component(&self) -> (Vec<&str>, RouteChildrenByComponent<'_>) {
        let mut fragment_ids: Vec<&str> = self.fragments.keys().map(String::as_str).collect();
        fragment_ids.sort_unstable();

        let mut route_owners = Vec::new();
        let mut route_definitions = RouteChildrenByComponent::new();
        let mut work = Vec::new();
        for fragment_id in fragment_ids {
            let Some(fragment_list) = self.fragments.get(fragment_id) else {
                continue;
            };
            for fragment in fragment_list.fragments.iter().rev() {
                if let Some(web_ui_fragment::Fragment::Route(route)) = fragment.fragment.as_ref() {
                    work.push(route);
                }
            }
            while let Some(route) = work.pop() {
                if !route.fragment_id.is_empty() && !route.children.is_empty() {
                    if let Some(routes) = route_definitions.get_mut(route.fragment_id.as_str()) {
                        routes.push(&route.children);
                    } else {
                        route_owners.push(route.fragment_id.as_str());
                        route_definitions.insert(&route.fragment_id, vec![&route.children]);
                    }
                }
                for child in route.children.iter().rev() {
                    work.push(child);
                }
            }
        }
        (route_owners, route_definitions)
    }

    fn route_children_by_component(&self) -> RouteChildrenByComponent<'_> {
        let (route_owners, route_definitions) = self.route_definitions_by_component();
        let mut route_children = RouteChildrenByComponent::new();
        for route_owner in route_owners {
            let Some(route_sets) = route_definitions.get(route_owner) else {
                continue;
            };
            self.map_route_children_to_outlets(route_owner, route_sets, &mut route_children);
        }
        route_children
    }

    fn map_route_children_to_outlets<'a>(
        &'a self,
        route_owner: &'a str,
        route_sets: &[&'a [WebUiFragmentRoute]],
        route_children: &mut RouteChildrenByComponent<'a>,
    ) {
        let mut work = vec![(route_owner, route_owner)];
        let mut visited_fragments = HashSet::new();
        let mut mapped_components = HashSet::new();

        while let Some((fragment_id, component_owner)) = work.pop() {
            if !visited_fragments.insert((fragment_id, component_owner)) {
                continue;
            }
            let Some(fragment_list) = self.fragments.get(fragment_id) else {
                continue;
            };
            for fragment in fragment_list.fragments.iter().rev() {
                match fragment.fragment.as_ref() {
                    Some(web_ui_fragment::Fragment::Component(component)) => {
                        work.push((&component.fragment_id, &component.fragment_id));
                    }
                    Some(web_ui_fragment::Fragment::ForLoop(for_loop)) => {
                        work.push((&for_loop.fragment_id, component_owner));
                    }
                    Some(web_ui_fragment::Fragment::IfCond(if_cond)) => {
                        work.push((&if_cond.fragment_id, component_owner));
                    }
                    Some(web_ui_fragment::Fragment::Attribute(attribute))
                        if !attribute.template.is_empty() =>
                    {
                        work.push((&attribute.template, component_owner));
                    }
                    Some(web_ui_fragment::Fragment::Outlet(_))
                        if mapped_components.insert(component_owner) =>
                    {
                        route_children
                            .entry(component_owner)
                            .or_default()
                            .extend(route_sets.iter().copied());
                    }
                    _ => {}
                }
            }
        }
    }

    fn build_style_closure<'a>(
        &'a self,
        root: &'a str,
        route_children: &RouteChildrenByComponent<'a>,
    ) -> ComponentStyleClosure {
        let mut component_tags = Vec::new();
        let mut seen_styles = HashSet::new();
        let mut visited_fragments = HashSet::new();
        let mut work = Vec::new();

        // A component-root closure owns its paired CSS regardless of whether
        // callers normally encounter that component as a Light or Shadow child.
        if self.component_style_resource(root).is_some() {
            seen_styles.insert(root);
            component_tags.push(root.to_string());
        }
        work.push(StyleClosureOp::Fragment {
            fragment_id: root,
            component_owner: self.components.contains_key(root).then_some(root),
        });

        while let Some(op) = work.pop() {
            match op {
                StyleClosureOp::Fragment {
                    fragment_id,
                    component_owner,
                } => {
                    if !visited_fragments.insert((fragment_id, component_owner)) {
                        continue;
                    }
                    let Some(fragment_list) = self.fragments.get(fragment_id) else {
                        continue;
                    };
                    for fragment in fragment_list.fragments.iter().rev() {
                        match fragment.fragment.as_ref() {
                            Some(web_ui_fragment::Fragment::Component(component)) => {
                                work.push(StyleClosureOp::Component(&component.fragment_id));
                            }
                            Some(web_ui_fragment::Fragment::ForLoop(for_loop)) => {
                                work.push(StyleClosureOp::Fragment {
                                    fragment_id: &for_loop.fragment_id,
                                    component_owner,
                                });
                            }
                            Some(web_ui_fragment::Fragment::IfCond(if_cond)) => {
                                work.push(StyleClosureOp::Fragment {
                                    fragment_id: &if_cond.fragment_id,
                                    component_owner,
                                });
                            }
                            Some(web_ui_fragment::Fragment::Attribute(attribute))
                                if !attribute.template.is_empty() =>
                            {
                                work.push(StyleClosureOp::Fragment {
                                    fragment_id: &attribute.template,
                                    component_owner,
                                });
                            }
                            Some(web_ui_fragment::Fragment::Route(route)) => {
                                work.push(StyleClosureOp::Route(route));
                            }
                            Some(web_ui_fragment::Fragment::Outlet(_)) => {
                                if let Some(route_sets) =
                                    component_owner.and_then(|owner| route_children.get(owner))
                                {
                                    for routes in route_sets.iter().rev() {
                                        for route in routes.iter().rev() {
                                            work.push(StyleClosureOp::Route(route));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                StyleClosureOp::Component(tag_name) => {
                    if self.effective_component_dom_strategy(tag_name) == DomStrategy::Shadow {
                        continue;
                    }
                    if self.component_style_resource(tag_name).is_some()
                        && seen_styles.insert(tag_name)
                    {
                        component_tags.push(tag_name.to_string());
                    }
                    work.push(StyleClosureOp::Fragment {
                        fragment_id: tag_name,
                        component_owner: Some(tag_name),
                    });
                }
                StyleClosureOp::Route(route) => {
                    // The route body owns its nested routes at its outlet. Stack
                    // fallbacks first so they follow that complete body tree.
                    if !route.error_component.is_empty() {
                        work.push(StyleClosureOp::Component(&route.error_component));
                    }
                    if !route.pending_component.is_empty() {
                        work.push(StyleClosureOp::Component(&route.pending_component));
                    }
                    if !route.fragment_id.is_empty() {
                        work.push(StyleClosureOp::Component(&route.fragment_id));
                    }
                }
            }
        }

        ComponentStyleClosure { component_tags }
    }

    /// Create a protocol from fragment records with no CSS tokens.
    pub fn new(fragments: WebUIFragmentRecords) -> Self {
        Self {
            fragments,
            tokens: Vec::new(),
            components: HashMap::new(),
            css_strategy: 0,
            dom_strategy: DomStrategy::Light as i32,
            initial_state_strategy: InitialStateStrategy::Full as i32,
            module_preloads: Vec::new(),
            streaming_boundaries: HashMap::new(),
            style_closures: HashMap::new(),
        }
    }

    /// Create a protocol from fragment records with CSS tokens.
    pub fn with_tokens(fragments: WebUIFragmentRecords, tokens: Vec<String>) -> Self {
        Self {
            fragments,
            tokens,
            components: HashMap::new(),
            css_strategy: 0,
            dom_strategy: DomStrategy::Light as i32,
            initial_state_strategy: InitialStateStrategy::Full as i32,
            module_preloads: Vec::new(),
            streaming_boundaries: HashMap::new(),
            style_closures: HashMap::new(),
        }
    }
}

// ── Serialization / deserialization / validation ────────────────────────

impl WebUiProtocol {
    /// Validate that all fragment references point to existing fragment IDs.
    fn validate_protocol(protocol: Self) -> Result<Self> {
        CssStrategy::try_from(protocol.css_strategy).map_err(|_| {
            ProtocolError::Validation(format!(
                "unknown CSS strategy value: {}",
                protocol.css_strategy
            ))
        })?;
        DomStrategy::try_from(protocol.dom_strategy).map_err(|_| {
            ProtocolError::Validation(format!(
                "unknown DOM strategy value: {}",
                protocol.dom_strategy
            ))
        })?;
        for (tag, component) in &protocol.components {
            DomStrategy::try_from(component.effective_dom_strategy).map_err(|_| {
                ProtocolError::Validation(format!(
                    "component `{tag}` has unknown effective DOM strategy value: {}",
                    component.effective_dom_strategy
                ))
            })?;
        }

        let fragments = &protocol.fragments;

        let invalid_ref = fragments.iter().find_map(|(_, fragment_list)| {
            fragment_list
                .fragments
                .iter()
                .find_map(|frag| match frag.fragment.as_ref() {
                    Some(web_ui_fragment::Fragment::Component(comp))
                        if !fragments.contains_key(&comp.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "Component references non-existent fragment ID: {}",
                            comp.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::ForLoop(fl))
                        if !fragments.contains_key(&fl.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "For loop references non-existent fragment ID: {}",
                            fl.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::IfCond(ic))
                        if !fragments.contains_key(&ic.fragment_id) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "If condition references non-existent fragment ID: {}",
                            ic.fragment_id
                        )))
                    }
                    Some(web_ui_fragment::Fragment::Attribute(attr))
                        if !attr.template.is_empty() && !fragments.contains_key(&attr.template) =>
                    {
                        Some(ProtocolError::Validation(format!(
                            "Attribute references non-existent template fragment ID: {}",
                            attr.template
                        )))
                    }
                    Some(web_ui_fragment::Fragment::Route(route)) => {
                        if !route.fragment_id.is_empty()
                            && !fragments.contains_key(&route.fragment_id)
                        {
                            return Some(ProtocolError::Validation(format!(
                                "Route references non-existent fragment ID: {}",
                                route.fragment_id
                            )));
                        }
                        None
                    }
                    _ => None,
                })
        });

        if let Some(err) = invalid_ref {
            return Err(err);
        }

        Ok(protocol)
    }

    /// Serialize protocol to pretty JSON (for debug/inspect output only).
    pub fn to_json_pretty(&self) -> std::result::Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize protocol to protobuf binary format.
    pub fn to_protobuf(&self) -> Result<Vec<u8>> {
        let len = self.encoded_len();
        let mut buf = Vec::with_capacity(len);
        self.encode(&mut buf)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf encode error: {e}")))?;
        Ok(buf)
    }

    /// Deserialize protocol from protobuf binary bytes with validation.
    pub fn from_protobuf(bytes: &[u8]) -> Result<Self> {
        let protocol = Self::decode(bytes)
            .map_err(|e| ProtocolError::Validation(format!("Protobuf decode error: {e}")))?;
        Self::validate_protocol(protocol)
    }

    /// Read and deserialize a protobuf file with validation.
    pub fn from_protobuf_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_protobuf(&bytes)
    }

    /// Write protocol to a protobuf file.
    pub fn to_protobuf_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.to_protobuf()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_protocol() -> WebUIProtocol {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, WebUI!\n"),
                    WebUIFragment::for_loop("person", "people", "for-1"),
                    WebUIFragment::signal("description", true),
                    WebUIFragment::if_cond(ConditionExpr::identifier("contact"), "if-1"),
                ],
            },
        );
        fragments.insert(
            "for-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("person.name", false)],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("contact-card")],
            },
        );
        fragments.insert(
            "contact-card".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                ],
            },
        );
        WebUIProtocol::new(fragments)
    }

    fn add_style_component(
        protocol: &mut WebUIProtocol,
        tag: &str,
        mode: DomStrategy,
        has_css: bool,
    ) {
        protocol
            .fragments
            .entry(tag.to_string())
            .or_insert_with(|| FragmentList {
                fragments: Vec::new(),
            });
        protocol.components.insert(
            tag.to_string(),
            ComponentData {
                css: has_css.then(|| format!(".{tag}{{}}")).unwrap_or_default(),
                effective_dom_strategy: mode as i32,
                ..Default::default()
            },
        );
    }

    #[test]
    fn style_closure_preserves_order_deduplicates_and_cuts_at_shadow() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("light-a"),
                    WebUIFragment::component("light-b"),
                    WebUIFragment::component("no-css"),
                    WebUIFragment::component("shadow-cut"),
                    WebUIFragment::component("after-cut"),
                ],
            },
        );
        fragments.insert(
            "light-a".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("shared-light")],
            },
        );
        fragments.insert(
            "light-b".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("shared-light")],
            },
        );
        fragments.insert(
            "no-css".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("styled-descendant")],
            },
        );
        fragments.insert(
            "shadow-cut".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("shadow-descendant")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.set_dom_strategy(DomStrategy::Light);
        for tag in [
            "light-a",
            "light-b",
            "shared-light",
            "styled-descendant",
            "shadow-descendant",
            "after-cut",
        ] {
            add_style_component(&mut protocol, tag, DomStrategy::Light, true);
        }
        add_style_component(&mut protocol, "no-css", DomStrategy::Light, false);
        add_style_component(&mut protocol, "shadow-cut", DomStrategy::Shadow, true);

        protocol.populate_style_closures(&["index.html"]);

        assert_eq!(
            protocol.style_closure("index.html").expect("entry closure"),
            [
                "light-a",
                "shared-light",
                "light-b",
                "styled-descendant",
                "after-cut",
            ]
        );
        assert_eq!(
            protocol
                .style_closure("shadow-cut")
                .expect("Shadow closure"),
            ["shadow-cut", "shadow-descendant"]
        );
    }

    #[test]
    fn style_closure_follows_dynamic_and_route_dependencies_in_order() {
        let nested_route = WebUiFragmentRoute {
            fragment_id: "nested-route".to_string(),
            ..Default::default()
        };
        let route = WebUiFragmentRoute {
            fragment_id: "route-body".to_string(),
            children: vec![nested_route],
            pending_component: "route-pending".to_string(),
            error_component: "route-error".to_string(),
            ..Default::default()
        };
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::for_loop("item", "items", "for-body"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("show"), "if-body"),
                    WebUIFragment::attribute_template("title", "attribute-body"),
                    WebUIFragment::route_from(route),
                ],
            },
        );
        for (fragment, component) in [
            ("for-body", "for-card"),
            ("if-body", "if-card"),
            ("attribute-body", "attribute-card"),
        ] {
            fragments.insert(
                fragment.to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::component(component)],
                },
            );
        }
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.set_dom_strategy(DomStrategy::Light);
        for tag in [
            "for-card",
            "if-card",
            "attribute-card",
            "route-body",
            "nested-route",
            "route-pending",
            "route-error",
        ] {
            add_style_component(&mut protocol, tag, DomStrategy::Light, true);
        }
        protocol.fragments.insert(
            "route-body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::outlet()],
            },
        );

        protocol.populate_style_closures(&["index.html"]);

        assert_eq!(
            protocol.style_closure("index.html").expect("entry closure"),
            [
                "for-card",
                "if-card",
                "attribute-card",
                "route-body",
                "nested-route",
                "route-pending",
                "route-error",
            ]
        );
    }

    #[test]
    fn style_closure_places_nested_routes_in_shadow_route_outlet_tree() {
        let child_route = WebUiFragmentRoute {
            fragment_id: "dashboard-page".to_string(),
            pending_component: "dashboard-pending".to_string(),
            error_component: "dashboard-error".to_string(),
            ..Default::default()
        };
        let route = WebUiFragmentRoute {
            fragment_id: "app-shell".to_string(),
            children: vec![child_route],
            pending_component: "shell-pending".to_string(),
            error_component: "shell-error".to_string(),
            ..Default::default()
        };
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(route)],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("shell-header"),
                    WebUIFragment::component("route-layout"),
                    WebUIFragment::component("shell-footer"),
                ],
            },
        );
        fragments.insert(
            "route-layout".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "dashboard-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("contact-card")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.set_dom_strategy(DomStrategy::Light);
        for tag in [
            "shell-header",
            "route-layout",
            "shell-footer",
            "dashboard-page",
            "contact-card",
            "dashboard-pending",
            "dashboard-error",
            "shell-pending",
            "shell-error",
        ] {
            add_style_component(&mut protocol, tag, DomStrategy::Light, true);
        }
        add_style_component(&mut protocol, "app-shell", DomStrategy::Shadow, true);

        protocol.populate_style_closures(&["index.html"]);

        assert_eq!(
            protocol.style_closure("index.html").expect("entry closure"),
            ["shell-pending", "shell-error"]
        );
        assert_eq!(
            protocol
                .style_closure("app-shell")
                .expect("Shadow shell closure"),
            [
                "app-shell",
                "shell-header",
                "route-layout",
                "dashboard-page",
                "contact-card",
                "dashboard-pending",
                "dashboard-error",
                "shell-footer",
            ]
        );
    }

    #[test]
    fn style_closure_routes_through_nested_shadow_outlet_component() {
        let route = WebUiFragmentRoute {
            fragment_id: "light-shell".to_string(),
            children: vec![WebUiFragmentRoute {
                fragment_id: "dashboard-page".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(route)],
            },
        );
        fragments.insert(
            "light-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("shadow-layout")],
            },
        );
        fragments.insert(
            "shadow-layout".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "dashboard-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("contact-card")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.set_dom_strategy(DomStrategy::Light);
        for tag in ["light-shell", "dashboard-page", "contact-card"] {
            add_style_component(&mut protocol, tag, DomStrategy::Light, true);
        }
        add_style_component(&mut protocol, "shadow-layout", DomStrategy::Shadow, true);

        protocol.populate_style_closures(&["index.html"]);

        assert_eq!(
            protocol.style_closure("index.html").expect("entry closure"),
            ["light-shell"]
        );
        assert_eq!(
            protocol
                .style_closure("shadow-layout")
                .expect("nested Shadow closure"),
            ["shadow-layout", "dashboard-page", "contact-card"]
        );
    }

    #[test]
    fn style_closure_guards_same_tag_cycles_and_is_deterministic() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("cycle-card")],
            },
        );
        fragments.insert(
            "cycle-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("cycle-card")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.set_dom_strategy(DomStrategy::Light);
        add_style_component(&mut protocol, "cycle-card", DomStrategy::Light, true);

        protocol.populate_style_closures(&["index.html"]);
        let first = protocol.style_closures.clone();
        protocol.populate_style_closures(&["index.html"]);

        assert_eq!(
            protocol.style_closure("index.html").expect("entry closure"),
            ["cycle-card"]
        );
        assert_eq!(
            protocol
                .style_closure("cycle-card")
                .expect("component closure"),
            ["cycle-card"]
        );
        assert_eq!(protocol.style_closures, first);
    }

    #[test]
    fn style_closure_roundtrips() {
        let mut protocol = sample_protocol();
        protocol.set_css_strategy(CssStrategy::Link);
        protocol.set_dom_strategy(DomStrategy::Light);
        protocol.components.insert(
            "contact-card".to_string(),
            ComponentData {
                css_href: "/contact-card.css".to_string(),
                ..Default::default()
            },
        );
        protocol.populate_style_closures(&["index.html"]);

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(
            decoded.style_closure("index.html").expect("entry closure"),
            ["contact-card"]
        );
    }

    #[test]
    fn current_json_requires_style_and_component_dom_metadata() {
        let mut protocol = sample_protocol();
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                effective_dom_strategy: DomStrategy::Light as i32,
                ..Default::default()
            },
        );
        protocol.populate_style_closures(&["index.html"]);
        let current = serde_json::to_value(&protocol).expect("JSON encode failed");

        let mut missing_closures = current.clone();
        missing_closures
            .as_object_mut()
            .expect("protocol JSON object")
            .remove("style_closures");
        assert!(serde_json::from_value::<WebUIProtocol>(missing_closures).is_err());

        let mut missing_dom = current;
        missing_dom["components"]["my-card"]
            .as_object_mut()
            .expect("component JSON object")
            .remove("effective_dom_strategy");
        assert!(serde_json::from_value::<WebUIProtocol>(missing_dom).is_err());
    }

    #[test]
    fn protobuf_rejects_unknown_dom_strategy_values() {
        let mut protocol = sample_protocol();
        protocol.dom_strategy = 99;
        let bytes = protocol.to_protobuf().expect("encode failed");
        let error = WebUIProtocol::from_protobuf(&bytes).expect_err("invalid strategy must fail");
        assert!(error.to_string().contains("unknown DOM strategy value: 99"));

        protocol.dom_strategy = DomStrategy::Light as i32;
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                effective_dom_strategy: 99,
                ..Default::default()
            },
        );
        let bytes = protocol.to_protobuf().expect("encode failed");
        let error = WebUIProtocol::from_protobuf(&bytes).expect_err("invalid mode must fail");
        assert!(error
            .to_string()
            .contains("component `my-card` has unknown effective DOM strategy value: 99"));
    }

    #[test]
    fn component_style_resource_uses_the_active_strategy_field() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                css: ".card{}".to_string(),
                css_href: "/my-card.css".to_string(),
                ..Default::default()
            },
        );

        protocol.set_css_strategy(CssStrategy::Link);
        assert_eq!(
            protocol.component_style_resource("my-card"),
            Some("/my-card.css")
        );
        protocol.set_css_strategy(CssStrategy::Style);
        assert_eq!(
            protocol.component_style_resource("my-card"),
            Some(".card{}")
        );
        protocol.set_css_strategy(CssStrategy::Module);
        assert_eq!(
            protocol.component_style_resource("my-card"),
            Some(".card{}")
        );
        assert_eq!(protocol.component_style_resource("missing-card"), None);
    }

    #[test]
    fn declared_link_strategy_does_not_infer_module_from_css_bytes() {
        let mut protocol = WebUIProtocol::default();
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                css: ".card{}".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(protocol.css_strategy(), CssStrategy::Link);
        assert_eq!(protocol.component_style_resource("my-card"), None);
    }

    #[test]
    fn test_protobuf_roundtrip() {
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_module_preloads_roundtrip_preserves_order() {
        // Order is load-bearing: preloads are issued in document order over one
        // connection, and a measured 125 ms swing separates largest-first from
        // smallest-first. A reordering encoder would silently erase the win.
        let mut protocol = sample_protocol();
        protocol.module_preloads = vec![
            "/chunk-big.js".to_string(),
            "/chunk-mid.js".to_string(),
            "/chunk-small.js".to_string(),
        ];

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        assert_eq!(decoded.module_preloads, protocol.module_preloads);
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_no_module_preloads_is_absent_from_the_wire() {
        // Builds without hints must not pay a byte for the field.
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        assert!(decoded.module_preloads.is_empty());
        assert!(
            !bytes.windows(2).any(|w| w == [0x3a, 0x00]),
            "an empty repeated field must not be encoded"
        );
    }

    #[test]
    fn test_protobuf_streaming_boundary_names_roundtrip_in_declaration_order() {
        let mut protocol = sample_protocol();
        protocol.streaming_boundaries.insert(
            "main".to_string(),
            StreamingBoundaryList {
                names: vec![
                    "weather shell".to_string(),
                    "composer/ready".to_string(),
                    "feed:batch".to_string(),
                ],
            },
        );

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        assert_eq!(
            decoded.streaming_boundaries["main"].names,
            protocol.streaming_boundaries["main"].names
        );
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_all_fragment_types() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("text"),
                    WebUIFragment::component("comp"),
                    WebUIFragment::for_loop("x", "xs", "loop"),
                    WebUIFragment::signal("sig", true),
                    WebUIFragment::if_cond(
                        ConditionExpr::predicate("a", ComparisonOperator::GreaterThan, "1"),
                        "cond",
                    ),
                ],
            },
        );
        fragments.insert(
            "comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("c")],
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("l")],
            },
        );
        fragments.insert(
            "cond".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("i")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_all_comparison_operators() {
        let ops = [
            ComparisonOperator::GreaterThan,
            ComparisonOperator::LessThan,
            ComparisonOperator::Equal,
            ComparisonOperator::NotEqual,
            ComparisonOperator::GreaterThanOrEqual,
            ComparisonOperator::LessThanOrEqual,
        ];
        for op in &ops {
            let mut fragments = HashMap::new();
            fragments.insert(
                "main".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::if_cond(
                        ConditionExpr::predicate("a", *op, "b"),
                        "then",
                    )],
                },
            );
            fragments.insert(
                "then".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("ok")],
                },
            );
            let p = WebUIProtocol::new(fragments);
            let bytes = p.to_protobuf().unwrap();
            let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
            assert_eq!(p, decoded);
        }
    }

    #[test]
    fn test_protobuf_nested_conditions() {
        let nested = ConditionExpr::compound(
            ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "admin"),
            LogicalOperator::And,
            ConditionExpr::negated(ConditionExpr::predicate(
                "user.disabled",
                ComparisonOperator::Equal,
                "true",
            )),
        );

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(nested, "then")],
            },
        );
        fragments.insert(
            "then".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("ok")],
            },
        );
        let p = WebUIProtocol::new(fragments);
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn test_protobuf_compound_or_condition() {
        let compound = ConditionExpr::compound(
            ConditionExpr::identifier("isAdmin"),
            LogicalOperator::Or,
            ConditionExpr::identifier("isEditor"),
        );

        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(compound, "body")],
            },
        );
        fragments.insert(
            "body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
            },
        );
        let p = WebUIProtocol::new(fragments);
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn test_protobuf_invalid_bytes() {
        let result = WebUIProtocol::from_protobuf(&[0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn test_protobuf_empty_bytes() {
        let result = WebUIProtocol::from_protobuf(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().fragments.is_empty());
    }

    #[test]
    fn test_protobuf_file_roundtrip() {
        let protocol = sample_protocol();
        let dir = std::env::temp_dir().join("webui-proto-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.bin");

        protocol.to_protobuf_file(&path).unwrap();
        let decoded = WebUIProtocol::from_protobuf_file(&path).unwrap();
        assert_eq!(protocol, decoded);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_protobuf_validation_catches_missing_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("does-not-exist")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_protobuf_validation_catches_missing_for_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "missing-for")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-for"));
        }
    }

    #[test]
    fn test_protobuf_validation_catches_missing_if_reference() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("flag"),
                    "missing-if",
                )],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let buf = protocol.to_protobuf().unwrap();

        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-if"));
        }
    }

    #[test]
    fn test_protobuf_signal_default_raw_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("name", false)],
            },
        );
        let p = WebUIProtocol::new(fragments);
        let bytes = p.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let frag = &decoded.fragments["main"].fragments[0];
        match frag.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Signal(s)) => assert!(!s.raw),
            _ => panic!("expected signal"),
        }
    }

    #[test]
    fn test_protobuf_pre_allocated_buffer() {
        let protocol = sample_protocol();
        let bytes = protocol.to_protobuf().unwrap();
        assert_eq!(bytes.len(), protocol.encoded_len());
    }

    #[test]
    fn test_protocol_new_has_empty_tokens() {
        let protocol = WebUIProtocol::new(HashMap::new());
        assert!(protocol.tokens.is_empty());
        assert!(protocol.fragments.is_empty());
        assert_eq!(protocol.dom_strategy(), DomStrategy::Light);
        assert_eq!(
            protocol.initial_state_strategy,
            InitialStateStrategy::Full as i32
        );
    }

    #[test]
    fn test_absent_wire_dom_strategy_decodes_as_light() {
        let protocol =
            WebUIProtocol::from_protobuf(&[]).expect("empty protocol payload should decode");
        assert_eq!(protocol.dom_strategy(), DomStrategy::Light);
    }

    #[test]
    fn test_projection_metadata_roundtrips() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                hydration_keys: vec!["name".to_string()],
                hydration_mode: StateProjectionMode::Keys as i32,
                navigation_mode: Some(StateProjectionMode::All as i32),
                ..Default::default()
            },
        );

        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(
            decoded.initial_state_strategy,
            InitialStateStrategy::Components as i32
        );
        let component = &decoded.components["my-card"];
        assert_eq!(component.hydration_mode, StateProjectionMode::Keys as i32);
        assert_eq!(component.hydration_keys, ["name"]);
        assert_eq!(
            component.navigation_mode,
            Some(StateProjectionMode::All as i32)
        );
        assert!(component.navigation_keys.is_empty());
    }

    #[test]
    fn test_component_effective_dom_strategy_roundtrips() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.set_dom_strategy(DomStrategy::Light);
        protocol.components.insert(
            "my-card".to_string(),
            ComponentData {
                effective_dom_strategy: DomStrategy::Shadow as i32,
                ..Default::default()
            },
        );

        let bytes = protocol.to_protobuf().unwrap();
        let decoded = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert_eq!(
            decoded.effective_component_dom_strategy("my-card"),
            DomStrategy::Shadow
        );
    }

    #[test]
    fn test_protocol_with_tokens() {
        let tokens = vec!["color-primary".to_string(), "spacing-m".to_string()];
        let protocol = WebUIProtocol::with_tokens(HashMap::new(), tokens.clone());
        assert_eq!(protocol.tokens, tokens);
        assert_eq!(protocol.dom_strategy(), DomStrategy::Light);
    }

    #[test]
    fn test_protobuf_route_fragment_roundtrip() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/profile/:id", "profile-page")],
            },
        );
        fragments.insert(
            "profile-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>Profile</h1>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(protocol, decoded);

        let frag = &decoded.fragments["main"].fragments[0];
        match frag.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile/:id");
                assert_eq!(r.fragment_id, "profile-page");
            }
            _ => panic!("expected route fragment"),
        }
    }

    #[test]
    fn test_protobuf_route_fragment_all_fields() {
        let mut fragments = HashMap::new();
        let route_frag = WebUiFragment {
            fragment: Some(web_ui_fragment::Fragment::Route(WebUiFragmentRoute {
                path: "/users/:id/posts/:postId".to_string(),
                fragment_id: "user-posts".to_string(),
                exact: true,
                children: Vec::new(),
                allowed_query: "action,to,subject".to_string(),
                keep_alive: false,
                cache_tags: vec!["user:{id}".to_string(), "posts".to_string()],
                invalidates: vec!["posts".to_string(), "counts".to_string()],
                pending_component: "loading-skeleton".to_string(),
                error_component: "error-page".to_string(),
            })),
        };
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![route_frag],
            },
        );
        fragments.insert(
            "user-posts".into(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("posts")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_route_validation_missing_fragment() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/test", "missing-fragment")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let buf = protocol.to_protobuf().expect("encode failed");
        let result = WebUIProtocol::from_protobuf(&buf);
        assert!(result.is_err());
        if let Err(ProtocolError::Validation(msg)) = result {
            assert!(msg.contains("missing-fragment"));
        }
    }

    #[test]
    fn test_protobuf_route_no_fragment_id_roundtrip() {
        let mut fragments = HashMap::new();
        let route_frag = WebUiFragment {
            fragment: Some(web_ui_fragment::Fragment::Route(WebUiFragmentRoute {
                path: "/old-path".to_string(),
                keep_alive: false,
                ..Default::default()
            })),
        };
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![route_frag],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_backward_compat_no_routes() {
        // Protocol without any fragments should decode successfully
        let protocol = WebUIProtocol::new(HashMap::new());
        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");
        assert!(decoded.fragments.is_empty());
    }

    #[test]
    fn test_protobuf_roundtrip_with_tokens() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Hello")],
            },
        );
        let tokens = vec!["border-radius-m".to_string(), "color-primary".to_string()];
        let protocol = WebUIProtocol::with_tokens(fragments, tokens.clone());

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        assert_eq!(decoded.tokens, tokens);
        assert!(decoded.fragments.contains_key("index.html"));
    }
}
