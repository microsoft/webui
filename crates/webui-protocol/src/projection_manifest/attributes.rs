// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;

use super::ProjectionAttribute;
use crate::{web_ui_fragment::Fragment, WebUIFragmentAttribute, WebUIProtocol};

/// Compile validated component attribute declarations into host-callsite fragments.
///
/// Both native and WASM builders run this after projection validation. Runtime
/// renderers consume the resulting property names and modes without manifest lookups.
pub fn apply_component_attributes<'a>(
    protocol: &mut WebUIProtocol,
    attributes_for: impl Fn(&str) -> Option<&'a BTreeMap<String, ProjectionAttribute>>,
) {
    let mut window = Vec::new();
    for list in protocol.fragments.values_mut() {
        window.clear();
        let mut collecting = false;
        for index in 0..list.fragments.len() {
            let metadata = match list.fragments[index].fragment.as_ref() {
                Some(Fragment::Attribute(attribute)) => {
                    if attribute.attr_start {
                        window.clear();
                        collecting = true;
                    }
                    if collecting {
                        window.push(index);
                    }
                    continue;
                }
                Some(Fragment::Component(component)) => attributes_for(&component.fragment_id),
                _ => continue,
            };
            if let Some(metadata) = metadata {
                for &index in &window {
                    if let Some(Fragment::Attribute(attribute)) =
                        list.fragments[index].fragment.as_mut()
                    {
                        apply_attribute(attribute, metadata);
                    }
                }
            }
            window.clear();
            collecting = false;
        }
    }
}

fn apply_attribute(
    attribute: &mut WebUIFragmentAttribute,
    metadata: &BTreeMap<String, ProjectionAttribute>,
) {
    let definition = if attribute.complex {
        let property = attribute.name.strip_prefix(':');
        metadata
            .values()
            .find(|definition| Some(definition.property.as_str()) == property)
    } else {
        metadata.get(&attribute.name)
    };
    if let Some(definition) = definition {
        attribute.property.clone_from(&definition.property);
        attribute.boolean = definition.mode == 1;
        attribute.attr_skip = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FragmentList, WebUIFragment};
    use prost::Message;

    #[test]
    fn serialized_attributes_require_the_current_metadata_fields() {
        for field in ["property", "boolean"] {
            let mut value = serde_json::to_value(WebUIFragmentAttribute::default()).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<WebUIFragmentAttribute>(value).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn resolves_exact_aliases_modes_and_property_bindings_only_on_their_host() {
        let attributes = BTreeMap::from([
            (
                "aria-describedby".into(),
                ProjectionAttribute {
                    property: "ariaDescribedby".into(),
                    mode: 0,
                },
            ),
            (
                "data-expanded".into(),
                ProjectionAttribute {
                    property: "expanded".into(),
                    mode: 1,
                },
            ),
            (
                "open".into(),
                ProjectionAttribute {
                    property: "open".into(),
                    mode: 0,
                },
            ),
        ]);
        let attribute = |name: &str, start, skipped, boolean| WebUIFragment {
            fragment: Some(Fragment::Attribute(WebUIFragmentAttribute {
                name: name.into(),
                attr_start: start,
                attr_skip: skipped,
                boolean,
                ..Default::default()
            })),
        };
        let mut property = WebUIFragment::attribute_complex(":ariaDescribedby", "description");
        if let Some(Fragment::Attribute(attr)) = &mut property.fragment {
            attr.attr_start = true;
        }
        let mut protocol = WebUIProtocol::new(std::collections::HashMap::from([(
            "index.html".into(),
            FragmentList {
                fragments: vec![
                    attribute("aria-describedby", false, false, false),
                    attribute("data-expanded", true, true, false),
                    attribute("aria-describedby", false, false, false),
                    attribute("open", false, false, true),
                    WebUIFragment::component("test-dialog"),
                    property,
                    WebUIFragment::component("test-dialog"),
                    attribute("aria-describedby", true, false, false),
                    WebUIFragment::component("unrelated-dialog"),
                ],
                contains_boundary: false,
            },
        )]));
        apply_component_attributes(&mut protocol, |tag| {
            (tag == "test-dialog").then_some(&attributes)
        });
        let actual: Vec<_> = protocol.fragments["index.html"]
            .fragments
            .iter()
            .filter_map(|fragment| {
                if let Some(Fragment::Attribute(attr)) = &fragment.fragment {
                    Some((attr.property.as_str(), attr.boolean, attr.attr_skip))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            actual,
            [
                ("", false, false),
                ("expanded", true, false),
                ("ariaDescribedby", false, false),
                ("open", false, false),
                ("ariaDescribedby", false, false),
                ("", false, false),
            ]
        );
        let decoded = WebUIProtocol::decode(protocol.encode_to_vec().as_slice()).unwrap();
        assert_eq!(protocol, decoded);
    }
}
