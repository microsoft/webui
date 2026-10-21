// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod serialization;

use crate::{lazy, route_handler, HandlerError, Protocol, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serialization::write_record;
use std::collections::HashSet;
use webui_protocol::{lazy_fragment_id, web_ui_fragment::Fragment};

/// Client availability used by lazy-template requests.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LazyOptions {
    /// Loaded template-resource inventory.
    pub inventory: String,
    /// Loaded stylesheet-definition inventory.
    pub style_inventory: String,
    /// Artifact identity associated with the client inventory.
    pub build_id: String,
}

/// A resource record and its acknowledgement, without reparsing the record JSON.
#[derive(Debug, Serialize)]
pub struct PreparedLazyResources {
    /// JSON resource record to flush before waiting for application data.
    pub json: String,
    /// Availability to pass to the terminal state call after flushing `json`.
    pub options: LazyOptions,
}

/// An owner-scoped lazy template request. It does not create a component.
#[derive(Clone, Copy, Debug)]
pub struct LazyRequest<'a> {
    /// The owning template, usually a component tag.
    pub owner: &'a str,
    /// The lazy if's authored id.
    pub id: &'a str,
    /// Loaded client template resources.
    pub inventory: &'a str,
    /// Loaded stylesheet definitions.
    pub style_inventory: &'a str,
    /// Client artifact identity; required with nonempty inventory.
    pub build_id: &'a str,
}

impl<'a> LazyRequest<'a> {
    /// Request a named region without assuming any client resources.
    #[must_use]
    pub fn new(owner: &'a str, id: &'a str) -> Self {
        Self {
            owner,
            id,
            inventory: "",
            style_inventory: "",
            build_id: "",
        }
    }

    /// Borrow client availability from deserialized host options.
    #[must_use]
    pub fn with_options(mut self, options: &'a LazyOptions) -> Self {
        self.inventory = &options.inventory;
        self.style_inventory = &options.style_inventory;
        self.build_id = &options.build_id;
        self
    }
}

/// An actionable lazy-resource request failure.
#[derive(Debug)]
pub struct LazyRequestError {
    /// Stable machine-readable failure code.
    pub code: &'static str,
    /// Plain diagnostic text with a corrective action.
    pub message: String,
}

impl Protocol {
    /// Prepare resource JSON and the small inventory acknowledgement for streaming.
    ///
    /// Hosts can flush `json`, start/await their data provider, then pass `options`
    /// to the state call without parsing or retaining a second metadata tree.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::render_lazy_resources`].
    #[must_use = "flush the resource record and carry its acknowledgement into the state call"]
    pub fn prepare_lazy_resources(
        &self,
        request: LazyRequest<'_>,
    ) -> Result<PreparedLazyResources> {
        let root = validate_request(self, request)?;
        let selected = collect_resources(self, root, None)?;
        let record = prepare_resources(self, root, request, selected)?;
        let options = LazyOptions {
            inventory: record.inventory.clone(),
            style_inventory: record.style_inventory.clone(),
            build_id: self.protocol().build_id.clone(),
        };
        Ok(PreparedLazyResources {
            json: write_record(record, None::<&Value>)?,
            options,
        })
    }

    /// Render a resource-only record for a lazy template body.
    ///
    /// Ordinary block/component dependencies are included; nested lazy
    /// bodies remain independent. This operation loads no application state.
    ///
    /// # Errors
    ///
    /// Rejects unknown regions, incompatible builds, invalid inventories, and
    /// malformed compiled metadata.
    #[must_use = "handle lazy resource errors before sending a response"]
    pub fn render_lazy_resources(&self, request: LazyRequest<'_>) -> Result<String> {
        let root = validate_request(self, request)?;
        let selected = collect_resources(self, root, None)?;
        let record = prepare_resources(self, root, request, selected)?;
        write_record(record, None::<&Value>)
    }

