// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;

use super::{invalid_field, ProjectionAttribute, ProjectionManifestError};
use crate::{
    attrs::attribute_to_camel, web_ui_fragment::Fragment, ConditionExpr, FragmentList,
    WebUIFragment, WebUIFragmentAttribute, WebUIFragmentRecords, WebUIProtocol,
};

/// Compile declared component inputs into existing attribute/property fragments.
///
/// Declarations are consumed at build time; no declaration metadata is added to
/// the runtime protocol. Host attributes keep their HTML names and values.
pub fn lower_component_attributes<'a>(
    protocol: &mut WebUIProtocol,
    declarations: impl Fn(&str) -> Option<&'a BTreeMap<String, ProjectionAttribute>>,
) -> Result<(), ProjectionManifestError> {
    let mut owners: Vec<_> = protocol.fragments.keys().cloned().collect();
    owners.sort_unstable();
    for owner in owners {
        let Some(mut list) = protocol.fragments.remove(&owner) else {
            return Err(invalid_field(
                "an attribute-lowering owner is missing from the fragment graph",
            ));
        };
        lower_list(&owner, &mut list, &mut protocol.fragments, &declarations);
        protocol.fragments.insert(owner, list);
    }
    Ok(())
}

fn lower_list<'a>(
    owner: &str,
    list: &mut FragmentList,
    records: &mut WebUIFragmentRecords,
    declarations: &impl Fn(&str) -> Option<&'a BTreeMap<String, ProjectionAttribute>>,
) {
    let mut start = None;
    let mut additions = Vec::new();
    for index in 0..list.fragments.len() {
        let attributes = match list.fragments[index].fragment.as_ref() {
            Some(Fragment::Attribute(attribute)) if attribute.attr_start => {
                start = Some(index);
                continue;
            }
            Some(Fragment::Component(component)) => declarations(&component.fragment_id),
            _ => continue,
        };
        let Some(start) = start.take() else { continue };
        let Some(attributes) = attributes else {
            continue;
        };
        for position in start..index {
            let Some(Fragment::Attribute(attribute)) = list.fragments[position].fragment.as_mut()
            else {
                continue;
            };
            if attribute.complex {
                continue;
            }
            let Some(declaration) = attributes.get(&attribute.name) else {
                continue;
            };
            if let Some(property) = lower_attribute(attribute, declaration) {
                let mut property = property;
                if !property.raw_value
                    && property.condition_tree.is_none()
                    && property.template.is_empty()
                {
                    let mut id = format!("attr-{owner}-{position}");
                    while records.contains_key(&id) {
                        id.push('_');
                    }
                    records.insert(
                        id.clone(),
                        FragmentList {
                            fragments: vec![WebUIFragment::signal(
                                std::mem::take(&mut property.value),
                                false,
                            )],
                            contains_boundary: false,
                        },
                    );
                    property.template = id;
                }
                additions.push((
                    position,
                    WebUIFragment {
                        fragment: Some(Fragment::Attribute(property)),
                    },
                ));
            }
        }
    }
    if additions.is_empty() {
        return;
    }
    let original = std::mem::take(&mut list.fragments);
    list.fragments = Vec::with_capacity(original.len() + additions.len());
    let mut additions = additions.into_iter().peekable();
    for (index, fragment) in original.into_iter().enumerate() {
        list.fragments.push(fragment);
        while additions
            .peek()
            .is_some_and(|(position, _)| *position == index)
        {
            if let Some((_, property)) = additions.next() {
                list.fragments.push(property);
            }
        }
    }
}

