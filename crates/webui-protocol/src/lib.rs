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
pub type WebUIFragmentBoundary = WebUiFragmentBoundary;
pub type ComponentData = proto::ComponentData;

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

    /// Create the start marker of an inline streaming boundary tape.
    ///
    /// The body fragments follow this marker in the same record and are closed
    /// by [`Self::boundary_end`] carrying the same `declaration_id`.
    pub fn boundary(
        declaration_id: u32,
        owner_fragment_id: impl Into<String>,
        name: impl Into<String>,
        key: Option<String>,
    ) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Boundary(WebUiFragmentBoundary {
                declaration_id,
                owner_fragment_id: owner_fragment_id.into(),
                name: name.into(),
                key,
                may_repeat: false,
                phase: BoundaryPhase::Start as i32,
            })),
        }
    }

    /// Create the end marker that closes an inline streaming boundary tape.
    #[must_use]
    pub fn boundary_end(declaration_id: u32) -> Self {
        Self {
            fragment: Some(web_ui_fragment::Fragment::Boundary(WebUiFragmentBoundary {
                declaration_id,
                phase: BoundaryPhase::End as i32,
                ..Default::default()
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

impl WebUiProtocol {
    /// Create a protocol from fragment records with no CSS tokens.
    pub fn new(fragments: WebUIFragmentRecords) -> Self {
        Self {
            fragments,
            tokens: Vec::new(),
            components: HashMap::new(),
            css_strategy: 0,
            dom_strategy: 0,
            initial_state_strategy: InitialStateStrategy::Full as i32,
            module_preloads: Vec::new(),
            component_render_css: String::new(),
            component_asset_style_preloads: Vec::new(),
        }
    }

    /// Create a protocol from fragment records with CSS tokens.
    pub fn with_tokens(fragments: WebUIFragmentRecords, tokens: Vec<String>) -> Self {
        Self {
            fragments,
            tokens,
            components: HashMap::new(),
            css_strategy: 0,
            dom_strategy: 0,
            initial_state_strategy: InitialStateStrategy::Full as i32,
            module_preloads: Vec::new(),
            component_render_css: String::new(),
            component_asset_style_preloads: Vec::new(),
        }
    }
}

// ── Serialization / deserialization / validation ────────────────────────

impl WebUiProtocol {
    /// Validate that all fragment references point to existing fragment IDs.
    fn validate_protocol(protocol: Self) -> Result<Self> {
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
                    Some(web_ui_fragment::Fragment::Boundary(boundary))
                        if boundary.phase() == BoundaryPhase::Start
                            && !fragments.contains_key(&boundary.owner_fragment_id) =>
                    {
                        Some(Self::missing_boundary_owner_error(boundary))
                    }
                    Some(web_ui_fragment::Fragment::Route(route)) => {
                        Self::validate_route_references(route, fragments)
                    }
                    _ => None,
                })
        });

        if let Some(err) = invalid_ref {
            return Err(err);
        }

        let mut declaration_ids = HashSet::new();
        let mut owner_names = HashSet::new();
        for fragment_list in fragments.values() {
            let mut open: Option<u32> = None;
            for fragment in &fragment_list.fragments {
                let Some(web_ui_fragment::Fragment::Boundary(boundary)) =
                    fragment.fragment.as_ref()
                else {
                    continue;
                };
                if boundary.phase() == BoundaryPhase::End {
                    match open.take() {
                        Some(declaration_id) if declaration_id == boundary.declaration_id => {}
                        _ => return Err(Self::unbalanced_boundary_tape_error(boundary)),
                    }
                    continue;
                }
                if open.is_some() {
                    return Err(Self::unbalanced_boundary_tape_error(boundary));
                }
                open = Some(boundary.declaration_id);
                if boundary.name.trim().is_empty() {
                    return Err(Self::empty_boundary_name_error(boundary));
                }
                if boundary
                    .key
                    .as_ref()
                    .is_some_and(|key| key.trim().is_empty())
                {
                    return Err(Self::empty_boundary_key_error(boundary));
                }
                if !declaration_ids.insert(boundary.declaration_id) {
                    return Err(Self::duplicate_boundary_id_error(boundary));
                }
                if !owner_names.insert((&boundary.owner_fragment_id, &boundary.name)) {
                    return Err(Self::duplicate_boundary_name_error(boundary));
                }
            }
            if let Some(declaration_id) = open {
                return Err(Self::unterminated_boundary_tape_error(declaration_id));
            }
        }

        Ok(protocol)
    }

    fn validate_route_references(
        root: &WebUiFragmentRoute,
        fragments: &WebUIFragmentRecords,
    ) -> Option<ProtocolError> {
        let mut pending = vec![root];
        while let Some(route) = pending.pop() {
            for (kind, fragment_id) in [
                ("component", route.fragment_id.as_str()),
                ("content", route.content_fragment_id.as_str()),
            ] {
                if !fragment_id.is_empty() && !fragments.contains_key(fragment_id) {
                    return Some(Self::missing_route_reference_error(kind, fragment_id));
                }
            }
            pending.extend(route.children.iter());
        }
        None
    }

    #[cold]
    #[inline(never)]
    fn unbalanced_boundary_tape_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Boundary declaration {} has an unbalanced inline tape marker",
            boundary.declaration_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn unterminated_boundary_tape_error(declaration_id: u32) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Boundary declaration {declaration_id} is missing its end marker"
        ))
    }

    #[cold]
    #[inline(never)]
    fn missing_boundary_owner_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Boundary declaration {} references non-existent owner fragment ID: {}",
            boundary.declaration_id, boundary.owner_fragment_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn empty_boundary_name_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Boundary declaration {} has an empty authored name",
            boundary.declaration_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn empty_boundary_key_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Boundary declaration {} has an empty key expression",
            boundary.declaration_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn duplicate_boundary_id_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Duplicate boundary declaration ID: {}",
            boundary.declaration_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn duplicate_boundary_name_error(boundary: &WebUiFragmentBoundary) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Duplicate boundary name '{}' in owner fragment '{}'",
            boundary.name, boundary.owner_fragment_id
        ))
    }

    #[cold]
    #[inline(never)]
    fn missing_route_reference_error(kind: &str, fragment_id: &str) -> ProtocolError {
        ProtocolError::Validation(format!(
            "Route {kind} references non-existent fragment ID: {fragment_id}"
        ))
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
                contains_boundary: false,
            },
        );
        fragments.insert(
            "for-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("person.name", false)],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("contact-card")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "contact-card".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                ],
                contains_boundary: false,
            },
        );
        WebUIProtocol::new(fragments)
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
    fn test_component_asset_style_preloads_roundtrip_preserves_generated_hrefs() {
        let mut protocol = sample_protocol();
        protocol.component_asset_style_preloads = vec![ComponentAssetStylePreload {
            root: "lazy-panel".to_string(),
            style_hrefs: vec![
                "/assets/lazy-panel-e5f6a7b8.css".to_string(),
                "/assets/shared-detail-10203040.css".to_string(),
            ],
        }];

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        assert_eq!(
            decoded.component_asset_style_preloads,
            protocol.component_asset_style_preloads
        );
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protobuf_boundary_fragment_roundtrip() {
        let mut protocol = sample_protocol();
        let main = protocol
            .fragments
            .get_mut("index.html")
            .expect("sample entry exists");
        main.fragments.push(WebUIFragment::boundary(
            7,
            "index.html",
            "weather shell",
            Some("forecast.id".to_string()),
        ));
        main.fragments
            .push(WebUIFragment::raw("<weather-panel></weather-panel>"));
        main.fragments.push(WebUIFragment::boundary_end(7));
        main.contains_boundary = true;

        let bytes = protocol.to_protobuf().expect("encode failed");
        let decoded = WebUIProtocol::from_protobuf(&bytes).expect("decode failed");

        let markers: Vec<&WebUiFragmentBoundary> = decoded.fragments["index.html"]
            .fragments
            .iter()
            .filter_map(|fragment| match fragment.fragment.as_ref() {
                Some(web_ui_fragment::Fragment::Boundary(boundary)) => Some(boundary),
                _ => None,
            })
            .collect();
        assert_eq!(markers.len(), 2, "the tape survives roundtrip as a pair");
        let boundary = markers[0];
        assert_eq!(boundary.phase(), BoundaryPhase::Start);
        assert_eq!(boundary.declaration_id, 7);
        assert_eq!(boundary.owner_fragment_id, "index.html");
        assert_eq!(boundary.name, "weather shell");
        assert_eq!(boundary.key.as_deref(), Some("forecast.id"));
        assert_eq!(markers[1].phase(), BoundaryPhase::End);
        assert_eq!(markers[1].declaration_id, 7);
        assert!(decoded.fragments["index.html"].contains_boundary);
        assert_eq!(protocol, decoded);
    }

    #[test]
    fn test_protocol_rejects_unbalanced_boundary_tape() {
        let tape_cases: [(&str, Vec<WebUIFragment>); 3] = [
            (
                "start without end",
                vec![WebUIFragment::boundary(0, "index.html", "ready", None)],
            ),
            ("end without start", vec![WebUIFragment::boundary_end(0)]),
            (
                "nested start",
                vec![
                    WebUIFragment::boundary(0, "index.html", "outer", None),
                    WebUIFragment::boundary(1, "index.html", "inner", None),
                    WebUIFragment::boundary_end(1),
                    WebUIFragment::boundary_end(0),
                ],
            ),
        ];
        for (label, fragments) in tape_cases {
            let protocol = WebUIProtocol::new(HashMap::from([(
                "index.html".to_string(),
                FragmentList {
                    fragments,
                    contains_boundary: true,
                },
            )]));
            let bytes = protocol.to_protobuf().expect("encode failed");
            let error =
                WebUIProtocol::from_protobuf(&bytes).expect_err("unbalanced tape must be rejected");
            assert!(
                error.to_string().contains("Boundary"),
                "{label}: unexpected error {error}"
            );
        }
    }

    #[test]
    fn test_protocol_has_no_fixed_streaming_boundary_table() {
        let json = sample_protocol().to_json_pretty().expect("serialize JSON");
        assert!(!json.contains("streaming_boundaries"));
        assert!(!json.contains("StreamingBoundaryList"));
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
                    WebUIFragment::boundary(0, "main", "ready", None),
                    WebUIFragment::boundary_end(0),
                ],
                contains_boundary: true,
            },
        );
        fragments.insert(
            "comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("c")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("l")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "cond".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("i")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "boundary".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("b")],
                contains_boundary: false,
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
                    contains_boundary: false,
                },
            );
            fragments.insert(
                "then".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("ok")],
                    contains_boundary: false,
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
                contains_boundary: false,
            },
        );
        fragments.insert(
            "then".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("ok")],
                contains_boundary: false,
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
                contains_boundary: false,
            },
        );
        fragments.insert(
            "body".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
                contains_boundary: false,
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
                contains_boundary: false,
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
                contains_boundary: false,
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
                contains_boundary: false,
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
                contains_boundary: false,
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
        assert_eq!(
            protocol.initial_state_strategy,
            InitialStateStrategy::Full as i32
        );
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
    fn test_protocol_with_tokens() {
        let tokens = vec!["color-primary".to_string(), "spacing-m".to_string()];
        let protocol = WebUIProtocol::with_tokens(HashMap::new(), tokens.clone());
        assert_eq!(protocol.tokens, tokens);
    }

    #[test]
    fn test_protobuf_route_fragment_roundtrip() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/profile/:id", "profile-page")],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "profile-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>Profile</h1>")],
                contains_boundary: false,
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
                content_fragment_id: String::new(),
            })),
        };
        fragments.insert(
            "main".to_string(),
            FragmentList {
                fragments: vec![route_frag],
                contains_boundary: false,
            },
        );
        fragments.insert(
            "user-posts".into(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("posts")],
                contains_boundary: false,
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
                contains_boundary: false,
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
                contains_boundary: false,
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
                contains_boundary: false,
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