    /// Render a complete lazy response, or the terminal state record of NDJSON.
    ///
    /// State belongs to the existing owner/scope. Inventory never suppresses
    /// state. If the state activates nested lazy bodies, this record adds
    /// their missing resources before the client mounts anything.
    ///
    /// # Errors
    ///
    /// Returns request/protocol errors or an invalid-state error for non-object input.
    #[must_use = "handle lazy response errors before sending a response"]
    pub fn render_lazy_partial(&self, state: &Value, request: LazyRequest<'_>) -> Result<String> {
        let root = validate_request(self, request)?;
        if !state.is_object() {
            return Err(HandlerError::InvalidState(
                "lazy state must be a JSON object".into(),
            ));
        }
        let render = self.render_fragments().resolve(self.protocol());
        let active = self
            .lazy_index()
            .map(|index| {
                lazy::select(lazy::SelectionInput {
                    protocol: self.protocol(),
                    render: &render,
                    index,
                    state,
                    entry: root,
                    request_path: "/",
                    routes: self.route_index(),
                })
            })
            .transpose()?;
        let selected = collect_resources(self, root, active.as_ref())?;
        let selection = crate::collect_navigation_state(self.protocol(), selected.iter().copied());
        let state = serialization::SelectedState {
            value: state,
            selection: &selection,
        };
        let record = prepare_resources(self, root, request, selected)?;
        write_record(record, Some(&state))
    }

    /// Render lazy state from JSON, borrowing projected values when possible.
    ///
    /// State is parsed once only when nested lazy decisions need evaluation;
    /// parsed-state Rust hosts can use [`Self::render_lazy_partial`] directly.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::render_lazy_partial`], including
    /// malformed state JSON.
    #[must_use = "handle lazy response errors before sending a response"]
    pub fn render_lazy_partial_json(
        &self,
        state_json: &str,
        request: LazyRequest<'_>,
    ) -> Result<String> {
        let root = validate_request(self, request)?;
        if self.lazy_index().is_some_and(|index| {
            self.fragment_slot(root)
                .is_some_and(|slot| index.relevant(slot as usize, false))
        }) {
            let state = serde_json::from_str(state_json)
                .map_err(|error| HandlerError::InvalidState(error.to_string()))?;
            return self.render_lazy_partial(&state, request);
        }
        if !state_json.trim_start().starts_with('{') {
            return Err(HandlerError::InvalidState(
                "lazy state must be a JSON object".into(),
            ));
        }
        let selected = collect_resources(self, root, None)?;
        let selection = crate::collect_navigation_state(self.protocol(), selected.iter().copied());
        let state = route_handler::select_raw_state(state_json, &selection)?;
        let record = prepare_resources(self, root, request, selected)?;
        write_record(record, Some(&state))
    }
}

fn validate_request<'a>(protocol: &'a Protocol, request: LazyRequest<'_>) -> Result<&'a str> {
    protocol.ensure_style_metadata()?;
    let document = protocol.protocol();
    if document.build_id.is_empty() {
        return Err(request_error(
            "lazy-missing-build-id",
            "rebuild the protocol with the current compiler before requesting lazy regions",
        ));
    }
    if !request.build_id.is_empty() && request.build_id != document.build_id {
        return Err(request_error(
            "lazy-build-mismatch",
            "the document and protocol are different builds; reload the document",
        ));
    }
    if request.build_id.is_empty()
        && (!request.inventory.is_empty() || !request.style_inventory.is_empty())
    {
        return Err(request_error(
            "lazy-missing-build-id",
            "send the document buildId with its inventory, or request without inventory",
        ));
    }
    if request.owner.len() > 512 || request.id.is_empty() || request.id.len() > 128 {
        return Err(request_error(
            "lazy-invalid-target",
            "use the owning template and an authored lazy id",
        ));
    }
    if request.inventory.len() > protocol.component_index().len().div_ceil(8) * 2
        || request.style_inventory.len() > protocol.style_resource_index().len().div_ceil(8) * 2
    {
        return Err(request_error(
            "lazy-invalid-inventory",
            "inventory exceeds this artifact's resource count; use inventory from the same build",
        ));
    }
    let key = lazy_fragment_id(request.owner, Some(request.id), 0);
    document.client_fragments.get_key_value(&key)
        .filter(|(_, data)| data.entry && data.owner == request.owner)
        .map(|(key, _)| key.as_str())
        .ok_or_else(|| request_error("lazy-not-found", "declare <if condition=\"...\" lazy id=\"...\"> in this owning template and rebuild"))
}

