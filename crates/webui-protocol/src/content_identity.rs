// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::ProtocolError;
use crate::{
    ClientFragmentData, ComponentAssetStylePreload, ComponentData, ComponentStyleClosure,
    FragmentList, StyleChunk, WebUIProtocol,
};
use serde::{ser::SerializeMap, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Write};

struct SortedMap<'a, T>(Vec<(&'a str, &'a T)>);

impl<'a, T> SortedMap<'a, T> {
    fn new(values: &'a HashMap<String, T>) -> Self {
        let mut entries: Vec<_> = values
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect();
        entries.sort_unstable_by_key(|(name, _)| *name);
        Self(entries)
    }
}

impl<T: Serialize> Serialize for SortedMap<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct Content<'a> {
    fragments: SortedMap<'a, FragmentList>,
    tokens: &'a [String],
    components: SortedMap<'a, ComponentData>,
    css_strategy: i32,
    initial_state_strategy: i32,
    module_preloads: &'a [String],
    component_render_css: &'a str,
    style_closures: SortedMap<'a, ComponentStyleClosure>,
    style_chunks: &'a [StyleChunk],
    component_asset_style_preloads: &'a [ComponentAssetStylePreload],
    client_fragments: SortedMap<'a, ClientFragmentData>,
    // Omitted when unset, so builds without an asset base keep their identity.
    #[serde(skip_serializing_if = "is_empty")]
    asset_base: &'a str,
}

fn is_empty(value: &&str) -> bool {
    value.is_empty()
}

impl<'a> Content<'a> {
    fn new(protocol: &'a WebUIProtocol) -> Self {
        Self {
            fragments: SortedMap::new(&protocol.fragments),
            tokens: &protocol.tokens,
            components: SortedMap::new(&protocol.components),
            css_strategy: protocol.css_strategy,
            initial_state_strategy: protocol.initial_state_strategy,
            module_preloads: &protocol.module_preloads,
            component_render_css: &protocol.component_render_css,
            style_closures: SortedMap::new(&protocol.style_closures),
            style_chunks: &protocol.style_chunks,
            component_asset_style_preloads: &protocol.component_asset_style_preloads,
            client_fragments: SortedMap::new(&protocol.client_fragments),
            asset_base: &protocol.asset_base,
        }
    }
}

struct HashWriter(Sha256);

impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn compute(protocol: &WebUIProtocol) -> Result<String, ProtocolError> {
    let mut writer = HashWriter(Sha256::new());
    writer.0.update(b"webui.protocol-content/v1\0");
    serde_json::to_writer(&mut writer, &Content::new(protocol))
        .map_err(|error| ProtocolError::Validation(error.to_string()))?;
    Ok(crate::build_hash::format_sha256(
        writer.0.finalize().as_slice(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WebUIFragment;

    #[test]
    fn identity_is_independent_of_map_iteration_and_its_existing_identity() {
        let entries = [
            (
                "x-first".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>First</p>")],
                    ..Default::default()
                },
            ),
            (
                "x-second".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>Second</p>")],
                    ..Default::default()
                },
            ),
        ];
        let first = WebUIProtocol::new(entries.clone().into_iter().collect());
        let mut second = WebUIProtocol::new(entries.into_iter().rev().collect());
        second.build_id = "previous".into();
        assert_eq!(compute(&first).unwrap(), compute(&second).unwrap());
        second.tokens.push("changed".into());
        assert_ne!(compute(&first).unwrap(), compute(&second).unwrap());
    }

    #[test]
    fn identity_covers_every_protocol_field_except_itself() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.asset_base = "https://cdn.example.com/app/".into();
        let mut expected = serde_json::to_value(&protocol).unwrap();
        expected.as_object_mut().unwrap().remove("build_id");
        assert_eq!(
            serde_json::to_value(Content::new(&protocol)).unwrap(),
            expected
        );
    }

    #[test]
    fn an_unset_asset_base_keeps_existing_identities() {
        let protocol = WebUIProtocol::new(HashMap::new());
        let content = serde_json::to_value(Content::new(&protocol)).unwrap();
        assert!(content.get("asset_base").is_none(), "{content}");
        let mut based = protocol.clone();
        based.asset_base = "/static/".into();
        assert_ne!(compute(&protocol).unwrap(), compute(&based).unwrap());
    }
}
