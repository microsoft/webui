// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::Pack;
use crate::{HandlerError, Result};
use serde::Serialize;

/// Serialize a pack without reparsing compiled template metadata.
pub(super) fn write(pack: &Pack<'_>) -> Result<String> {
    let document = pack.protocol.protocol();
    let components = pack.protocol.component_index();
    let bytes: usize = pack
        .needed
        .iter()
        .filter_map(|id| document.client_template(id))
        .map(|data| data.metadata.len() + data.functions.len() + data.legacy_html.len())
        .sum();
    let mut output = Vec::with_capacity(256 + bytes);
    output.extend_from_slice(b"{\"type\":\"webui-pack\",\"version\":1,\"fragment\":");
    json(&mut output, pack.root)?;
    output.extend_from_slice(b",\"inventory\":");
    json(&mut output, &pack.inventory)?;
    output.extend_from_slice(b",\"styleInventory\":");
    json(&mut output, &pack.style_inventory)?;
    output.extend_from_slice(b",\"required\":");
    json(&mut output, &pack.required)?;
    output.extend_from_slice(b",\"bits\":{");
    let mut wrote = false;
    for id in &pack.needed {
        let Some(index) = components.get(*id) else {
            continue;
        };
        if wrote {
            output.push(b',');
        }
        wrote = true;
        json(&mut output, *id)?;
        output.push(b':');
        json(&mut output, index)?;
    }
    output.push(b'}');
    // A missing catalog means "no styles"; keeping it out keeps unstyled packs
    // independent of the CSS strategy.
    if !pack.styles.is_empty() {
        output.extend_from_slice(b",\"componentStyles\":");
        json(&mut output, &pack.styles)?;
    }
    if !pack.needed.is_empty() {
        output.extend_from_slice(b",\"templates\":{");
        for (position, id) in pack.needed.iter().enumerate() {
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
    for id in &pack.needed {
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
    output.push(b'}');
    String::from_utf8(output).map_err(|error| HandlerError::Rendering(error.to_string()))
}

fn json(output: &mut Vec<u8>, value: &(impl Serialize + ?Sized)) -> Result<()> {
    serde_json::to_writer(output, value).map_err(|error| HandlerError::Rendering(error.to_string()))
}
