// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time template packs for lazy regions.
//!
//! A pack is the immutable JSON resource record for one lazy region: its
//! template, private and shared blocks, component dependencies, styles,
//! condition functions, and inventory bits. Templates and styles that the
//! owning template already guarantees are omitted, and nested lazy regions
//! are separate packs. Builds publish packs as static files; hosts never
//! render them per request.

mod serialization;

use crate::{route_handler, HandlerError, Protocol, Result};
use std::collections::HashSet;
use webui_protocol::web_ui_fragment::Fragment;

impl Protocol {
    /// Template resources serialized by a lazy region's pack.
    ///
    /// Builds use this to render packs only after every template they contain
    /// has its final metadata.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError::MissingFragment`] for an unknown region and
    /// propagates malformed compiled metadata.
    #[must_use = "order dependent pack rendering with the returned templates"]
    pub fn lazy_pack_templates(&self, region: &str) -> Result<Vec<String>> {
        let pack = Pack::select(self, region)?;
        Ok(pack.needed.iter().map(|id| (*id).to_owned()).collect())
    }

    /// Render the immutable JSON pack for a lazy region.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::lazy_pack_templates`], plus
    /// serialization failures.
    #[must_use = "publish the rendered pack"]
    pub fn render_lazy_pack(&self, region: &str) -> Result<String> {
        serialization::write(&Pack::select(self, region)?)
    }
}

struct Pack<'a> {
    protocol: &'a Protocol,
    root: &'a str,
    /// Every template the region needs, including owner-provided ones.
    required: Vec<&'a str>,
    /// Templates serialized into the pack.
    needed: Vec<&'a str>,
    styles: route_handler::BorrowedComponentStyleDelta<'a>,
    inventory: String,
    style_inventory: String,
}

impl<'a> Pack<'a> {
    fn select(protocol: &'a Protocol, region: &str) -> Result<Self> {
        protocol.ensure_style_metadata()?;
        let document = protocol.protocol();
        let (root, data) = document
            .client_fragments
            .get_key_value(region)
            .filter(|(_, data)| data.entry)
            .ok_or_else(|| HandlerError::MissingFragment(region.to_owned()))?;
        let owner = collect_resources(protocol, &data.owner)?;
        let selected = collect_resources(protocol, root)?;
        let components = protocol.component_index();
        let style_index = protocol.style_resource_index();
        let chunk_index = document.style_chunk_index();
        let is_component = |id: &&str| !document.client_fragments.contains_key(*id);

        // The owner and its eager closure are mounted whenever the region can activate.
        let mut owned = Vec::new();
        for id in &owner {
            if let Some(&index) = components.get(*id) {
                route_handler::set_component(&mut owned, index);
            }
        }
        let owner_styles = route_handler::collect_borrowed_component_style_delta(
            document,
            owner.iter().copied().filter(is_component),
            &[],
            style_index,
            &chunk_index,
        )?;
        let mut owned_styles = Vec::new();
        for name in owner_styles.resource_names() {
            if let Some(&index) = style_index.get(name) {
                route_handler::set_component(&mut owned_styles, index);
            }
        }
        let styles = route_handler::collect_borrowed_component_style_delta(
            document,
            selected.iter().copied().filter(is_component),
            &owned_styles,
            style_index,
            &chunk_index,
        )?;
        let mut style_bits = Vec::new();
        for name in styles.resource_names() {
            if let Some(&index) = style_index.get(name) {
                route_handler::set_component(&mut style_bits, index);
            }
        }
        let mut bits = Vec::new();
        let mut needed = Vec::with_capacity(selected.len());
        for &id in &selected {
            match components.get(id) {
                Some(&index) if route_handler::has_component(&owned, index) => {}
                Some(&index) => {
                    route_handler::set_component(&mut bits, index);
                    needed.push(id);
                }
                None => needed.push(id),
            }
        }
        needed.sort_unstable();
        let mut required = selected;
        required.sort_unstable();
        Ok(Self {
            protocol,
            root: root.as_str(),
            required,
            needed,
            styles,
            inventory: route_handler::encode_inventory(&bits),
            style_inventory: route_handler::encode_inventory(&style_bits),
        })
    }
}

