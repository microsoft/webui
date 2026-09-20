// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use anyhow::{Context, Result};
use serde::{de::IgnoredAny, Deserialize};
use serde_json::{Map, Value};

pub(super) type SharedState = Arc<RwLock<Value>>;

#[derive(Deserialize)]
struct InitialState {
    contacts: Vec<Value>,
    groups: Vec<Value>,
    // Browser fixtures duplicate these collections. Skip them while parsing;
    // desktop routes derive fresh lists from the canonical store.
    #[serde(default, rename = "filteredContacts")]
    _filtered_contacts: IgnoredAny,
    #[serde(default, rename = "favoriteContacts")]
    _favorite_contacts: IgnoredAny,
    #[serde(default, rename = "recentContacts")]
    _recent_contacts: IgnoredAny,
    #[serde(flatten)]
    seed: Map<String, Value>,
}

impl InitialState {
    fn into_runtime_state(self) -> (Value, SharedState) {
        let store = Map::from_iter([
            ("contacts".to_string(), Value::Array(self.contacts)),
            ("groups".to_string(), Value::Array(self.groups)),
        ]);
        (
            Value::Object(self.seed),
            Arc::new(RwLock::new(Value::Object(store))),
        )
    }
}

pub(super) fn load_state(path: &Path) -> Result<(Value, SharedState)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    parse_state(&bytes).with_context(|| {
        format!(
            "failed to parse {}; provide a JSON object with contacts and groups arrays",
            path.display()
        )
    })
}

pub(super) fn parse_state(bytes: &[u8]) -> Result<(Value, SharedState)> {
    let state: InitialState = serde_json::from_slice(bytes)?;
    Ok(state.into_runtime_state())
}

pub(super) fn read_state(state: &SharedState) -> webui_desktop::Result<RwLockReadGuard<'_, Value>> {
    state
        .read()
        .map_err(|_| webui_desktop::DesktopError::UnsupportedRuntime {
            message: "contact book state lock is poisoned".to_string(),
            help: "restart the desktop app to reinitialize the in-memory state".to_string(),
        })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    #[test]
    fn moves_canonical_collections_and_preserves_global_seed() {
        use super::{read_state, InitialState};
        use serde_json::json;

        let initial: InitialState = serde_json::from_value(json!({
            "contacts": [{"id": "1", "firstName": "Ada"}],
            "groups": ["Work"],
            "filteredContacts": [{"id": "stale"}],
            "favoriteContacts": [{"id": "stale"}],
            "recentContacts": [{"id": "stale"}],
            "searchQuery": "",
            "tokens": {"light": "--background:white;", "dark": "--background:black;"},
            "extension": {"enabled": true}
        }))
        .unwrap();
        let contacts_pointer = initial.contacts.as_ptr();
        let (seed, store) = initial.into_runtime_state();
        let stored = read_state(&store).unwrap();

        assert_eq!(
            stored["contacts"].as_array().unwrap().as_ptr(),
            contacts_pointer
        );
        assert_eq!(stored["contacts"][0]["id"], "1");
        assert_eq!(stored["groups"], json!(["Work"]));
        assert_eq!(stored.as_object().unwrap().len(), 2);
        for key in [
            "contacts",
            "groups",
            "filteredContacts",
            "favoriteContacts",
            "recentContacts",
        ] {
            assert!(
                seed.get(key).is_none(),
                "{key} must not be retained in the render seed"
            );
        }
        assert_eq!(seed["searchQuery"], "");
        assert_eq!(seed["tokens"]["light"], "--background:white;");
        assert_eq!(seed["tokens"]["dark"], "--background:black;");
        assert_eq!(seed["extension"]["enabled"], true);
    }

    #[test]
    fn accepts_seed_without_browser_derived_lists() {
        use super::{parse_state, read_state};
        use serde_json::json;

        let (seed, store) = parse_state(br#"{"contacts":[],"groups":[]}"#).unwrap();
        assert_eq!(seed, json!({}));
        assert_eq!(
            *read_state(&store).unwrap(),
            json!({"contacts": [], "groups": []})
        );
    }

    #[test]
    fn rejects_invalid_store_collections_and_malformed_ignored_data() {
        use super::parse_state;

        for bytes in [
            &b"null"[..],
            br#"{"groups":[]}"#,
            br#"{"contacts":[],"groups":"invalid"}"#,
            br#"{"contacts":{},"groups":[]}"#,
            br#"{"contacts":[],"groups":[],"recentContacts":[}"#,
        ] {
            assert!(parse_state(bytes).is_err());
        }
    }
}