fn collect_resources<'a>(
    protocol: &'a Protocol,
    root: &'a str,
    active: Option<&lazy::LazySelection<'a>>,
) -> Result<Vec<&'a str>> {
    let document = protocol.protocol();
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    let mut selected = Vec::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if document
            .client_template(id)
            .is_some_and(|data| !data.metadata.is_empty() || !data.legacy_html.is_empty())
        {
            selected.push(id);
        }
        if let Some(fragment) = document.client_fragments.get(id) {
            pending.extend(fragment.dependencies.iter().map(String::as_str));
        }
        let Some(list) = document.fragments.get(id) else {
            if document.client_fragments.contains_key(id) {
                continue;
            }
            return Err(HandlerError::MissingFragment(id.to_owned()));
        };
        for fragment in list.fragments.iter().rev() {
            match fragment.fragment.as_ref() {
                Some(Fragment::Component(component)) => pending.push(&component.fragment_id),
                Some(Fragment::ForLoop(repeat)) => pending.push(&repeat.fragment_id),
                Some(Fragment::IfCond(condition))
                    if !condition.lazy
                        || active.is_some_and(|active| {
                            active.contains(condition.fragment_id.as_str())
                        }) =>
                {
                    pending.push(&condition.fragment_id);
                }
                Some(Fragment::Attribute(attribute)) if !attribute.template.is_empty() => {
                    pending.push(&attribute.template)
                }
                _ => {}
            }
        }
    }
    Ok(selected)
}

struct ResourceRecord<'a> {
    protocol: &'a Protocol,
    root: &'a str,
    needed: Vec<&'a str>,
    required: Vec<&'a str>,
    styles: route_handler::BorrowedComponentStyleDelta<'a>,
    inventory: String,
    style_inventory: String,
}

fn prepare_resources<'a>(
    protocol: &'a Protocol,
    root: &'a str,
    request: LazyRequest<'_>,
    selected: Vec<&'a str>,
) -> Result<ResourceRecord<'a>> {
    let mut inventory =
        route_handler::parse_inventory(request.inventory).map_err(invalid_inventory)?;
    let mut style_inventory =
        route_handler::parse_inventory(request.style_inventory).map_err(invalid_inventory)?;
    let styles = route_handler::collect_borrowed_component_style_delta(
        protocol.protocol(),
        selected
            .iter()
            .copied()
            .filter(|id| !protocol.protocol().client_fragments.contains_key(*id)),
        &style_inventory,
        protocol.style_resource_index(),
        &protocol.protocol().style_chunk_index(),
    )?;
    for name in styles.resource_names() {
        if let Some(&index) = protocol.style_resource_index().get(name) {
            route_handler::set_component(&mut style_inventory, index);
        }
    }
    let mut needed = Vec::with_capacity(selected.len());
    for &id in &selected {
        if let Some(&index) = protocol.component_index().get(id) {
            if !route_handler::has_component(&inventory, index) {
                needed.push(id);
            }
            route_handler::set_component(&mut inventory, index);
        } else {
            needed.push(id);
        }
    }
    needed.sort_unstable();
    let mut required = selected;
    required.sort_unstable();
    Ok(ResourceRecord {
        protocol,
        root,
        needed,
        required,
        styles,
        inventory: route_handler::encode_inventory(&inventory),
        style_inventory: route_handler::encode_inventory(&style_inventory),
    })
}

#[cold]
#[inline(never)]
fn request_error(code: &'static str, message: &str) -> HandlerError {
    HandlerError::LazyRequest(Box::new(LazyRequestError {
        code,
        message: message.into(),
    }))
}

