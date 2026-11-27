// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;
use webui_protocol::HydrationReadField;

use super::{compact_string_length, string_length_shape};

// Paths are sorted lexicographically by field keys and prefix-free; an empty
// set selects the whole value. Prune each container on its first visit, when
// the contiguous run beneath it still includes every selected child.
pub(crate) fn project_owned_paths<P: AsRef<[HydrationReadField]>>(
    value: &mut Value,
    paths: &[P],
    compact_lengths: bool,
) {
    let mut previous: &[HydrationReadField] = &[];
    for (i, path) in paths.iter().enumerate() {
        let path = path.as_ref();
        let shared = previous
            .iter()
            .zip(path)
            .take_while(|(a, b)| a == b)
            .count();
        let mut run = &paths[i..];
        let mut slot = &mut *value;
        for (depth, field) in path.iter().enumerate() {
            if i == 0 || depth > shared {
                prune(slot, run, depth, compact_lengths);
            }
            let child = match slot {
                Value::Object(map) => map.get_mut(&field.key),
                Value::Array(items) => field.array_index.and_then(|n| items.get_mut(n as usize)),
                _ => None,
            };
            let Some(child) = child else { break };
            slot = child;
            let count = run.partition_point(|p| p.as_ref()[depth].key == field.key);
            run = &run[..count];
        }
        previous = path;
    }
}

fn prune<P: AsRef<[HydrationReadField]>>(
    slot: &mut Value,
    paths: &[P],
    depth: usize,
    compact_lengths: bool,
) {
    match slot {
        Value::Object(map) => {
            // Build a compact map of survivors even with preserve_order, where
            // retaining entries in place would keep the original map capacity.
            let mut source = std::mem::take(map);
            for group in paths.chunk_by(|a, b| a.as_ref()[depth].key == b.as_ref()[depth].key) {
                if let Some((key, value)) = source.remove_entry(&group[0].as_ref()[depth].key) {
                    map.insert(key, value);
                }
            }
        }
        Value::Array(items) => clear_unread_slots(items, paths, depth),
        Value::String(text)
            if paths.iter().any(|p| {
                let path = p.as_ref();
                path.len() == depth + 1 && path[depth].key == "length"
            }) =>
        {
            if paths.len() == 1 && compact_lengths && compact_string_length(text.len()) {
                *slot = string_length_shape(text.len());
            }
        }
        _ => *slot = Value::Null,
    }
}

fn clear_unread_slots<P: AsRef<[HydrationReadField]>>(
    items: &mut [Value],
    paths: &[P],
    depth: usize,
) {
    // Within one decimal digit width, lexical field order is numeric order.
    // Walk only the authored selectors by width (u32 indices), clearing gaps
    // once without an auxiliary array, index set, or a lookup per input row.
    let mut next = 0;
    let mut end = 10usize;
    loop {
        let mut later = false;
        for index in paths.iter().filter_map(|p| p.as_ref()[depth].array_index) {
            let index = index as usize;
            if index < next || index >= items.len() {
                continue;
            }
            if index >= end {
                later = true;
            } else {
                items[next..index].fill(Value::Null);
                next = index + 1;
            }
        }
        if !later {
            break;
        }
        end = end.saturating_mul(10);
    }
    items[next..].fill(Value::Null);
}
