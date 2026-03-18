// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;

use super::context::ShellContext;
use super::route_keys;
use super::shell::{apply_page_metadata, page_state_base};

pub(crate) fn static_page_state(
    context: &ShellContext<'_>,
    route_key: &str,
    is_partial: bool,
) -> Option<Value> {
    if !route_keys::is_static_page(route_key) {
        return None;
    }

    let mut state = page_state_base(context, is_partial);
    if !is_partial {
        apply_page_metadata(&mut state, route_key, false, "default-shell");
    }
    Some(Value::Object(state))
}
