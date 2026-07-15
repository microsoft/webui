// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Route component inventory management for incremental route rendering.
//!
//! These helpers walk the normal render fragment graph. The request-aware path is
//! route-aware but state-agnostic: it follows the active route chain for the
//! current request path, while conservatively traversing `if`, `for`, and
//! attribute-template edges without evaluating runtime state.

use crate::{route_matcher, route_renderer, HandlerError, StateSelection};
use route_matcher::CompiledRouteCache;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::{value::RawValue, Map, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::RwLock;
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragmentRoute, WebUIProtocol};

// ── Protocol Index ──────────────────────────────────────────────────────

/// A decoded protocol with reusable deterministic indices.
///
/// Construct this once when a server loads `protocol.bin`, then reuse it for
/// full renders, partial navigation, component-template requests, and token
/// queries. Full renders borrow the immutable protocol without locking.
/// Partial and action requests use request-local route caches. Parsed template
/// metadata is populated lazily behind a read-write lock whose scope is limited
/// to individual metadata lookups.
///
/// The request-specific route cache is cleared before each partial/action
/// operation. Nested relative routes can resolve against request values, so
/// retaining those cache entries across requests would allow unbounded growth.
pub struct PreparedProtocol {
    protocol: WebUIProtocol,
    component_index: HashMap<String, u32>,
    template_metadata_cache: RwLock<HashMap<String, Value>>,
}

impl PreparedProtocol {
    /// Decode protobuf bytes and build the reusable protocol index.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when `bytes` is not a valid WebUI protobuf.
    pub fn from_protobuf(bytes: &[u8]) -> std::result::Result<Self, webui_protocol::ProtocolError> {
        WebUIProtocol::from_protobuf(bytes).map(Self::new)
    }

    /// Prepare an already decoded protocol for repeated use.
    #[must_use]
    pub fn new(protocol: WebUIProtocol) -> Self {
        let component_index = build_component_index(&protocol);
        Self {
            protocol,
            component_index,
            template_metadata_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Borrow the decoded protocol for full rendering.
    #[must_use]
    pub fn protocol(&self) -> &WebUIProtocol {
        &self.protocol
    }

    /// Borrow the build-time CSS token list.
    #[must_use]
    pub fn tokens(&self) -> &[String] {
        &self.protocol.tokens
    }

    fn request_index<'a>(
        &'a self,
        route_cache: &'a mut CompiledRouteCache,
    ) -> RequestProtocolIndex<'a> {
        RequestProtocolIndex {
            component_index: &self.component_index,
            route_cache,
            template_metadata_cache: TemplateMetadataCache::Shared(&self.template_metadata_cache),
        }
    }
}

/// Pre-computed index for a protocol, caching expensive lookups.
///
/// The component bit-index map is deterministic for a given protocol. The
/// route cache and template metadata cache support callers that create and
/// reuse a standalone index directly.
pub struct ProtocolIndex {
    /// Component name → bit index for inventory tracking.
    pub component_index: HashMap<String, u32>,
    /// Compiled route template patterns (lazily populated on first match).
    pub route_cache: CompiledRouteCache,
    /// Parsed WebUI template metadata keyed by component tag.
    template_metadata_cache: HashMap<String, Value>,
}

struct ComponentAssets {
    styles: Vec<Value>,
    templates: serde_json::Map<String, Value>,
    functions: serde_json::Map<String, Value>,
}

impl ProtocolIndex {
    /// Build a protocol index from a compiled protocol.
    #[must_use]
    pub fn new(protocol: &WebUIProtocol) -> Self {
        Self {
            component_index: build_component_index(protocol),
            route_cache: CompiledRouteCache::new(),
            template_metadata_cache: HashMap::new(),
        }
    }

    fn request_index(&mut self) -> RequestProtocolIndex<'_> {
        RequestProtocolIndex {
            component_index: &self.component_index,
            route_cache: &mut self.route_cache,
            template_metadata_cache: TemplateMetadataCache::Exclusive(
                &mut self.template_metadata_cache,
            ),
        }
    }
}

enum TemplateMetadataCache<'a> {
    Exclusive(&'a mut HashMap<String, Value>),
    Shared(&'a RwLock<HashMap<String, Value>>),
}

struct RequestProtocolIndex<'a> {
    component_index: &'a HashMap<String, u32>,
    route_cache: &'a mut CompiledRouteCache,
    template_metadata_cache: TemplateMetadataCache<'a>,
}

// ── Component Inventory ─────────────────────────────────────────────────

/// Build a deterministic component-name → bit-index map from the protocol.
///
/// Derives names from fragment keys (hyphenated = custom element) since that
/// is the source of truth regardless of whether a plugin populated
/// `protocol.components`. Components are sorted alphabetically; index =
/// position in that order.
pub fn build_component_index(protocol: &WebUIProtocol) -> HashMap<String, u32> {
    let mut sorted: Vec<&String> = protocol
        .fragments
        .keys()
        .filter(|key| key.contains('-'))
        .collect();
    sorted.sort_unstable();
    let mut index = HashMap::with_capacity(sorted.len());
    for (i, name) in sorted.into_iter().enumerate() {
        // Index count bounded by component registry size, well within u32 range
        #[allow(clippy::cast_possible_truncation)]
        let idx = i as u32;
        index.insert(name.clone(), idx);
    }
    index
}

/// Check if a component's bit is set in the inventory bitfield.
fn has_component(inventory: &[u8], index: u32) -> bool {
    let byte_idx = (index / 8) as usize;
    let bit_idx = index % 8;
    byte_idx < inventory.len() && inventory[byte_idx] & (1 << bit_idx) != 0
}

/// Set a component's bit in the inventory bitfield.
fn set_component(inventory: &mut Vec<u8>, index: u32) {
    let byte_idx = (index / 8) as usize;
    let bit_idx = index % 8;
    if byte_idx >= inventory.len() {
        inventory.resize(byte_idx + 1, 0);
    }
    inventory[byte_idx] |= 1 << bit_idx;
}

/// Parse a hex-encoded inventory string into bytes.
///
/// Returns an error if the hex string has odd length or contains non-hex characters.
pub fn parse_inventory(hex: &str) -> Result<Vec<u8>, HandlerError> {
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    if !hex.len().is_multiple_of(2) {
        return Err(HandlerError::Protocol(
            webui_protocol::ProtocolError::Validation(format!(
                "inventory hex has odd length: {}",
                hex.len()
            )),
        ));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.bytes();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        let h = route_matcher::hex_val(hi).ok_or_else(|| {
            HandlerError::Protocol(webui_protocol::ProtocolError::Validation(format!(
                "invalid hex digit in inventory: {:#04x}",
                hi
            )))
        })?;
        let l = route_matcher::hex_val(lo).ok_or_else(|| {
            HandlerError::Protocol(webui_protocol::ProtocolError::Validation(format!(
                "invalid hex digit in inventory: {:#04x}",
                lo
            )))
        })?;
        bytes.push((h << 4) | l);
    }
    Ok(bytes)
}

/// Encode an inventory bitfield as a hex string.
pub fn encode_inventory(inv: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(inv.len() * 2);
    for byte in inv {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Encode the inventory bitfield for a known set of rendered components.
pub(crate) fn encode_component_inventory(
    component_names: &HashSet<String>,
    index: &HashMap<String, u32>,
) -> String {
    let mut inventory = Vec::new();
    for name in component_names {
        if let Some(&idx) = index.get(name.as_str()) {
            set_component(&mut inventory, idx);
        }
    }
    encode_inventory(&inventory)
}

/// Walk the protocol fragment graph from `entry_id` and return the names of
/// all components the route needs that are NOT in the client's inventory.
///
/// Returns `(needed_names, updated_inventory_hex)`.
pub fn get_needed_components(
    protocol: &WebUIProtocol,
    entry_id: &str,
    inventory_hex: &str,
    component_index: &HashMap<String, u32>,
) -> Result<(Vec<String>, String), HandlerError> {
    let component_names = collect_inventoryable_components(
        protocol,
        entry_id,
        None,
        true,
        &mut CompiledRouteCache::new(),
    );
    filter_needed_components(&component_names, inventory_hex, component_index)
}

/// Walk the protocol from the persistent `entry_id` and return the component
/// templates needed for the current `request_path`.
///
/// Returns `(needed_names, updated_inventory_hex)`.
pub fn get_needed_components_for_request(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
    component_index: &HashMap<String, u32>,
) -> Result<(Vec<String>, String), HandlerError> {
    let component_names = collect_inventoryable_components(
        protocol,
        entry_id,
        Some(request_path),
        false,
        &mut CompiledRouteCache::new(),
    );
    filter_needed_components(&component_names, inventory_hex, component_index)
}

/// Render partial-navigation metadata using a prepared protocol and its reusable index.
///
/// This state-free form is intended for NDJSON chunk 1. Use
/// [`render_partial_prepared`] for a complete JSON response.
pub fn render_partial_metadata_prepared(
    prepared: &PreparedProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<Value, HandlerError> {
    let mut route_cache = CompiledRouteCache::new();
    let mut index = prepared.request_index(&mut route_cache);
    render_partial_indexed(
        prepared.protocol(),
        entry_id,
        request_path,
        inventory_hex,
        &mut index,
    )
}

/// Produce a complete partial-navigation response with request-projected state.
///
/// The input JSON is validated without materializing the full state tree.
/// Only keys required to create or update components reachable on the active
/// route are copied into the response.
pub fn render_partial_prepared(
    prepared: &PreparedProtocol,
    state_json: &str,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<String, HandlerError> {
    let mut route_cache = CompiledRouteCache::new();
    let mut index = prepared.request_index(&mut route_cache);
    let (response, state_selection) = render_partial_indexed_with_state(
        prepared.protocol(),
        entry_id,
        request_path,
        inventory_hex,
        &mut index,
    )?;
    serialize_partial_response(&response, state_json, &state_selection)
}

/// Render an action response using a prepared protocol and its reusable index.
pub fn render_action_response_prepared(
    prepared: &PreparedProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
) -> Result<Value, HandlerError> {
    let mut route_cache = CompiledRouteCache::new();
    let mut index = prepared.request_index(&mut route_cache);
    Ok(render_action_response_indexed(
        prepared.protocol(),
        state,
        entry_id,
        request_path,
        &mut index,
    ))
}

/// Render component template payloads using a prepared protocol index.
pub fn render_component_templates_prepared(
    prepared: &PreparedProtocol,
    component_tags: &[&str],
    inventory_hex: &str,
) -> Result<Value, HandlerError> {
    let mut route_cache = CompiledRouteCache::new();
    let mut index = prepared.request_index(&mut route_cache);
    render_component_templates_indexed(
        prepared.protocol(),
        component_tags,
        inventory_hex,
        &mut index,
    )
}

fn serialize_partial_response(
    response: &Value,
    state_json: &str,
    state_selection: &StateSelection<'_>,
) -> Result<String, HandlerError> {
    let response = response
        .as_object()
        .ok_or_else(partial_response_not_object)?;
    let state = select_raw_state(state_json, state_selection)?;
    serde_json::to_string(&PartialResponseWithState { response, state })
        .map_err(|error| partial_serialize_error(&error.to_string()))
}

fn validate_json(json: &str) -> Result<(), HandlerError> {
    validate_json_inner(json).map_err(|error| invalid_state_json(&error.to_string()))
}

fn validate_json_inner(json: &str) -> serde_json::Result<()> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    ValidJson
        .deserialize(&mut deserializer)
        .and_then(|_| deserializer.end())
}

struct ValidJson;

impl<'de> DeserializeSeed<'de> for ValidJson {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ValidJsonVisitor)
    }
}

struct ValidJsonVisitor;

impl<'de> Visitor<'de> for ValidJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("valid JSON")
    }

    fn visit_bool<E>(self, _value: bool) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<(), E>
    where
        E: serde::de::Error,
    {
        if value.is_finite() {
            Ok(())
        } else {
            Err(E::custom("number out of range"))
        }
    }

    fn visit_str<E>(self, _value: &str) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_borrowed_str<E>(self, _value: &'de str) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        ValidJson.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(ValidJson)?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(ValidJson)?;
        }
        Ok(())
    }
}