#[cold]
#[inline(never)]
fn invalid_inventory(error: HandlerError) -> HandlerError {
    request_error(
        "lazy-invalid-inventory",
        &format!("{error}; use even-length hexadecimal inventories from the matching document"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use webui_protocol::{
        ClientFragmentData, ConditionExpr, FragmentList, StateProjectionMode, WebUIFragment,
        WebUIProtocol,
    };

    fn fixture() -> Protocol {
        let root = lazy_fragment_id("app-shell", Some("settings"), 0);
        let nested = lazy_fragment_id("app-shell", Some("advanced"), 0);
        let mut document = WebUIProtocol::new(HashMap::from([
            (
                root.clone(),
                FragmentList {
                    fragments: vec![WebUIFragment::lazy_if(
                        ConditionExpr::identifier("advanced"),
                        &nested,
                    )],
                    ..Default::default()
                },
            ),
            (nested.clone(), FragmentList::default()),
        ]));
        document.build_id = "fixture".into();
        for (id, name, keys) in [
            (root, "settings", vec!["title", "advanced"]),
            (nested, "advanced", vec!["details"]),
        ] {
            document.client_fragments.insert(
                id,
                ClientFragmentData {
                    owner: "app-shell".into(),
                    name: name.into(),
                    entry: true,
                    template_json: r#"{"h":"<p>Inline</p>","v":3}"#.into(),
                    state_keys: keys.into_iter().map(str::to_owned).collect(),
                    state_mode: Some(StateProjectionMode::Keys as i32),
                    ..Default::default()
                },
            );
        }
        Protocol::new(document)
    }

    #[test]
    fn prepared_resources_return_acknowledgement_without_state_or_nested_lazy_bodies() {
        let protocol = fixture();
        let prepared = protocol
            .prepare_lazy_resources(LazyRequest::new("app-shell", "settings"))
            .unwrap();
        let json: Value = serde_json::from_str(&prepared.json).unwrap();
        assert_eq!(json["type"], "webui-lazy");
        assert!(json.get("state").is_none());
        assert!(json["templates"]["app-shell::lazy:settings"].is_object());
        assert!(json["templates"].get("app-shell::lazy:advanced").is_none());
        assert_eq!(json["inventory"], prepared.options.inventory);
        assert_eq!(json["styleInventory"], prepared.options.style_inventory);
        assert_eq!(json["buildId"], prepared.options.build_id);
        assert_eq!(
            protocol
                .render_lazy_resources(LazyRequest::new("app-shell", "settings"))
                .unwrap(),
            prepared.json
        );
    }

    #[test]
    fn state_record_adds_newly_active_fragments_and_warm_inventory_never_discards_state() {
        let protocol = fixture();
        let prepared = protocol
            .prepare_lazy_resources(LazyRequest::new("app-shell", "settings"))
            .unwrap();
        let state = serde_json::json!({
            "title": "Fresh", "advanced": true, "details": [1, 2], "unused": 42,
            "$webui": { "bodyEnd": "private host markup" },
        });
        let payload: Value = serde_json::from_str(
            &protocol
                .render_lazy_partial(
                    &state,
                    LazyRequest::new("app-shell", "settings").with_options(&prepared.options),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(payload["templates"]
            .get("app-shell::lazy:settings")
            .is_none());
        assert!(payload["templates"]["app-shell::lazy:advanced"].is_object());
        assert_eq!(
            payload["state"],
            serde_json::json!({ "title": "Fresh", "advanced": true, "details": [1, 2] })
        );
        let options = LazyOptions {
            inventory: payload["inventory"].as_str().unwrap().into(),
            style_inventory: payload["styleInventory"].as_str().unwrap().into(),
            build_id: "fixture".into(),
        };
        let warm: Value = serde_json::from_str(
            &protocol
                .render_lazy_partial_json(
                    &state.to_string(),
                    LazyRequest::new("app-shell", "settings").with_options(&options),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(warm["state"], payload["state"]);
        assert!(warm.get("templates").is_none());
        assert!(warm.get("componentStyles").is_none());
    }

    #[test]
    fn raw_json_projection_validates_input_without_sending_unrelated_keys() {
        let protocol = fixture();
        let payload: Value = serde_json::from_str(
            &protocol
                .render_lazy_partial_json(
                    r#"{"details":"fresh","unrelated":[1,2,3]}"#,
                    LazyRequest::new("app-shell", "advanced"),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["state"], serde_json::json!({ "details": "fresh" }));
        for state in ["[]", "{", r#"{"unrelated":1e999}"#] {
            assert!(protocol
                .render_lazy_partial_json(state, LazyRequest::new("app-shell", "advanced"))
                .is_err());
        }
    }

    #[test]
    fn bad_targets_builds_and_inventories_return_distinct_request_errors() {
        let protocol = fixture();
        for (request, code) in [
            (LazyRequest::new("app-shell", "missing"), "lazy-not-found"),
            (
                LazyRequest {
                    build_id: "old",
                    ..LazyRequest::new("app-shell", "settings")
                },
                "lazy-build-mismatch",
            ),
            (
                LazyRequest {
                    inventory: "01",
                    ..LazyRequest::new("app-shell", "settings")
                },
                "lazy-missing-build-id",
            ),
            (
                LazyRequest {
                    inventory: "ffffffff",
                    build_id: "fixture",
                    ..LazyRequest::new("app-shell", "settings")
                },
                "lazy-invalid-inventory",
            ),
        ] {
            let Err(HandlerError::LazyRequest(error)) = protocol.render_lazy_resources(request)
            else {
                panic!("expected typed request failure");
            };
            assert_eq!(error.code, code);
        }
    }
}
