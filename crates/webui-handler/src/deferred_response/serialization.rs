// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::ResourceRecord;
use crate::{HandlerError, Result, StateSelection};
use serde::{ser::SerializeMap, Serialize, Serializer};
use serde_json::Value;

pub(super) struct SelectedState<'a, 'keys> {
    pub(super) value: &'a Value,
    pub(super) selection: &'a StateSelection<'keys>,
}

impl Serialize for SelectedState<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let values = self
            .value
            .as_object()
            .ok_or_else(|| serde::ser::Error::custom("deferred state must be an object"))?;
        let mut output = serializer.serialize_map(None)?;
        match self.selection {
            StateSelection::Full => {
                for (key, value) in values {
                    if key != crate::STATE_INJECT_KEY {
                        output.serialize_entry(key, value)?;
                    }
                }
            }
            StateSelection::Keys(keys) => {
                for key in keys {
                    if *key != crate::STATE_INJECT_KEY {
                        if let Some(value) = values.get(*key) {
                            output.serialize_entry(key, value)?;
                        }
                    }
                }
            }
            _ => {
                return Err(serde::ser::Error::custom(
                    "invalid deferred state projection",
                ))
            }
        }
        output.end()
    }
}

pub(super) fn write_record(
    record: ResourceRecord<'_>,
    state: Option<&impl Serialize>,
) -> Result<String> {
    let document = record.protocol.protocol();
    let root = &document.client_fragments[record.root];
    let bytes: usize = record
        .needed
        .iter()
        .filter_map(|id| document.client_template(id))
        .map(|data| data.metadata.len() + data.functions.len() + data.legacy_html.len())
        .sum();
    let mut output = Vec::with_capacity(256 + bytes);
    output.extend_from_slice(b"{\"type\":\"webui-deferred\",\"version\":1,\"fragment\":");
    json(&mut output, record.root)?;
    output.extend_from_slice(b",\"owner\":");
    json(&mut output, &root.owner)?;
    output.extend_from_slice(b",\"buildId\":");
    json(&mut output, &document.build_id)?;
    output.extend_from_slice(b",\"inventory\":");
    json(&mut output, &record.inventory)?;
    output.extend_from_slice(b",\"styleInventory\":");
    json(&mut output, &record.style_inventory)?;
    output.extend_from_slice(b",\"required\":");
    json(&mut output, &record.required)?;
    if !record.needed.is_empty() || record.styles.resource_names().next().is_some() {
        output.extend_from_slice(b",\"componentStyles\":");
        json(&mut output, &record.styles)?;
    }
    if !record.needed.is_empty() {
        output.extend_from_slice(b",\"templates\":{");
        for (position, id) in record.needed.iter().enumerate() {
            if position != 0 {
                output.push(b',');
            }
            json(&mut output, *id)?;
            output.push(b':');
            let payload = document
                .client_template(id)
                .ok_or_else(|| HandlerError::MissingFragment((*id).into()))?;
            if payload.metadata.is_empty() {
                json(&mut output, payload.legacy_html)?;
            } else {
                output.extend_from_slice(payload.metadata.as_bytes());
            }
        }
        output.push(b'}');
    }
    let mut wrote = false;
    for id in &record.needed {
        let payload = document
            .client_template(id)
            .ok_or_else(|| HandlerError::MissingFragment((*id).into()))?;
        if payload.functions.is_empty() {
            continue;
        }
        if wrote {
            output.push(b',');
        } else {
            output.extend_from_slice(b",\"templateFunctions\":{");
        }
        wrote = true;
        json(&mut output, *id)?;
        output.push(b':');
        json(&mut output, payload.functions)?;
    }
    if wrote {
        output.push(b'}');
    }
    if let Some(state) = state {
        output.extend_from_slice(b",\"state\":");
        json(&mut output, state)?;
    }
    output.push(b'}');
    String::from_utf8(output).map_err(|error| HandlerError::Rendering(error.to_string()))
}

fn json(output: &mut Vec<u8>, value: &(impl Serialize + ?Sized)) -> Result<()> {
    serde_json::to_writer(output, value).map_err(|error| HandlerError::Rendering(error.to_string()))
}