fn lower_attribute(
    attribute: &mut WebUIFragmentAttribute,
    declaration: &ProjectionAttribute,
) -> Option<WebUIFragmentAttribute> {
    let canonical = attribute_to_camel(&attribute.name) == declaration.property;
    if canonical {
        if attribute.condition_tree.is_some()
            || (declaration.mode == 0 && (attribute.raw_value || !attribute.template.is_empty()))
        {
            attribute.attr_skip = false;
            return None;
        }
        if declaration.mode == 1 && attribute.raw_value && attribute.value.is_empty() {
            attribute.raw_value = false;
            attribute.condition_tree = Some(ConditionExpr::identifier("true"));
            attribute.attr_skip = false;
            return None;
        }
    }
    let mut name = String::with_capacity(declaration.property.len() + 1);
    name.push(':');
    name.push_str(&declaration.property);
    let property = WebUIFragmentAttribute {
        name,
        complex: true,
        condition_tree: attribute
            .condition_tree
            .clone()
            .or_else(|| (declaration.mode == 1).then(|| ConditionExpr::identifier("true"))),
        value: if declaration.mode == 0 {
            attribute.value.clone()
        } else {
            String::new()
        },
        raw_value: declaration.mode == 0 && attribute.raw_value,
        template: if declaration.mode == 0 {
            attribute.template.clone()
        } else {
            String::new()
        },
        ..Default::default()
    };
    attribute.attr_skip = true;
    Some(property)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn drawer_inputs_use_existing_attribute_and_property_fragments() {
        let literal = |name: &str, value: &str, start, skip| WebUIFragment {
            fragment: Some(Fragment::Attribute(WebUIFragmentAttribute {
                name: name.into(),
                value: value.into(),
                raw_value: true,
                attr_start: start,
                attr_skip: skip,
                ..Default::default()
            })),
        };
        let mut protocol = WebUIProtocol::new(HashMap::from([
            (
                "index.html".into(),
                FragmentList {
                    fragments: vec![
                        literal("open", "", true, false),
                        literal("aria-label", "Canvas information", false, true),
                        literal("aria-describedby", "details", false, true),
                        WebUIFragment::attribute("aria-labelledby", "headingId"),
                        WebUIFragment::component("mai-drawer"),
                    ],
                    contains_boundary: false,
                },
            ),
            ("mai-drawer".into(), FragmentList::default()),
        ]));
        let declarations = BTreeMap::from([
            (
                "open".into(),
                ProjectionAttribute {
                    property: "open".into(),
                    mode: 1,
                },
            ),
            (
                "aria-label".into(),
                ProjectionAttribute {
                    property: "ariaLabel".into(),
                    mode: 0,
                },
            ),
            (
                "aria-describedby".into(),
                ProjectionAttribute {
                    property: "ariaDescribedby".into(),
                    mode: 0,
                },
            ),
            (
                "aria-labelledby".into(),
                ProjectionAttribute {
                    property: "ariaLabelledby".into(),
                    mode: 0,
                },
            ),
        ]);
        lower_component_attributes(&mut protocol, |tag| {
            (tag == "mai-drawer").then_some(&declarations)
        })
        .unwrap();
        let attributes: Vec<_> = protocol.fragments["index.html"]
            .fragments
            .iter()
            .filter_map(|fragment| match &fragment.fragment {
                Some(Fragment::Attribute(attribute)) => Some(attribute),
                _ => None,
            })
            .collect();
        assert_eq!(attributes.len(), 6);
        assert_eq!(
            attributes[0].condition_tree,
            Some(ConditionExpr::identifier("true"))
        );
        assert!(!attributes[1].attr_skip);
        assert_eq!(attributes[1].value, "Canvas information");
        assert!(attributes[2].attr_skip);
        assert_eq!(attributes[3].name, ":ariaDescribedby");
        assert!(attributes[3].complex && attributes[3].raw_value);
        assert_eq!(attributes[3].value, "details");
        assert!(attributes[4].attr_skip);
        assert_eq!(attributes[5].name, ":ariaLabelledby");
        assert!(attributes[5].complex);
        assert_eq!(
            protocol.fragments[&attributes[5].template].fragments,
            vec![WebUIFragment::signal("headingId", false)],
        );
        assert!(protocol.components.is_empty());
        assert_eq!(
            WebUIProtocol::from_protobuf(&protocol.to_protobuf().unwrap()).unwrap(),
            protocol,
        );
    }
}