struct PartialResponseWithState<'a, 'state> {
    response: &'a Map<String, Value>,
    state: SelectedRawState<'state>,
}

impl Serialize for PartialResponseWithState<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.response.len().saturating_add(1)))?;
        for (key, value) in self.response {
            map.serialize_entry(key, value)?;
        }
        map.serialize_entry("state", &self.state)?;
        map.end()
    }
}

struct ProjectedRawState<'de> {
    entries: Vec<(Cow<'de, str>, &'de RawValue)>,
}

enum SelectedRawState<'de> {
    Full(&'de RawValue),
    Keys(ProjectedRawState<'de>),
}

impl Serialize for SelectedRawState<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Full(state) => state.serialize(serializer),
            Self::Keys(state) => state.serialize(serializer),
        }
    }
}

impl Serialize for ProjectedRawState<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (key, value) in &self.entries {
            map.serialize_entry(key.as_ref(), value)?;
        }
        map.end()
    }
}

fn project_raw_state<'de>(
    state_json: &'de str,
    state_keys: &[&str],
) -> Result<ProjectedRawState<'de>, HandlerError> {
    let is_object = state_json
        .as_bytes()
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'{');
    if !is_object {
        validate_json(state_json)?;
        return Ok(ProjectedRawState {
            entries: Vec::new(),
        });
    }

    let mut deserializer = serde_json::Deserializer::from_str(state_json);
    let mut state = ProjectedRawStateSeed { state_keys }
        .deserialize(&mut deserializer)
        .and_then(|state| {
            deserializer.end()?;
            Ok(state)
        })
        .map_err(|error| invalid_state_json(&error.to_string()))?;
    state.entries.sort_unstable_by(|left, right| {
        left.0.as_ref().as_bytes().cmp(right.0.as_ref().as_bytes())
    });
    Ok(state)
}

fn select_raw_state<'de>(
    state_json: &'de str,
    selection: &StateSelection<'_>,
) -> Result<SelectedRawState<'de>, HandlerError> {
    match selection {
        StateSelection::Full => serde_json::from_str::<&RawValue>(state_json)
            .map(SelectedRawState::Full)
            .map_err(|error| invalid_state_json(&error.to_string())),
        StateSelection::Keys(keys) => {
            project_raw_state(state_json, keys).map(SelectedRawState::Keys)
        }
    }
}

struct ProjectedRawStateSeed<'a> {
    state_keys: &'a [&'a str],
}

impl<'de> DeserializeSeed<'de> for ProjectedRawStateSeed<'_> {
    type Value = ProjectedRawState<'de>;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ProjectedRawStateVisitor {
            state_keys: self.state_keys,
        })
    }
}

struct ProjectedRawStateVisitor<'a> {
    state_keys: &'a [&'a str],
}

impl<'de> Visitor<'de> for ProjectedRawStateVisitor<'_> {
    type Value = ProjectedRawState<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries: Vec<(Cow<'de, str>, &'de RawValue)> =
            Vec::with_capacity(self.state_keys.len());
        while let Some(key) = map.next_key_seed(BorrowedString)? {
            if self.state_keys.binary_search(&key.as_ref()).is_err() {
                map.next_value_seed(ValidJson)?;
                continue;
            }

            let value: &'de RawValue = map.next_value()?;
            validate_json_inner(value.get()).map_err(serde::de::Error::custom)?;
            if let Some(entry) = entries
                .iter_mut()
                .find(|(existing, _)| existing.as_ref() == key.as_ref())
            {
                *entry = (key, value);
            } else {
                entries.push((key, value));
            }
        }
        Ok(ProjectedRawState { entries })
    }
}

struct BorrowedString;

impl<'de> DeserializeSeed<'de> for BorrowedString {
    type Value = Cow<'de, str>;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(BorrowedStringVisitor)
    }
}

struct BorrowedStringVisitor;