/// Collect a template's ordinary resource closure; nested lazy bodies stay separate.
fn collect_resources<'a>(protocol: &'a Protocol, root: &'a str) -> Result<Vec<&'a str>> {
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
                Some(Fragment::IfCond(condition)) if !condition.lazy => {
                    pending.push(&condition.fragment_id);
                }
                Some(Fragment::Attribute(attribute)) if !attribute.template.is_empty() => {
                    pending.push(&attribute.template);
                }
                _ => {}
            }
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use webui_protocol::{
        lazy_fragment_id, ClientFragmentData, ComponentData, ConditionExpr, FragmentList,
        WebUIFragment, WebUIProtocol,
    };

    fn fixture() -> Protocol {
        let root = lazy_fragment_id("app-shell", 0);
        let nested = lazy_fragment_id("app-shell", 1);
        let mut document = WebUIProtocol::new(HashMap::from([
            (
                "app-shell".to_owned(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::component("shared-badge"),
                        WebUIFragment::lazy_if(ConditionExpr::identifier("open"), &root),
                    ],
                    ..Default::default()
                },
            ),
            (
                root.clone(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::component("shared-badge"),
                        WebUIFragment::component("settings-form"),
                        WebUIFragment::lazy_if(ConditionExpr::identifier("advanced"), &nested),
                    ],
                    ..Default::default()
                },
            ),
            (nested.clone(), FragmentList::default()),
            ("shared-badge".to_owned(), FragmentList::default()),
            ("settings-form".to_owned(), FragmentList::default()),
        ]));
        for tag in ["app-shell", "shared-badge", "settings-form"] {
            document.components.insert(
                tag.to_owned(),
                ComponentData {
                    template_json: format!(r#"{{"h":"<p>{tag}</p>","v":3}}"#),
                    ..Default::default()
                },
            );
        }
        for id in [root, nested] {
            document.client_fragments.insert(
                id,
                ClientFragmentData {
                    owner: "app-shell".into(),
                    entry: true,
                    template_json: r#"{"h":"<p>Inline</p>","v":3}"#.into(),
                    ..Default::default()
                },
            );
        }
        Protocol::new(document)
    }

    #[test]
    fn packs_omit_owner_templates_and_nested_regions() {
        let protocol = fixture();
        let region = lazy_fragment_id("app-shell", 0);
        assert_eq!(
            protocol.lazy_pack_templates(&region).unwrap(),
            [region.as_str(), "settings-form"]
        );
        let pack: Value =
            serde_json::from_str(&protocol.render_lazy_pack(&region).unwrap()).unwrap();
        assert_eq!(pack["type"], "webui-pack");
        assert_eq!(pack["version"], 1);
        assert_eq!(pack["fragment"], region);
        assert!(pack.get("buildId").is_none(), "packs are content-addressed");
        assert!(pack.get("state").is_none());
        assert!(
            pack.get("componentStyles").is_none(),
            "unstyled packs omit the style catalog"
        );
        let templates = pack["templates"].as_object().unwrap();
        assert!(templates.contains_key(&region));
        assert!(templates.contains_key("settings-form"));
        assert!(
            !templates.contains_key("shared-badge"),
            "the owner already mounted it"
        );
        assert!(
            !templates.contains_key("app-shell::lazy:@1"),
            "nested regions are separate packs"
        );
        assert_eq!(
            pack["required"],
            serde_json::json!([region, "settings-form", "shared-badge"])
        );
        let bits = pack["bits"].as_object().unwrap();
        assert_eq!(bits.len(), 2);
        let components = protocol.component_index();
        assert_eq!(bits["settings-form"], components["settings-form"]);
        assert_eq!(bits[&region], components[&region]);
    }

    #[test]
    fn rendering_is_deterministic_and_rejects_unknown_regions() {
        let protocol = fixture();
        let region = lazy_fragment_id("app-shell", 0);
        assert_eq!(
            protocol.render_lazy_pack(&region).unwrap(),
            protocol.render_lazy_pack(&region).unwrap()
        );
        assert!(matches!(
            protocol.render_lazy_pack("app-shell::lazy:@9"),
            Err(HandlerError::MissingFragment(_))
        ));
        assert!(matches!(
            protocol.lazy_pack_templates("shared-badge"),
            Err(HandlerError::MissingFragment(_))
        ));
    }
}