impl<'de> Visitor<'de> for BorrowedStringVisitor {
    type Value = Cow<'de, str>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string")
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E> {
        Ok(Cow::Borrowed(value))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(Cow::Owned(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(Cow::Owned(value))
    }
}

#[cold]
#[inline(never)]
fn invalid_state_json(message: &str) -> HandlerError {
    HandlerError::Rendering(format!("invalid state JSON: {message}"))
}

#[cold]
#[inline(never)]
fn partial_serialize_error(message: &str) -> HandlerError {
    HandlerError::Rendering(format!("failed to serialize partial response: {message}"))
}

#[cold]
#[inline(never)]
fn partial_response_not_object() -> HandlerError {
    HandlerError::Invariant("partial response must serialize as a JSON object".to_string())
}

/// Collect all route-reachable inventoryable components for the request path.
pub(crate) fn collect_reachable_components_for_request(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    cache: &mut CompiledRouteCache,
) -> HashSet<String> {
    // Callers here need set semantics (`.contains`, plugin `&HashSet` API);
    // discovery order is irrelevant for reachable-template emission.
    collect_inventoryable_components(protocol, entry_id, Some(request_path), false, cache)
        .into_iter()
        .collect()
}

/// Filter components against the client's inventory bitfield using sequential indices.
/// Zero collisions — each component has a unique bit.
///
/// Expects `component_names` to already be unique. Input order is preserved
/// in the returned `needed` vector (no alphabetical re-sort), so downstream
/// `<head>` CSS `<link>` emission follows document/traversal order. The
/// updated inventory hex is order-independent (bits are keyed by `index`).
///
/// Returns the missing component names and the updated inventory hex string.
pub fn filter_needed_components(
    component_names: &[String],
    inventory_hex: &str,
    index: &HashMap<String, u32>,
) -> Result<(Vec<String>, String), HandlerError> {
    let client_inv = parse_inventory(inventory_hex)?;
    let mut updated_inv = client_inv.clone();

    let mut needed = Vec::with_capacity(component_names.len());
    for name in component_names {
        if let Some(&idx) = index.get(name.as_str()) {
            if !has_component(&client_inv, idx) {
                needed.push(name.clone());
            }
            set_component(&mut updated_inv, idx);
        } else {
            // Component exists in the fragment graph but has no index entry
            // (no protocol.components record). It can't be tracked in the
            // bitfield, so we must always send it.
            needed.push(name.clone());
        }
    }

    Ok((needed, encode_inventory(&updated_inv)))
}

fn has_template_payload(component: &webui_protocol::ComponentData) -> bool {
    !component.template_json.is_empty() || !component.template.is_empty()
}

#[derive(Debug)]
struct QueuedFragment {
    id: String,
    inventoryable: bool,
    /// Base path for resolving relative route paths at this level.
    route_base: String,
}

/// Shared context for route child walkers, bundling common parameters
/// to stay within the 5-argument clippy limit.
struct ChildWalkCtx<'a> {
    request_path: &'a str,
    protocol: &'a WebUIProtocol,
    cache: &'a mut CompiledRouteCache,
}

/// Walk the fragment graph from `entry_id` and collect all inventoryable
/// component names in document/traversal (first-discovery) order.
///
/// Uses an iterative stack-based traversal. When `request_path` is provided,
/// the walk follows only the best-matching route at each nesting level (route-aware).
/// Without a request path, all route branches are followed conservatively.
///
/// Components are marked `inventoryable` when they have a corresponding entry
/// in `protocol.components` with a non-empty template payload — these are the
/// components whose client metadata the browser may need during navigation.
fn collect_inventoryable_components(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: Option<&str>,
    root_inventoryable: bool,
    cache: &mut CompiledRouteCache,
) -> Vec<String> {
    let mut visited_fragments = HashSet::new();
    // Preserve first-discovery (document/traversal) order so Link-strategy
    // CSS `<link>` tags are emitted in source order, not alphabetically.
    // `seen_components` dedups; `component_ids` keeps order (a plain
    // `HashSet` would lose it).
    let mut seen_components = HashSet::new();
    let mut component_ids: Vec<String> = Vec::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: root_inventoryable,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() {
            continue;
        }

        if queued.inventoryable && seen_components.insert(queued.id.clone()) {
            component_ids.push(queued.id.clone());
        }

        if !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route = request_path.and_then(|path| {
            route_renderer::find_best_route_match(
                &frag_list.fragments,
                path,
                &queued.route_base,
                cache,
            )
        });

        // Reversed iteration so this LIFO stack pops children in document
        // order, giving deterministic, source-ordered component discovery.
        for frag in frag_list.fragments.iter().rev() {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: true,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    stack.push(QueuedFragment {
                        id: for_loop.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::IfCond(if_cond)) => {
                    stack.push(QueuedFragment {
                        id: if_cond.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Attribute(attr)) if !attr.template.is_empty() => {
                    stack.push(QueuedFragment {
                        id: attr.template.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_frag.fragment_id.as_str());
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        // Compute new route base from consumed segments
                        let child_route_base = if let Some((_, ref rm)) = matched_route {
                            if let Some(path) = request_path {
                                route_matcher::compute_route_base(path, rm.consumed_segments)
                            } else {
                                queued.route_base.clone()
                            }
                        } else {
                            queued.route_base.clone()
                        };

                        stack.push(QueuedFragment {
                            id: route_frag.fragment_id.clone(),
                            inventoryable: protocol
                                .components
                                .get(&route_frag.fragment_id)
                                .is_some_and(has_template_payload),
                            route_base: child_route_base.clone(),
                        });

                        // Inventory pending/error components for the entire
                        // route subtree, not just the matched chain. Pending UI
                        // renders while a (possibly unmatched) sibling route's
                        // partial is in flight, and error UI renders if that
                        // fetch fails, so the client must already hold these
                        // templates before — or without — a successful fetch for
                        // the target route.
                        collect_route_boundary_components(
                            std::slice::from_ref(route_frag),
                            &child_route_base,
                            protocol,
                            &mut stack,
                        );

                        // Walk nested child routes to find the next matched level.
                        // This mirrors the handler's outlet rendering: match children
                        // against the request path and follow the matched chain.
                        if !route_frag.children.is_empty() {
                            if let Some(path) = request_path {
                                walk_route_children(
                                    &route_frag.children,
                                    &child_route_base,
                                    &mut stack,
                                    &mut ChildWalkCtx {
                                        request_path: path,
                                        protocol,
                                        cache,
                                    },
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    component_ids
}

/// Select the best-matching child route among siblings by specificity.
///
/// Returns the index and match result of the highest-specificity match,
/// preserving first-match-wins for equal specificity (declaration order).
fn select_best_child_route(
    children: &[WebUIFragmentRoute],
    request_path: &str,
    route_base: &str,
    cache: &mut CompiledRouteCache,
) -> Option<(usize, route_matcher::RouteMatch)> {
    let request_segments = route_matcher::split_request_path(request_path);
    let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
    for (idx, child) in children.iter().enumerate() {
        let resolved = route_matcher::resolve_route_path_cow(&child.path, route_base);
        if let Some(m) = route_matcher::match_route_cached_with_segments(
            cache,
            resolved.as_ref(),
            &request_segments,
            child.exact,
        ) {
            let is_better = best
                .as_ref()
                .is_none_or(|(_, prev)| m.specificity > prev.specificity);
            if is_better {
                best = Some((idx, m));
            }
        }
    }
    best
}

/// Walk nested route children to find matched routes and add their
/// components to the inventory stack. Mirrors the handler's outlet rendering.
fn walk_route_children(
    children: &[WebUIFragmentRoute],
    route_base: &str,
    stack: &mut Vec<QueuedFragment>,
    ctx: &mut ChildWalkCtx<'_>,
) {
    let mut current = children;
    let mut base = route_base.to_string();

    loop {
        let Some((idx, ref rm)) =
            select_best_child_route(current, ctx.request_path, &base, ctx.cache)
        else {
            break;
        };
        let matched = &current[idx];
        if matched.fragment_id.is_empty() {
            break;
        }

        let child_base = route_matcher::compute_route_base(ctx.request_path, rm.consumed_segments);

        stack.push(QueuedFragment {
            id: matched.fragment_id.clone(),
            inventoryable: ctx
                .protocol
                .components
                .get(&matched.fragment_id)
                .is_some_and(has_template_payload),
            route_base: child_base.clone(),
        });

        if !matched.pending_component.is_empty() {
            stack.push(QueuedFragment {
                id: matched.pending_component.clone(),
                inventoryable: ctx
                    .protocol
                    .components
                    .get(&matched.pending_component)
                    .is_some_and(has_template_payload),
                route_base: child_base.clone(),
            });
        }
        if !matched.error_component.is_empty() {
            stack.push(QueuedFragment {
                id: matched.error_component.clone(),
                inventoryable: ctx
                    .protocol
                    .components
                    .get(&matched.error_component)
                    .is_some_and(has_template_payload),
                route_base: child_base.clone(),
            });
        }

        if matched.children.is_empty() {
            break;
        }
        current = &matched.children;
        base = child_base;
    }
}

/// Inventory the pending/error boundary components for an entire route subtree.
///
/// Unlike [`walk_route_children`], which follows only the best-matching chain,
/// this collects boundary components for *every* route reachable from `routes`.
/// Pending UI renders while a route's partial fetch is in flight and error UI
/// renders if that fetch fails, so the client must already hold these templates
/// for any navigable sibling — not only the currently active route. The walk is
/// iterative because the framework forbids recursion in core paths.
fn collect_route_boundary_components(
    routes: &[WebUIFragmentRoute],
    route_base: &str,
    protocol: &WebUIProtocol,
    stack: &mut Vec<QueuedFragment>,
) {
    let mut remaining: Vec<&WebUIFragmentRoute> = routes.iter().collect();
    while let Some(route) = remaining.pop() {
        for component in [&route.pending_component, &route.error_component] {
            if component.is_empty() {
                continue;
            }
            stack.push(QueuedFragment {
                id: component.clone(),
                inventoryable: protocol
                    .components
                    .get(component)
                    .is_some_and(has_template_payload),
                route_base: route_base.to_string(),
            });
        }
        remaining.extend(route.children.iter());
    }
}

/// Walk the fragment graph following matched routes and collect all route
/// parameters from every level of the active route chain.
///
/// This is used by the dev server to inject nested route params into state.
pub fn collect_nested_route_params(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    cache: &mut CompiledRouteCache,
) -> HashMap<String, String> {
    let mut all_params = HashMap::new();
    let mut visited_fragments = HashSet::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: false,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() || !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route = route_renderer::find_best_route_match(
            &frag_list.fragments,
            request_path,
            &queued.route_base,
            cache,
        );

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_frag.fragment_id.as_str());
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        if let Some((_, ref rm)) = matched_route {
                            // Collect params from this route level
                            all_params.extend(rm.params.clone());

                            let child_route_base = route_matcher::compute_route_base(
                                request_path,
                                rm.consumed_segments,
                            );

                            stack.push(QueuedFragment {
                                id: route_frag.fragment_id.clone(),
                                inventoryable: false,
                                route_base: child_route_base.clone(),
                            });

                            // Walk nested children to collect params from deeper levels
                            collect_params_from_children(
                                &route_frag.children,
                                request_path,
                                &child_route_base,
                                &mut all_params,
                                cache,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    all_params
}

/// Iteratively collect route params from nested route children.
fn collect_params_from_children(
    children: &[WebUIFragmentRoute],
    request_path: &str,
    route_base: &str,
    all_params: &mut HashMap<String, String>,
    cache: &mut CompiledRouteCache,
) {
    let mut current = children;
    let mut base = route_base.to_string();

    loop {
        let Some((idx, rm)) = select_best_child_route(current, request_path, &base, cache) else {
            break;
        };
        all_params.extend(rm.params);
        let matched = &current[idx];
        if matched.children.is_empty() {
            break;
        }
        let child_base = route_matcher::compute_route_base(request_path, rm.consumed_segments);
        current = &matched.children;
        base = child_base;
    }
}

// ── Partial Response ────────────────────────────────────────────────

/// Resolve `{param}` placeholders in a list of tag templates.
///
/// Replaces `{paramName}` with the actual value from `params`.
/// Tags without placeholders are returned as-is. Missing params
/// result in the placeholder being left unresolved (defensive).
fn resolve_tag_templates(templates: &[String], params: &HashMap<String, String>) -> Vec<String> {
    let mut resolved = Vec::with_capacity(templates.len());
    for template in templates {
        if !template.contains('{') {
            resolved.push(template.clone());
            continue;
        }
        let mut result = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            result.push_str(&rest[..open]);
            rest = &rest[open + 1..];
            if let Some(close) = rest.find('}') {
                let name = &rest[..close];
                if let Some(value) = params.get(name) {
                    result.push_str(value);
                } else {
                    result.push('{');
                    result.push_str(name);
                    result.push('}');
                }
                rest = &rest[close + 1..];
            } else {
                result.push('{');
            }
        }
        result.push_str(rest);
        resolved.push(result);
    }
    resolved
}

/// Single-pass graph walk that collects both inventoryable component names
/// and the matched route chain. Eliminates the duplicate graph traversal
/// that previously existed in `render_partial`.
fn collect_inventory_and_chain(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    index: &mut RequestProtocolIndex<'_>,
) -> (Vec<String>, Vec<RouteChainEntry>) {
    let mut visited_fragments = HashSet::new();
    // Deduped ordered collection: `filter_needed_components` now takes a
    // slice of unique names. Order is irrelevant for this consumer
    // (`render_partial` re-sorts in `collect_component_assets`), but the
    // list must stay duplicate-free.
    let mut seen_components = HashSet::new();
    let mut component_ids: Vec<String> = Vec::new();
    let mut chain = Vec::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: false,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() {
            continue;
        }

        if queued.inventoryable && seen_components.insert(queued.id.clone()) {
            component_ids.push(queued.id.clone());
        }

        if !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route = route_renderer::find_best_route_match(
            &frag_list.fragments,
            request_path,
            &queued.route_base,
            &mut *index.route_cache,
        );

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    // Inventory: components are inventoryable
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: true,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    // Inventory: follow control-flow edges conservatively
                    stack.push(QueuedFragment {
                        id: for_loop.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::IfCond(if_cond)) => {
                    stack.push(QueuedFragment {
                        id: if_cond.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Attribute(attr)) if !attr.template.is_empty() => {
                    stack.push(QueuedFragment {
                        id: attr.template.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_frag.fragment_id.as_str());
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        if let Some((_, ref rm)) = matched_route {
                            // Chain: record matched route entry
                            chain.push(RouteChainEntry {
                                component: route_frag.fragment_id.clone(),
                                path: route_frag.path.clone(),
                                params: rm.params.clone(),
                                exact: route_frag.exact,
                                allowed_query: route_frag.allowed_query.clone(),
                                keep_alive: route_frag.keep_alive,
                                cache_tags: route_frag.cache_tags.clone(),
                                invalidates: route_frag.invalidates.clone(),
                                pending_component: route_frag.pending_component.clone(),
                                error_component: route_frag.error_component.clone(),
                            });

                            let child_route_base = route_matcher::compute_route_base(
                                request_path,
                                rm.consumed_segments,
                            );

                            // Inventory: follow matched route component
                            let is_inventoryable = protocol
                                .components
                                .get(&route_frag.fragment_id)
                                .is_some_and(has_template_payload);
                            stack.push(QueuedFragment {
                                id: route_frag.fragment_id.clone(),
                                inventoryable: is_inventoryable,
                                route_base: child_route_base.clone(),
                            });

                            collect_route_boundary_components(
                                std::slice::from_ref(route_frag),
                                &child_route_base,
                                protocol,
                                &mut stack,
                            );

                            // Both: walk nested child routes
                            if !route_frag.children.is_empty() {
                                walk_children_for_inventory_and_chain(
                                    &route_frag.children,
                                    &child_route_base,
                                    &mut stack,
                                    &mut chain,
                                    &mut ChildWalkCtx {
                                        request_path,
                                        protocol,
                                        cache: &mut *index.route_cache,
                                    },
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    (component_ids, chain)
}

/// Walk nested route children, collecting both inventory and chain entries.
fn walk_children_for_inventory_and_chain(
    children: &[WebUIFragmentRoute],
    route_base: &str,
    stack: &mut Vec<QueuedFragment>,
    chain: &mut Vec<RouteChainEntry>,
    ctx: &mut ChildWalkCtx<'_>,
) {
    let mut current = children;
    let mut base = route_base.to_string();

    loop {
        let Some((idx, ref rm)) =
            select_best_child_route(current, ctx.request_path, &base, ctx.cache)
        else {
            break;
        };
        let matched = &current[idx];
        if matched.fragment_id.is_empty() {
            break;
        }

        let child_base = route_matcher::compute_route_base(ctx.request_path, rm.consumed_segments);

        chain.push(RouteChainEntry {
            component: matched.fragment_id.clone(),
            path: matched.path.clone(),
            params: rm.params.clone(),
            exact: matched.exact,
            allowed_query: matched.allowed_query.clone(),
            keep_alive: matched.keep_alive,
            cache_tags: matched.cache_tags.clone(),
            invalidates: matched.invalidates.clone(),
            pending_component: matched.pending_component.clone(),
            error_component: matched.error_component.clone(),
        });

        stack.push(QueuedFragment {
            id: matched.fragment_id.clone(),
            inventoryable: ctx
                .protocol
                .components
                .get(&matched.fragment_id)
                .is_some_and(has_template_payload),
            route_base: child_base.clone(),
        });

        if matched.children.is_empty() {
            break;
        }
        current = &matched.children;
        base = child_base;
    }
}

/// Produce state-free partial-navigation metadata for NDJSON chunk 1.
///
/// Returns the matched route chain, templates, inventory, and cache tags.
/// State is intentionally excluded so a streaming host can send it later.
/// Use [`render_partial`] for a complete JSON response.
///
/// Returns a `serde_json::Value` object with fields:
/// - `templateStyles`: module CSS definition tags for inventory-new components (empty for Link/Style)
/// - `templates`: client template metadata keyed by component tag (inventory-filtered)
/// - `templateFunctions`: component-local condition closure arrays keyed by component tag
/// - `inventory`: updated hex bitmask
/// - `path`: the request path
/// - `chain`: matched route chain array
/// - `cacheTags`: resolved cache tags from the full route chain (union of all levels)
pub fn render_partial_metadata(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
    index: &mut ProtocolIndex,
) -> Result<Value, HandlerError> {
    let mut request_index = index.request_index();
    render_partial_indexed(
        protocol,
        entry_id,
        request_path,
        inventory_hex,
        &mut request_index,
    )
}

/// Produce a complete partial-navigation response with projected parsed state.
///
/// Only keys required to create or update client components reachable on the
/// active route are retained. Authored components use their explicit
/// navigation surface (`@observable + @attr + template roots`); scriptless
/// templates use compiled template roots.
pub fn render_partial(
    protocol: &WebUIProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> Result<Value, HandlerError> {
    let mut index = ProtocolIndex::new(protocol);
    let mut request_index = index.request_index();
    let (mut response, state_selection) = render_partial_indexed_with_state(
        protocol,
        entry_id,
        request_path,
        inventory_hex,
        &mut request_index,
    )?;
    let response = response
        .as_object_mut()
        .ok_or_else(partial_response_not_object)?;
    response.insert("state".into(), select_owned_state(state, &state_selection));
    Ok(Value::Object(std::mem::take(response)))
}

fn render_partial_indexed(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
    index: &mut RequestProtocolIndex<'_>,
) -> Result<Value, HandlerError> {
    render_partial_indexed_with_state(protocol, entry_id, request_path, inventory_hex, index)
        .map(|(response, _)| response)
}

fn render_partial_indexed_with_state<'a>(
    protocol: &'a WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    inventory_hex: &str,
    index: &mut RequestProtocolIndex<'_>,
) -> Result<(Value, StateSelection<'a>), HandlerError> {
    // Single-pass walk: collect both inventory components and route chain.
    let (component_ids, mut chain) =
        collect_inventory_and_chain(protocol, entry_id, request_path, index);
    let state_selection =
        crate::collect_navigation_state(protocol, component_ids.iter().map(String::as_str));

    let (needed_names, updated_inv) =
        filter_needed_components(&component_ids, inventory_hex, index.component_index)?;

    // Resolve cache tags and invalidation templates with accumulated params.
    let mut accumulated_params: HashMap<String, String> = HashMap::new();
    let mut all_resolved_tags: Vec<String> = Vec::new();

    for entry in &mut chain {
        for (k, v) in &entry.params {
            accumulated_params.insert(k.clone(), v.clone());
        }
        entry.cache_tags = resolve_tag_templates(&entry.cache_tags, &accumulated_params);
        entry.invalidates = resolve_tag_templates(&entry.invalidates, &accumulated_params);
        all_resolved_tags.extend(entry.cache_tags.iter().cloned());
    }

    let tag_refs: Vec<&str> = needed_names.iter().map(|s| s.as_str()).collect();
    let assets = collect_component_assets(protocol, &tag_refs, index)?;

    let chain_array = Value::Array(chain.iter().map(RouteChainEntry::to_json).collect());

    let mut result = serde_json::Map::with_capacity(7);
    result.insert("templateStyles".into(), Value::Array(assets.styles));
    result.insert("templates".into(), Value::Object(assets.templates));
    result.insert("templateFunctions".into(), Value::Object(assets.functions));
    result.insert("inventory".into(), Value::String(updated_inv));
    result.insert("path".into(), Value::String(request_path.to_string()));
    result.insert("chain".into(), chain_array);
    if !all_resolved_tags.is_empty() {
        let mut seen = HashSet::new();
        let deduped: Vec<Value> = all_resolved_tags
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .map(Value::String)
            .collect();
        result.insert("cacheTags".into(), Value::Array(deduped));
    }
    Ok((Value::Object(result), state_selection))
}

fn select_owned_state(state: Value, selection: &StateSelection<'_>) -> Value {
    let StateSelection::Keys(state_keys) = selection else {
        return state;
    };
    let Value::Object(mut source) = state else {
        return Value::Object(Map::new());
    };
    let mut projected = Map::with_capacity(state_keys.len().min(source.len()));
    for &key in state_keys {
        if let Some(value) = source.remove(key) {
            projected.insert(key.to_owned(), value);
        }
    }
    Value::Object(projected)
}

/// Produce a JSON response for a POST mutation action.
///
/// Walks the matched route chain, resolves `invalidates` tag templates
/// with actual param values, and returns them alongside the application
/// state. Host servers call this for `POST` requests to route paths.
///
/// Returns a `serde_json::Value` object with fields:
/// - `state`: the application state passed through
/// - `invalidateTags`: resolved invalidation tags from the matched leaf route
/// - `path`: the request path
pub fn render_action_response(
    protocol: &WebUIProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
    index: &mut ProtocolIndex,
) -> Value {
    let mut request_index = index.request_index();
    render_action_response_indexed(protocol, state, entry_id, request_path, &mut request_index)
}

fn render_action_response_indexed(
    protocol: &WebUIProtocol,
    state: Value,
    entry_id: &str,
    request_path: &str,
    index: &mut RequestProtocolIndex<'_>,
) -> Value {
    let chain = collect_route_chain(protocol, entry_id, request_path, &mut *index.route_cache);

    // Accumulate params across the chain for tag resolution
    let mut accumulated_params: HashMap<String, String> = HashMap::new();
    let mut all_invalidates: Vec<String> = Vec::new();

    for entry in &chain {
        for (k, v) in &entry.params {
            accumulated_params.insert(k.clone(), v.clone());
        }
        let resolved = resolve_tag_templates(&entry.invalidates, &accumulated_params);
        all_invalidates.extend(resolved);
    }

    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    let deduped: Vec<Value> = all_invalidates
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .map(Value::String)
        .collect();

    let mut result = serde_json::Map::with_capacity(3);
    result.insert("state".into(), state);
    result.insert("invalidateTags".into(), Value::Array(deduped));
    result.insert("path".into(), Value::String(request_path.to_string()));
    Value::Object(result)
}

/// Return compiled templates and CSS for specific components by tag name.
///
/// This supports on-demand loading of components that aren't part of the
/// route tree (e.g., dialogs, popovers) — the client calls this before
/// creating the element so templates + styles are registered without FOUC.
///
/// Uses the same inventory bitfield as partial navigation to avoid sending
/// templates the client already has.
pub fn render_component_templates(
    protocol: &WebUIProtocol,
    component_tags: &[&str],
    inventory_hex: &str,
    index: &mut ProtocolIndex,
) -> Result<Value, HandlerError> {
    let mut request_index = index.request_index();
    render_component_templates_indexed(protocol, component_tags, inventory_hex, &mut request_index)
}

fn render_component_templates_indexed(
    protocol: &WebUIProtocol,
    component_tags: &[&str],
    inventory_hex: &str,
    index: &mut RequestProtocolIndex<'_>,
) -> Result<Value, HandlerError> {
    // Deduplicate preserving caller order; filter expects unique names.
    let mut seen: HashSet<&str> = HashSet::new();
    let requested: Vec<String> = component_tags
        .iter()
        .filter(|s| seen.insert(**s))
        .map(|s| (*s).to_string())
        .collect();
    let (needed, updated_inv) =
        filter_needed_components(&requested, inventory_hex, index.component_index)?;

    let tag_refs: Vec<&str> = needed.iter().map(|s| s.as_str()).collect();
    let assets = collect_component_assets(protocol, &tag_refs, index)?;

    let mut result = serde_json::Map::with_capacity(4);
    result.insert("templateStyles".into(), Value::Array(assets.styles));
    result.insert("templates".into(), Value::Object(assets.templates));
    result.insert("templateFunctions".into(), Value::Object(assets.functions));
    result.insert("inventory".into(), Value::String(updated_inv));
    Ok(Value::Object(result))
}

/// Shared helper: collect templates and module CSS styles for a set of component tags.
fn collect_component_assets(
    protocol: &WebUIProtocol,
    tags: &[&str],
    index: &mut RequestProtocolIndex<'_>,
) -> Result<ComponentAssets, HandlerError> {
    let mut style_array = Vec::new();
    let mut tmpl_map = serde_json::Map::new();
    let mut function_map = serde_json::Map::new();

    // Sort for deterministic output (reproducible responses, cache keys)
    let mut sorted_tags: Vec<&str> = tags.to_vec();
    sorted_tags.sort_unstable();

    for tag in sorted_tags {
        let Some(component) = protocol.components.get(tag) else {
            continue;
        };
        if !has_template_payload(component) {
            continue;
        }
        if !component.css.is_empty() {
            // No nonce here — the per-request CSP nonce is attached
            // client-side by the router when it materializes each
            // importmap script tag into the DOM.
            let tag_html = crate::css_module::build_importmap_tag(tag, &component.css, None);
            style_array.push(Value::String(tag_html));
        }
        if !component.template_json.is_empty() {
            let template_value = cached_template_metadata(
                &mut index.template_metadata_cache,
                tag,
                &component.template_json,
            )?;
            tmpl_map.insert(tag.to_string(), template_value);
            if !component.template_functions.is_empty() {
                function_map.insert(
                    tag.to_string(),
                    Value::String(component.template_functions.clone()),
                );
            }
        } else {
            tmpl_map.insert(tag.to_string(), Value::String(component.template.clone()));
        }
    }

    Ok(ComponentAssets {
        styles: style_array,
        templates: tmpl_map,
        functions: function_map,
    })
}

fn cached_template_metadata(
    cache: &mut TemplateMetadataCache<'_>,
    tag: &str,
    template_json: &str,
) -> Result<Value, HandlerError> {
    match cache {
        TemplateMetadataCache::Exclusive(cache) => {
            cached_template_metadata_exclusive(cache, tag, template_json)
        }
        TemplateMetadataCache::Shared(cache) => {
            if let Some(value) = cache
                .read()
                .map_err(|_| prepared_metadata_cache_poisoned())?
                .get(tag)
                .cloned()
            {
                return Ok(value);
            }

            let value = parse_template_metadata(tag, template_json)?;
            let mut cache = cache
                .write()
                .map_err(|_| prepared_metadata_cache_poisoned())?;
            Ok(cache.entry(tag.to_string()).or_insert(value).clone())
        }
    }
}

fn cached_template_metadata_exclusive(
    cache: &mut HashMap<String, Value>,
    tag: &str,
    template_json: &str,
) -> Result<Value, HandlerError> {
    if let Some(value) = cache.get(tag) {
        return Ok(value.clone());
    }

    let value = parse_template_metadata(tag, template_json)?;
    cache.insert(tag.to_string(), value.clone());
    Ok(value)
}

fn parse_template_metadata(tag: &str, template_json: &str) -> Result<Value, HandlerError> {
    serde_json::from_str(template_json).map_err(|error| {
        HandlerError::Rendering(format!(
            "failed to parse template metadata for {tag}: {error}"
        ))
    })
}

#[cold]
#[inline(never)]
fn prepared_metadata_cache_poisoned() -> HandlerError {
    HandlerError::Invariant(
        "prepared protocol metadata cache is unavailable after a previous panic".to_string(),
    )
}

// ── Route Chain ─────────────────────────────────────────────────────

/// A single entry in the matched route chain, one per nesting level.
#[derive(Debug, Clone)]
pub struct RouteChainEntry {
    /// Component tag name for this route level.
    pub component: String,
    /// The route path pattern (as declared in the template).
    pub path: String,
    /// Bound route parameters at this level.
    pub params: HashMap<String, String>,
    /// Whether this route requires an exact match.
    pub exact: bool,
    /// Comma-separated allowlist of query parameters forwarded as attributes.
    pub allowed_query: String,
    /// When true, the router keeps the component alive across navigations.
    pub keep_alive: bool,
    /// Cache tag templates from the proto (e.g. `["thread:{threadId}", "inbox"]`).
    pub cache_tags: Vec<String>,
    /// Invalidation tag templates from the proto.
    pub invalidates: Vec<String>,
    /// Component tag name for pending/loading UI.
    pub pending_component: String,
    /// Component tag name for error boundary UI.
    pub error_component: String,
}

impl RouteChainEntry {
    /// Serialize this entry to a JSON value ready for inclusion in a partial response.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::with_capacity(9);
        obj.insert("component".into(), Value::String(self.component.clone()));
        obj.insert("path".into(), Value::String(self.path.clone()));
        if !self.params.is_empty() {
            let params_obj: serde_json::Map<String, Value> = self
                .params
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            obj.insert("params".into(), Value::Object(params_obj));
        }
        if self.exact {
            obj.insert("exact".into(), Value::Bool(true));
        }
        if !self.allowed_query.is_empty() {
            obj.insert(
                "allowedQuery".into(),
                Value::String(self.allowed_query.clone()),
            );
        }
        if self.keep_alive {
            obj.insert("keepAlive".into(), Value::Bool(true));
        }
        if !self.pending_component.is_empty() {
            obj.insert(
                "pendingComponent".into(),
                Value::String(self.pending_component.clone()),
            );
        }
        if !self.error_component.is_empty() {
            obj.insert(
                "errorComponent".into(),
                Value::String(self.error_component.clone()),
            );
        }
        if !self.invalidates.is_empty() {
            obj.insert(
                "invalidates".into(),
                Value::Array(
                    self.invalidates
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        Value::Object(obj)
    }
}

/// Collect the matched route chain for a request path.
///
/// Walks the fragment graph from `entry_id`, follows the matched route at
/// each nesting level, and returns a chain entry per matched level.
pub fn collect_route_chain(
    protocol: &WebUIProtocol,
    entry_id: &str,
    request_path: &str,
    cache: &mut CompiledRouteCache,
) -> Vec<RouteChainEntry> {
    let mut chain = Vec::new();
    let mut visited_fragments = HashSet::new();
    let mut stack = vec![QueuedFragment {
        id: entry_id.to_string(),
        inventoryable: false,
        route_base: "/".to_string(),
    }];

    while let Some(queued) = stack.pop() {
        if queued.id.is_empty() || !visited_fragments.insert(queued.id.clone()) {
            continue;
        }

        let Some(frag_list) = protocol.fragments.get(&queued.id) else {
            continue;
        };

        let matched_route = route_renderer::find_best_route_match(
            &frag_list.fragments,
            request_path,
            &queued.route_base,
            cache,
        );

        for frag in &frag_list.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(QueuedFragment {
                        id: component.fragment_id.clone(),
                        inventoryable: false,
                        route_base: queued.route_base.clone(),
                    });
                }
                Some(Fragment::Route(route_frag)) => {
                    let is_selected = matched_route
                        .as_ref()
                        .is_some_and(|(best_key, _)| best_key == route_frag.fragment_id.as_str());
                    if is_selected && !route_frag.fragment_id.is_empty() {
                        if let Some((_, ref rm)) = matched_route {
                            chain.push(RouteChainEntry {
                                component: route_frag.fragment_id.clone(),
                                path: route_frag.path.clone(),
                                params: rm.params.clone(),
                                exact: route_frag.exact,
                                allowed_query: route_frag.allowed_query.clone(),
                                keep_alive: route_frag.keep_alive,
                                cache_tags: route_frag.cache_tags.clone(),
                                invalidates: route_frag.invalidates.clone(),
                                pending_component: route_frag.pending_component.clone(),
                                error_component: route_frag.error_component.clone(),
                            });

                            let child_route_base = route_matcher::compute_route_base(
                                request_path,
                                rm.consumed_segments,
                            );

                            stack.push(QueuedFragment {
                                id: route_frag.fragment_id.clone(),
                                inventoryable: false,
                                route_base: child_route_base.clone(),
                            });

                            // Walk nested children iteratively
                            collect_chain_from_children(
                                &route_frag.children,
                                &child_route_base,
                                &mut chain,
                                &mut ChildWalkCtx {
                                    request_path,
                                    protocol,
                                    cache,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    chain
}

/// Iteratively collect chain entries from nested route children.
fn collect_chain_from_children(
    children: &[WebUIFragmentRoute],
    route_base: &str,
    chain: &mut Vec<RouteChainEntry>,
    ctx: &mut ChildWalkCtx<'_>,
) {
    let mut pending: Vec<(&[WebUIFragmentRoute], String)> =
        vec![(children, route_base.to_string())];

    while let Some((current, base)) = pending.pop() {
        if let Some((idx, rm)) =
            select_best_child_route(current, ctx.request_path, &base, ctx.cache)
        {
            let matched = &current[idx];
            chain.push(RouteChainEntry {
                component: matched.fragment_id.clone(),
                path: matched.path.clone(),
                params: rm.params,
                exact: matched.exact,
                allowed_query: matched.allowed_query.clone(),
                keep_alive: matched.keep_alive,
                cache_tags: matched.cache_tags.clone(),
                invalidates: matched.invalidates.clone(),
                pending_component: matched.pending_component.clone(),
                error_component: matched.error_component.clone(),
            });
            if !matched.children.is_empty() {
                let child_base =
                    route_matcher::compute_route_base(ctx.request_path, rm.consumed_segments);
                pending.push((&matched.children, child_base));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use webui_protocol::{FragmentList, WebUIFragment, WebUiFragmentRoute};

    #[test]
    fn prepared_protocol_decodes_once_and_exposes_tokens() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Prepared</p>")],
            },
        );
        let protocol = WebUIProtocol::with_tokens(fragments, vec!["colorBrand".to_string()]);
        let bytes = protocol.to_protobuf().unwrap();
        let prepared = PreparedProtocol::from_protobuf(&bytes).unwrap();

        assert!(prepared.protocol().fragments.contains_key("index.html"));
        assert_eq!(prepared.tokens(), ["colorBrand"]);
    }

    #[test]
    fn prepared_partial_matches_direct_index_path() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/", "home-page")],
            },
        );
        fragments.insert(
            "home-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Home</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let mut direct_index = ProtocolIndex::new(&protocol);
        let direct =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut direct_index).unwrap();
        let prepared = PreparedProtocol::new(protocol);
        let cached = render_partial_metadata_prepared(&prepared, "index.html", "/", "").unwrap();

        assert_eq!(cached, direct);
    }

    #[test]
    fn prepared_partial_supports_concurrent_metadata_reads() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/", "home-page")],
            },
        );
        fragments.insert(
            "home-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Home</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "home-page".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>Home</p>"}"#.to_string(),
                ..Default::default()
            },
        );

        let prepared = Arc::new(PreparedProtocol::new(protocol));
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let prepared = Arc::clone(&prepared);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..50 {
                        let response =
                            render_partial_metadata_prepared(&prepared, "index.html", "/", "")
                                .unwrap();
                        assert_eq!(response["templates"]["home-page"]["h"], "<p>Home</p>");
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    fn prepared_partial_protocol(hydration_keys: &[&str]) -> PreparedProtocol {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route("/", "home-page")],
            },
        );
        fragments.insert(
            "home-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Home</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "home-page".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>Home</p>"}"#.to_string(),
                hydration_mode: webui_protocol::StateProjectionMode::Keys as i32,
                hydration_keys: hydration_keys
                    .iter()
                    .map(|key| (*key).to_string())
                    .collect(),
                navigation_mode: webui_protocol::StateProjectionMode::Keys as i32,
                navigation_keys: hydration_keys
                    .iter()
                    .map(|key| (*key).to_string())
                    .collect(),
                ..Default::default()
            },
        );
        PreparedProtocol::new(protocol)
    }

    fn prepared_full_state_partial_protocol() -> PreparedProtocol {
        let mut prepared = prepared_partial_protocol(&[]);
        let protocol = &mut prepared.protocol;
        if let Some(component) = protocol.components.get_mut("home-page") {
            component.navigation_mode = webui_protocol::StateProjectionMode::All as i32;
            component.navigation_keys.clear();
        }
        prepared
    }

    #[test]
    fn partial_state_serialization_preserves_validated_raw_json() {
        let prepared = prepared_partial_protocol(&["value"]);
        let output = render_partial_prepared(
            &prepared,
            r#"{"serverOnly":"drop","value":1e2}"#,
            "index.html",
            "/",
            "",
        )
        .unwrap();
        assert!(output.contains(r#""state":{"value":1e2}"#));
        assert!(!output.contains("serverOnly"));

        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["state"]["value"], 100.0);
    }

    #[test]
    fn uncertain_partial_surface_preserves_complete_raw_state() {
        let prepared = prepared_full_state_partial_protocol();
        let output = render_partial_prepared(
            &prepared,
            r#"{"serverOnly":"keep","value":1e2}"#,
            "index.html",
            "/",
            "",
        )
        .unwrap();
        assert!(output.contains(r#""state":{"serverOnly":"keep","value":1e2}"#));
    }

    #[test]
    fn partial_state_serialization_rejects_invalid_json() {
        let prepared = prepared_partial_protocol(&[]);
        let error = render_partial_prepared(&prepared, r#"{"broken":"#, "index.html", "/", "")
            .expect_err("invalid state JSON must fail");
        assert!(error.to_string().contains("invalid state JSON"));
    }

    #[test]
    fn partial_state_serialization_emits_empty_state_without_client_components() {
        let prepared = PreparedProtocol::new(WebUIProtocol::default());
        let output =
            render_partial_prepared(&prepared, r#"{"serverOnly":"drop"}"#, "index.html", "/", "")
                .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["state"], serde_json::json!({}));
    }

    #[test]
    fn partial_state_serialization_validates_selected_and_skipped_numbers() {
        let prepared = prepared_partial_protocol(&["value"]);
        for state in [r#"{"value":1e9999}"#, r#"{"serverOnly":1e9999,"value":1}"#] {
            let error = render_partial_prepared(&prepared, state, "index.html", "/", "")
                .expect_err("out-of-range state numbers must fail");
            assert!(error.to_string().contains("invalid state JSON"));
        }
    }

    #[test]
    fn partial_state_serialization_uses_last_duplicate_and_decodes_keys() {
        let prepared = prepared_partial_protocol(&["value"]);
        let output = render_partial_prepared(
            &prepared,
            r#"{"value":1,"va\u006cue":2}"#,
            "index.html",
            "/",
            "",
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["state"], serde_json::json!({"value": 2}));
    }

    #[test]
    fn parsed_partial_state_projects_without_cloning_server_only_values() {
        let prepared = prepared_partial_protocol(&["value"]);
        let response = render_partial(
            prepared.protocol(),
            serde_json::json!({"serverOnly": [1, 2, 3], "value": "kept"}),
            "index.html",
            "/",
            "",
        )
        .unwrap();
        assert_eq!(response["state"], serde_json::json!({"value": "kept"}));
    }

    #[test]
    fn uncertain_partial_surface_preserves_complete_parsed_state() {
        let prepared = prepared_full_state_partial_protocol();
        let state = serde_json::json!({"serverOnly": [1, 2, 3], "value": "kept"});
        let response =
            render_partial(prepared.protocol(), state.clone(), "index.html", "/", "").unwrap();
        assert_eq!(response["state"], state);
    }

    #[test]
    fn partial_state_validation_matches_value_number_limits() {
        for json in [
            r#"{"value":1e9999}"#,
            r#"{"value":-1e9999}"#,
            r#"{"value":1e308}"#,
            r#"{"value":18446744073709551615}"#,
            r#"{"value":18446744073709551616}"#,
        ] {
            assert_eq!(
                validate_json(json).is_ok(),
                serde_json::from_str::<Value>(json).is_ok(),
                "validation mismatch for {json}"
            );
        }
    }

    #[test]
    fn test_parse_encode_inventory_roundtrip() {
        let original = vec![0xABu8, 0xCD, 0xEF, 0x01];
        let hex = encode_inventory(&original);
        let decoded = parse_inventory(&hex).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_parse_inventory_empty() {
        let result = parse_inventory("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_inventory_rejects_odd_length() {
        let result = parse_inventory("abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_inventory_rejects_invalid_hex() {
        let result = parse_inventory("zz");
        assert!(result.is_err());
    }

    #[test]
    fn test_bitfield_set_and_check() {
        let mut inv = vec![0u8; 4];
        set_component(&mut inv, 0);
        set_component(&mut inv, 7);
        set_component(&mut inv, 8);
        set_component(&mut inv, 15);
        assert!(has_component(&inv, 0));
        assert!(has_component(&inv, 7));
        assert!(has_component(&inv, 8));
        assert!(has_component(&inv, 15));
        assert!(!has_component(&inv, 1));
        assert!(!has_component(&inv, 9));
    }

    #[test]
    fn test_filter_needed_components_no_false_positives() {
        // With sequential indices, no two components share a bit
        let mut index = HashMap::new();
        index.insert("email-message".to_string(), 0);
        index.insert("o-button".to_string(), 1);
        index.insert("o-avatar".to_string(), 2);

        let names = vec!["email-message".to_string(), "o-button".to_string()];

        // Only o-button (index 1) is loaded — bit 1 set
        let mut inv = vec![0u8; 1];
        set_component(&mut inv, 1); // o-button
        let inv_hex = encode_inventory(&inv);

        let (needed, _) = filter_needed_components(&names, &inv_hex, &index).unwrap();
        assert_eq!(needed.len(), 1);
        assert!(
            needed.contains(&"email-message".to_string()),
            "email-message must be needed: {needed:?}"
        );
    }

    #[test]
    fn test_get_needed_components_empty_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("card")],
            },
        );

        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp_index = build_component_index(&protocol);
        let (needed, _inv) =
            get_needed_components(&protocol, "app-shell", "", &comp_index).unwrap();
        assert!(needed.contains(&"app-shell".to_string()));
        assert!(needed.contains(&"my-card".to_string()));
    }

    #[test]
    fn test_get_needed_components_skips_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("card")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("app-shell".to_string())
            .or_default()
            .template = "<t></t>".to_string();
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .template = "<t></t>".to_string();

        let comp_index = build_component_index(&protocol);
        let (_needed, inv_hex) =
            get_needed_components(&protocol, "app-shell", "", &comp_index).unwrap();
        let (needed2, _) =
            get_needed_components(&protocol, "app-shell", &inv_hex, &comp_index).unwrap();
        assert!(needed2.is_empty());
    }

    #[test]
    fn test_get_needed_components_skips_control_fragments() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("showNav"),
                        "if-shell",
                    ),
                    WebUIFragment::for_loop("item", "items", "for-items"),
                    WebUIFragment::attribute_template("title", "attr-title"),
                ],
            },
        );
        fragments.insert(
            "if-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-category-nav")],
            },
        );
        fragments.insert(
            "for-items".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-card")],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Products")],
            },
        );
        fragments.insert(
            "mp-category-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-product-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<article></article>")],
            },
        );

        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp_index = build_component_index(&protocol);
        let needed: HashSet<String> =
            get_needed_components(&protocol, "app-shell", "", &comp_index)
                .unwrap()
                .0
                .into_iter()
                .collect();

        assert!(needed.contains("app-shell"));
        assert!(needed.contains("mp-category-nav"));
        assert!(needed.contains("mp-product-card"));
        assert!(!needed.contains("if-shell"));
        assert!(!needed.contains("for-items"));
        assert!(!needed.contains("attr-title"));
    }

    #[test]
    fn test_get_needed_components_for_request_includes_shell_and_matched_route_chain() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mp-category-nav"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/search/:category".into(),
                        fragment_id: "mp-search-page".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".into(),
                        fragment_id: "mp-product-page".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                ],
            },
        );
        fragments.insert(
            "mp-category-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>grid</div>")],
            },
        );
        fragments.insert(
            "mp-product-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-detail")],
            },
        );
        fragments.insert(
            "mp-product-detail".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>detail</div>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("mp-search-page".to_string())
            .or_default()
            .template = "<mp-search-page></mp-search-page>".to_string();
        protocol
            .components
            .entry("mp-product-page".to_string())
            .or_default()
            .template = "<mp-product-page></mp-product-page>".to_string();

        let comp_index = build_component_index(&protocol);
        let (needed, _inv) = get_needed_components_for_request(
            &protocol,
            "index.html",
            "/search/shirts",
            "",
            &comp_index,
        )
        .unwrap();
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-category-nav"));
        assert!(needed.contains("mp-search-page"));
        assert!(needed.contains("mp-product-grid"));
        assert!(!needed.contains("mp-product-page"));
        assert!(!needed.contains("mp-product-detail"));
    }

    #[test]
    fn test_get_needed_components_for_request_follows_nested_route_chain() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/account/*rest".into(),
                    fragment_id: "mp-account-shell".into(),
                    exact: false,
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-account-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mp-account-nav"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/profile".into(),
                        fragment_id: "mp-profile-page".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/account/orders/:id".into(),
                        fragment_id: "mp-order-page".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                ],
            },
        );
        fragments.insert(
            "mp-account-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-profile-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<profile></profile>")],
            },
        );
        fragments.insert(
            "mp-order-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-order-detail")],
            },
        );
        fragments.insert(
            "mp-order-detail".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<detail></detail>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("mp-account-shell".to_string())
            .or_default()
            .template = "<mp-account-shell></mp-account-shell>".to_string();
        protocol
            .components
            .entry("mp-profile-page".to_string())
            .or_default()
            .template = "<mp-profile-page></mp-profile-page>".to_string();
        protocol
            .components
            .entry("mp-order-page".to_string())
            .or_default()
            .template = "<mp-order-page></mp-order-page>".to_string();

        let comp_index = build_component_index(&protocol);
        let (needed, _inv) = get_needed_components_for_request(
            &protocol,
            "index.html",
            "/account/orders/42",
            "",
            &comp_index,
        )
        .unwrap();
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-account-shell"));
        assert!(needed.contains("mp-account-nav"));
        assert!(needed.contains("mp-order-page"));
        assert!(needed.contains("mp-order-detail"));
        assert!(!needed.contains("mp-profile-page"));
    }

    #[test]
    fn test_get_needed_components_for_request_over_ships_if_and_loop_within_matched_route() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search".into(),
                    fragment_id: "mp-search-page".into(),
                    exact: true,
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("showFilters"),
                        "if-filters",
                    ),
                    WebUIFragment::for_loop("item", "items", "item-loop"),
                ],
            },
        );
        fragments.insert(
            "if-filters".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-filter-panel")],
            },
        );
        fragments.insert(
            "item-loop".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-item-card")],
            },
        );
        fragments.insert(
            "mp-filter-panel".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<filters></filters>")],
            },
        );
        fragments.insert(
            "mp-item-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<item></item>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("mp-search-page".to_string())
            .or_default()
            .template = "<mp-search-page></mp-search-page>".to_string();

        let comp_index = build_component_index(&protocol);
        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/search", "", &comp_index)
                .unwrap();
        let needed: HashSet<String> = needed.into_iter().collect();

        assert!(needed.contains("mp-app"));
        assert!(needed.contains("mp-search-page"));
        assert!(needed.contains("mp-filter-panel"));
        assert!(needed.contains("mp-item-card"));
    }

    #[test]
    fn test_get_needed_components_for_request_respects_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search".into(),
                    fragment_id: "mp-search-page".into(),
                    exact: true,
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "mp-search-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<grid></grid>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        for name in ["mp-app", "mp-search-page", "mp-product-grid"] {
            protocol
                .components
                .entry(name.to_string())
                .or_default()
                .template = format!("<f-template id=\"{name}\"></f-template>");
        }

        let comp_index = build_component_index(&protocol);
        let (_needed, inventory) =
            get_needed_components_for_request(&protocol, "index.html", "/search", "", &comp_index)
                .unwrap();
        let (needed_again, _) = get_needed_components_for_request(
            &protocol,
            "index.html",
            "/search",
            &inventory,
            &comp_index,
        )
        .unwrap();
        assert!(needed_again.is_empty());
    }

    #[test]
    fn test_inventory_includes_boundary_components_for_unmatched_sibling_routes() {
        // Regression: pending/error components for routes that are NOT in the
        // matched chain at the current request path must still be inventoried.
        // The client renders pending UI before a sibling route's partial
        // resolves and error UI when it fails, so neither can be delivered by
        // the target route's own fetch — both must already be on the client.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "route-shell".into(),
                    children: vec![
                        WebUiFragmentRoute {
                            path: "slow".into(),
                            fragment_id: "page-slow".into(),
                            exact: true,
                            pending_component: "loading-skeleton".into(),
                            ..Default::default()
                        },
                        WebUiFragmentRoute {
                            path: "failing".into(),
                            fragment_id: "page-failing".into(),
                            exact: true,
                            error_component: "error-display".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                })],
            },
        );
        for id in [
            "route-shell",
            "page-slow",
            "page-failing",
            "loading-skeleton",
            "error-display",
        ] {
            fragments.insert(
                id.to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<x></x>")],
                },
            );
        }

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        for id in [
            "route-shell",
            "page-slow",
            "page-failing",
            "loading-skeleton",
            "error-display",
        ] {
            protocol
                .components
                .entry(id.to_string())
                .or_default()
                .template = format!("<f-template id=\"{id}\"></f-template>");
        }

        // Request the root path: neither "slow" nor "failing" is in the matched chain.
        let reachable = collect_reachable_components_for_request(
            &protocol,
            "index.html",
            "/",
            &mut CompiledRouteCache::new(),
        );
        assert!(
            reachable.contains("loading-skeleton"),
            "pending component of an unmatched sibling route must be inventoried: {reachable:?}"
        );
        assert!(
            reachable.contains("error-display"),
            "error component of an unmatched sibling route must be inventoried: {reachable:?}"
        );
    }

    #[test]
    fn test_render_partial_separates_module_styles_from_templates() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        component.css = ".page{color:red}".to_string();

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut index).unwrap();
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        assert_eq!(styles.len(), 1);
        let style_html = styles[0].as_str().unwrap_or_default();
        assert!(
            style_html.starts_with(r#"<script type="importmap""#)
                && style_html.contains(r#""my-page":"data:text/css,"#),
            "module style entry should be an importmap registering the component specifier: {style_html}"
        );
        assert!(
            style_html.contains(".page{color:red}"),
            "module style entry should contain the CSS content verbatim inside the data: URI: {style_html}"
        );

        // templates should contain only JSON-safe metadata
        let templates = partial["templates"]
            .as_object()
            .expect("templates should be an object");
        assert_eq!(templates.len(), 1);
        let template = templates.get("my-page").expect("my-page template");
        assert_eq!(template["h"], "<p>page</p>");
    }

    #[test]
    fn test_render_partial_link_strategy_has_empty_template_styles() {
        // Link-strategy components have css_href but no css content.
        // templateStyles should be empty; templates should contain metadata.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        component.css_href = "my-page.css".to_string();
        // css is empty — Link strategy stores href, not content

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut index).unwrap();
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial["templates"]
            .as_object()
            .expect("templates should be an object");

        assert!(
            styles.is_empty(),
            "Link strategy should produce empty templateStyles: {styles:?}"
        );
        assert_eq!(templates.len(), 1, "should include template metadata");
    }

    #[test]
    fn test_render_partial_style_strategy_has_empty_template_styles() {
        // Style-strategy components have CSS embedded in the template HTML
        // (as <style>...</style>), not in component.css.
        // templateStyles should be empty; templates should contain metadata.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        // Style strategy: CSS is inside the template HTML, not in component.css
        component.template_json = r#"{"h":"<style>.p{color:red}</style><p>page</p>"}"#.to_string();
        // css is empty for Style strategy

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut index).unwrap();
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial["templates"]
            .as_object()
            .expect("templates should be an object");

        assert!(
            styles.is_empty(),
            "Style strategy should produce empty templateStyles: {styles:?}"
        );
        assert_eq!(templates.len(), 1, "should include template metadata");
        assert!(
            templates
                .get("my-page")
                .and_then(|template| template["h"].as_str())
                .unwrap_or_default()
                .contains("<style>"),
            "Style strategy template should contain inline <style> tag"
        );
    }

    #[test]
    fn test_render_partial_empty_styles_for_no_css_components() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        // No CSS — simulates Link or Style mode

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut index).unwrap();
        let styles = partial["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        assert!(
            styles.is_empty(),
            "templateStyles should be empty when components have no CSS"
        );
    }

    #[test]
    fn test_render_partial_sends_styles_even_when_templates_filtered_by_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-page")],
            },
        );
        fragments.insert(
            "my-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>page</p>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let component = protocol
            .components
            .entry("my-page".to_string())
            .or_default();
        component.template_json = r#"{"h":"<p>page</p>"}"#.to_string();
        component.css = ".page{color:red}".to_string();

        let mut index = ProtocolIndex::new(&protocol);
        // First call to establish inventory
        let partial1 =
            render_partial_metadata(&protocol, "index.html", "/", "", &mut index).unwrap();
        let inv = partial1["inventory"].as_str().unwrap_or_default();
        assert!(!inv.is_empty());

        // Second call with the inventory — both templates and styles should be empty
        // because the inventory covers this component.  The SSR handler emits all
        // module style definitions in <head> for inventoried components, so the
        // client already has the CSS definition.
        let partial2 =
            render_partial_metadata(&protocol, "index.html", "/", inv, &mut index).unwrap();
        let styles = partial2["templateStyles"]
            .as_array()
            .expect("templateStyles should be an array");
        let templates = partial2["templates"]
            .as_object()
            .expect("templates should be an object");

        assert!(
            templates.is_empty(),
            "templates should be empty when inventory is full"
        );
        assert!(
            styles.is_empty(),
            "module styles should be empty when inventory is full — SSR already placed them"
        );
    }

    #[test]
    fn test_non_route_siblings_included_in_needed_components() {
        // Reproduces the commerce app layout: mp-app has both route children
        // (via outlet) AND non-route sibling components (mp-cart-panel, mp-footer).
        // All siblings must be in the needed set so their module CSS definitions
        // are emitted in <head> during SSR.
        let mut fragments = HashMap::new();

        // Entry → app-shell component
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("app-shell")],
            },
        );

        // app-shell has: navbar (component), route (outlet), cart-panel (component)
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("my-navbar"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/about".into(),
                        fragment_id: "page-about".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::component("cart-panel"),
                ],
            },
        );

        // Leaf fragments
        fragments.insert(
            "my-navbar".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav/>")],
            },
        );
        fragments.insert(
            "page-about".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>About</h1>")],
            },
        );
        fragments.insert(
            "cart-panel".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<aside>Cart</aside>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        for name in ["app-shell", "my-navbar", "page-about", "cart-panel"] {
            let comp = protocol.components.entry(name.to_string()).or_default();
            comp.template_json = format!(r#"{{"h":"<div class=\"{name}\"></div>"}}"#);
            comp.css = format!(".{name}{{display:block}}");
        }

        let comp_index = build_component_index(&protocol);
        let (needed, _inv) =
            get_needed_components_for_request(&protocol, "index.html", "/about", "", &comp_index)
                .unwrap();

        assert!(
            needed.contains(&"app-shell".to_string()),
            "app-shell should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"my-navbar".to_string()),
            "my-navbar (non-route sibling) should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"page-about".to_string()),
            "page-about (active route) should be needed: {needed:?}"
        );
        assert!(
            needed.contains(&"cart-panel".to_string()),
            "cart-panel (non-route sibling) should be needed: {needed:?}"
        );
    }

    // ── RouteChainEntry.to_json + allowed_query tests ────────────────

    #[test]
    fn test_chain_entry_to_json_includes_allowed_query() {
        let entry = RouteChainEntry {
            component: "compose-page".into(),
            path: "compose".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: "action,to,subject".into(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert_eq!(json["allowedQuery"], "action,to,subject");
    }

    #[test]
    fn test_chain_entry_to_json_omits_empty_allowed_query() {
        let entry = RouteChainEntry {
            component: "home-page".into(),
            path: "/".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert!(
            json.get("allowedQuery").is_none(),
            "empty allowed_query should not appear in JSON"
        );
    }

    #[test]
    fn test_collect_route_chain_carries_allowed_query() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "compose".into(),
                        fragment_id: "compose-page".into(),
                        exact: true,
                        allowed_query: "action,to".into(),
                        keep_alive: false,
                        ..Default::default()
                    }],
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "compose-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Compose</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol
            .components
            .entry("app-shell".into())
            .or_default()
            .css_href = "/app-shell.css".into();
        protocol
            .components
            .entry("compose-page".into())
            .or_default()
            .template_json = r#"{"h":"<p>Compose</p>"}"#.into();

        let chain = collect_route_chain(
            &protocol,
            "index.html",
            "/compose",
            &mut CompiledRouteCache::new(),
        );
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].component, "app-shell");
        assert!(chain[0].allowed_query.is_empty());
        assert_eq!(chain[1].component, "compose-page");
        assert_eq!(chain[1].allowed_query, "action,to");
    }

    #[test]
    fn scriptless_partial_uses_navigation_keys_without_bootstrap_keys() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/items".into(),
                    fragment_id: "items-page".into(),
                    exact: true,
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "items-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Items</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "items-page".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>Items</p>","th":1}"#.into(),
                navigation_mode: webui_protocol::StateProjectionMode::Keys as i32,
                navigation_keys: vec!["items".into(), "title".into()],
                ..Default::default()
            },
        );

        let partial = render_partial(
            &protocol,
            serde_json::json!({
                "items": [1, 2],
                "serverOnly": "secret",
                "title": "Catalog",
            }),
            "index.html",
            "/items",
            "",
        )
        .unwrap();

        assert_eq!(
            partial["state"],
            serde_json::json!({
                "items": [1, 2],
                "title": "Catalog",
            })
        );
    }

    #[test]
    fn partial_requires_explicit_navigation_keys() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("authored-card")],
            },
        );
        fragments.insert(
            "authored-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Card</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "authored-card".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: webui_protocol::StateProjectionMode::Keys as i32,
                hydration_keys: vec!["title".into()],
                ..Default::default()
            },
        );

        let partial = render_partial(
            &protocol,
            serde_json::json!({
                "serverOnly": "secret",
                "title": "Authored",
            }),
            "index.html",
            "/",
            "",
        )
        .unwrap();

        assert_eq!(partial["state"], serde_json::json!({}));
    }

    #[test]
    fn fully_static_scriptless_partial_emits_empty_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("static-card")],
            },
        );
        fragments.insert(
            "static-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Static</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.components.insert(
            "static-card".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>Static</p>","th":1}"#.into(),
                ..Default::default()
            },
        );

        let partial = render_partial(
            &protocol,
            serde_json::json!({ "serverOnly": "secret" }),
            "index.html",
            "/",
            "",
        )
        .unwrap();

        assert_eq!(partial["state"], serde_json::json!({}));
    }

    #[test]
    fn test_render_component_templates_returns_template_and_css() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "settings-dialog".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div class='dialog'>Settings</div>")],
            },
        );
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("settings-dialog".to_string())
            .or_default();
        comp.template_json = r#"{"h":"<div>Settings</div>"}"#.to_string();
        comp.css = ".dialog{position:fixed}".to_string();

        let mut index = ProtocolIndex::new(&protocol);
        let result =
            render_component_templates(&protocol, &["settings-dialog"], "", &mut index).unwrap();
        let templates = result["templates"].as_object().expect("templates object");
        let styles = result["templateStyles"].as_array().expect("styles array");

        assert_eq!(templates.len(), 1);
        assert_eq!(templates["settings-dialog"]["h"], "<div>Settings</div>");
        assert_eq!(styles.len(), 1);
        let style_html = styles[0].as_str().unwrap();
        assert!(
            style_html.starts_with(r#"<script type="importmap""#)
                && style_html.contains(r#""settings-dialog":"data:text/css,"#),
            "templateStyles entry should be an importmap registering settings-dialog: {style_html}"
        );
        // CSS content is embedded inside the data: URI verbatim — `{`, `}`,
        // `\` are not in the percent-encode set.
        assert!(
            style_html.contains(".dialog{position:fixed}"),
            "templateStyles entry should contain the CSS content verbatim: {style_html}"
        );
        assert!(
            index
                .template_metadata_cache
                .contains_key("settings-dialog"),
            "parsed static template metadata should be cached in ProtocolIndex"
        );
    }

    #[test]
    fn test_render_component_templates_respects_inventory() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "my-dialog".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Dialog</div>")],
            },
        );
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        let comp = protocol
            .components
            .entry("my-dialog".to_string())
            .or_default();
        comp.template_json = r#"{"h":"<div>Dialog</div>"}"#.to_string();
        comp.css = ".d{color:red}".to_string();

        let mut index = ProtocolIndex::new(&protocol);
        // First call: no inventory → should return the component
        let result1 =
            render_component_templates(&protocol, &["my-dialog"], "", &mut index).unwrap();
        let inv = result1["inventory"].as_str().expect("inventory string");
        assert_eq!(result1["templates"].as_object().unwrap().len(), 1);

        // Second call with inventory → component already loaded, should skip
        let result2 =
            render_component_templates(&protocol, &["my-dialog"], inv, &mut index).unwrap();
        assert_eq!(result2["templates"].as_object().unwrap().len(), 0);
        assert_eq!(result2["templateStyles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_render_component_templates_unknown_component_returns_empty() {
        let fragments = HashMap::new();
        let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());

        let mut index = ProtocolIndex::new(&protocol);
        let result =
            render_component_templates(&protocol, &["nonexistent-widget"], "", &mut index).unwrap();
        assert_eq!(result["templates"].as_object().unwrap().len(), 0);
        assert_eq!(result["templateStyles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_render_partial_includes_sibling_boundary_templates() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![
                        WebUiFragmentRoute {
                            path: "inbox".into(),
                            fragment_id: "inbox-page".into(),
                            exact: true,
                            pending_component: "inbox-loading".into(),
                            ..Default::default()
                        },
                        WebUiFragmentRoute {
                            path: "settings".into(),
                            fragment_id: "settings-page".into(),
                            exact: true,
                            pending_component: "settings-loading".into(),
                            error_component: "settings-error".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                })],
            },
        );

        for name in [
            "app-shell",
            "inbox-page",
            "inbox-loading",
            "settings-page",
            "settings-loading",
            "settings-error",
        ] {
            fragments.insert(
                name.to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw(format!("<p>{name}</p>"))],
                },
            );
        }

        let mut protocol = WebUIProtocol::new(fragments);
        for name in [
            "app-shell",
            "inbox-page",
            "inbox-loading",
            "settings-page",
            "settings-loading",
            "settings-error",
        ] {
            protocol
                .components
                .entry(name.to_string())
                .or_default()
                .template_json = format!(r#"{{"h":"<p>{name}</p>"}}"#);
        }

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/inbox", "", &mut index).unwrap();
        let templates = partial["templates"].as_object().unwrap();

        assert!(
            templates.contains_key("settings-loading"),
            "sibling pending template should be preloaded for route transitions: {partial}"
        );
        assert!(
            templates.contains_key("settings-error"),
            "sibling error template should be preloaded for failed route transitions: {partial}"
        );
        let chain = partial["chain"].as_array().unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0]["component"], "app-shell");
        assert_eq!(chain[1]["component"], "inbox-page");
    }

    // ── resolve_tag_templates tests ──────────────────────────────────

    #[test]
    fn test_resolve_tag_templates_no_placeholders() {
        let tags = vec!["inbox".to_string(), "counts".to_string()];
        let params = HashMap::new();
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["inbox", "counts"]);
    }

    #[test]
    fn test_resolve_tag_templates_with_params() {
        let tags = vec!["thread:{threadId}".to_string(), "inbox".to_string()];
        let mut params = HashMap::new();
        params.insert("threadId".to_string(), "42".to_string());
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["thread:42", "inbox"]);
    }

    #[test]
    fn test_resolve_tag_templates_multiple_params() {
        let tags = vec!["folder:{folderId}:user:{userId}".to_string()];
        let mut params = HashMap::new();
        params.insert("folderId".to_string(), "drafts".to_string());
        params.insert("userId".to_string(), "abc".to_string());
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["folder:drafts:user:abc"]);
    }

    #[test]
    fn test_resolve_tag_templates_missing_param_left_unresolved() {
        let tags = vec!["thread:{threadId}".to_string()];
        let params = HashMap::new();
        let resolved = resolve_tag_templates(&tags, &params);
        assert_eq!(resolved, vec!["thread:{threadId}"]);
    }

    // ── Chain entry to_json with new fields ───────────────────────

    #[test]
    fn test_chain_entry_to_json_includes_new_fields() {
        let entry = RouteChainEntry {
            component: "mail-thread".into(),
            path: "email/:threadId".into(),
            params: {
                let mut p = HashMap::new();
                p.insert("threadId".to_string(), "42".to_string());
                p
            },
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: vec!["thread:42".to_string()],
            invalidates: vec!["inbox".to_string(), "counts".to_string()],
            pending_component: "mail-skeleton".into(),
            error_component: "error-page".into(),
        };
        let json = entry.to_json();
        assert_eq!(json["pendingComponent"], "mail-skeleton");
        assert_eq!(json["errorComponent"], "error-page");
        let inv = json["invalidates"].as_array().unwrap();
        assert_eq!(inv.len(), 2);
        assert_eq!(inv[0], "inbox");
        assert_eq!(inv[1], "counts");
    }

    #[test]
    fn test_chain_entry_to_json_omits_empty_new_fields() {
        let entry = RouteChainEntry {
            component: "home-page".into(),
            path: "/".into(),
            params: HashMap::new(),
            exact: true,
            allowed_query: String::new(),
            keep_alive: false,
            cache_tags: Vec::new(),
            invalidates: Vec::new(),
            pending_component: String::new(),
            error_component: String::new(),
        };
        let json = entry.to_json();
        assert!(json.get("pendingComponent").is_none());
        assert!(json.get("errorComponent").is_none());
        assert!(json.get("invalidates").is_none());
    }

    // ── render_partial with cache tags ────────────────────────────

    #[test]
    fn test_render_partial_includes_resolved_cache_tags() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "email/:threadId".into(),
                        fragment_id: "mail-thread".into(),
                        exact: true,
                        cache_tags: vec!["thread:{threadId}".to_string(), "inbox".to_string()],
                        ..Default::default()
                    }],
                    cache_tags: vec!["folders".to_string()],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "mail-thread".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Thread</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let mut index = ProtocolIndex::new(&protocol);
        let partial =
            render_partial_metadata(&protocol, "index.html", "/email/42", "", &mut index).unwrap();
        let tags = partial["cacheTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            tag_strings.contains(&"folders"),
            "should contain parent tags"
        );
        assert!(
            tag_strings.contains(&"thread:42"),
            "should resolve threadId param to 42"
        );
        assert!(tag_strings.contains(&"inbox"), "should contain static tags");
    }

    // ── render_action_response tests ─────────────────────────────

    #[test]
    fn test_render_action_response_returns_resolved_invalidation_tags() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "compose".into(),
                        fragment_id: "compose-page".into(),
                        exact: true,
                        invalidates: vec![
                            "inbox".to_string(),
                            "sent".to_string(),
                            "counts".to_string(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "compose-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Compose</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let mut index = ProtocolIndex::new(&protocol);
        let result = render_action_response(
            &protocol,
            serde_json::json!({"ok": true}),
            "index.html",
            "/compose",
            &mut index,
        );

        let tags = result["invalidateTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(tag_strings, vec!["inbox", "sent", "counts"]);
        assert_eq!(result["state"]["ok"], true);
    }

    #[test]
    fn test_render_action_response_resolves_param_placeholders() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "email/:threadId/reply".into(),
                        fragment_id: "reply-page".into(),
                        exact: true,
                        invalidates: vec!["thread:{threadId}".to_string(), "inbox".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "reply-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Reply</p>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let mut index = ProtocolIndex::new(&protocol);
        let result = render_action_response(
            &protocol,
            serde_json::json!({}),
            "index.html",
            "/email/42/reply",
            &mut index,
        );

        let tags = result["invalidateTags"].as_array().unwrap();
        let tag_strings: Vec<&str> = tags.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(tag_strings.contains(&"thread:42"));
        assert!(tag_strings.contains(&"inbox"));
    }
}
