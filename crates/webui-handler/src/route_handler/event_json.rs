// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Project a validated raw value without materializing its unused siblings.
// Slots are bounded by the compiled paths, not by the number of JSON members
// (including duplicates). Whole leaves keep their original JSON spelling.
use super::{
    invalid_state_json, partial_serialize_error, raw_json, HandlerError, PartialResponseWithState,
    ProjectedRawState, ReadSelection,
};
use serde_json::{value::RawValue, Result};
use std::collections::BTreeMap;
use webui_protocol::HydrationReadField;

// Write into the final response buffer: even a whole object/string selected
// deep inside an event argument needs no intermediate JSON string or Value.
pub(super) fn serialize(
    response: PartialResponseWithState<ProjectedRawState<'_>>,
    events: &ReadSelection<'_>,
) -> std::result::Result<String, HandlerError> {
    let mut out = vec![b'{'];
    let write_error = |error: serde_json::Error| partial_serialize_error(&error.to_string());
    for (key, value) in &response.response {
        serde_json::to_writer(&mut out, key).map_err(write_error)?;
        out.push(b':');
        serde_json::to_writer(&mut out, value).map_err(write_error)?;
        out.push(b',');
    }
    out.extend_from_slice(br#""state":{"#);
    for (i, (key, raw)) in response.state.entries.into_iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        serde_json::to_writer(&mut out, &key).map_err(write_error)?;
        out.push(b':');
        match events.paths_for(&key)? {
            Some(plan) if !plan.paths.is_empty() => {
                project(raw, &plan.paths, plan.compact_lengths, &mut out)
                    .map_err(|error| invalid_state_json(&error.to_string()))?;
            }
            _ => out.extend_from_slice(raw.get().as_bytes()),
        }
    }
    out.extend_from_slice(b"}}");
    String::from_utf8(out).map_err(|error| partial_serialize_error(&error.to_string()))
}

#[derive(Default)]
struct Slot<'plan, 'de> {
    fields: BTreeMap<&'plan str, usize>,
    max_encoded_key: usize,
    indices: BTreeMap<usize, usize>,
    // Remaining input at the last occurrence. A child seen before a duplicate
    // of its parent is stale, even if the new parent omits that child.
    remaining: usize,
    value: Shape<'de>,
}

#[derive(Default)]
enum Shape<'de> {
    #[default]
    Missing,
    Leaf(&'de str),
    StringLength(usize),
    Object,
    Array(usize),
}

fn project(
    raw: &RawValue,
    paths: &[&[HydrationReadField]],
    compact: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut slots = vec![Slot::default()];
    for &path in paths {
        let mut parent = 0;
        for field in path {
            let next = slots.len();
            // Even a Unicode-escaped key takes at most six bytes per original
            // UTF-8 byte. Don't decode an arbitrarily large discarded key.
            slots[parent].max_encoded_key = slots[parent]
                .max_encoded_key
                .max(field.key.len().saturating_mul(6).saturating_add(2));
            let child = *slots[parent].fields.entry(&field.key).or_insert(next);
            if let Some(i) = field.array_index {
                slots[parent].indices.insert(i as usize, child);
            }
            if child == next {
                slots.push(Slot::default());
            }
            parent = child;
        }
    }
    scan(raw.get(), &mut slots, compact)?;
    emit(&slots, out)
}

// Only descend into selected containers. Skipped values and whole leaves
// remain borrowed spans, and their strings are never decoded.
fn scan<'de>(mut input: &'de str, slots: &mut [Slot<'_, 'de>], compact: bool) -> Result<()> {
    let mut stack = Vec::new();
    let mut key_scratch = String::new();
    let mut selected = Some(0);
    loop {
        if let Some(id) = selected {
            slots[id].remaining = input.len();
            let byte = *input.as_bytes().first().ok_or_else(unexpected_end)?;
            match byte {
                b'{' | b'[' if !slots[id].fields.is_empty() => {
                    slots[id].value = if byte == b'{' {
                        Shape::Object
                    } else {
                        Shape::Array(0)
                    };
                    input = &input[1..];
                    stack.push(id);
                }
                _ => {
                    let value = raw_json::take_validated(&mut input)?;
                    slots[id].value = scalar_shape(value, slots, id, compact)?;
                }
            }
        } else {
            raw_json::take_validated(&mut input)?;
        }
        loop {
            let Some(&parent) = stack.last() else {
                return Ok(());
            };
            // The top-level raw selector already validated the entire JSON
            // grammar and literals. Here punctuation only delimits our walk.
            input = input.trim_start_matches([' ', '\r', '\n', '\t', ',']);
            if input.starts_with(['}', ']']) {
                input = &input[1..];
                stack.pop();
                continue;
            }
            selected = match &mut slots[parent].value {
                Shape::Object => {
                    let key = raw_json::take_validated(&mut input)?;
                    input = input.trim_start_matches([' ', '\r', '\n', '\t', ':']);
                    if key.len() > slots[parent].max_encoded_key {
                        None
                    } else {
                        let key = raw_json::validated_key(key, &mut key_scratch)?;
                        slots[parent].fields.get(key).copied()
                    }
                }
                Shape::Array(len) => {
                    let index = *len;
                    *len += 1;
                    slots[parent].indices.get(&index).copied()
                }
                _ => return Err(unexpected_end()),
            };
            break;
        }
    }
}

fn scalar_shape<'de>(
    raw: &'de str,
    slots: &[Slot<'_, '_>],
    id: usize,
    compact: bool,
) -> Result<Shape<'de>> {
    let fields = &slots[id].fields;
    if fields.is_empty() {
        return Ok(Shape::Leaf(raw));
    }
    if raw.starts_with('"')
        && fields
            .get("length")
            .is_some_and(|&child| slots[child].fields.is_empty())
    {
        // The encoded payload is an upper bound. Most short strings can skip
        // even the search for escapes. Always apply the policy to decoded bytes.
        if compact
            && fields.len() == 1
            && crate::hydration_reads::compact_string_length(raw.len() - 2)
        {
            let len = raw_json::validated_string_len(raw)?;
            if crate::hydration_reads::compact_string_length(len) {
                return Ok(Shape::StringLength(len));
            }
        }
        return Ok(Shape::Leaf(raw));
    }
    Ok(Shape::Leaf("null"))
}

#[cold]
fn unexpected_end() -> serde_json::Error {
    serde::de::Error::custom("unexpected end of validated event state")
}

enum Write<'a> {
    Node(usize),
    Field(&'a str),
    Close(u8),
    Nulls(usize),
}

fn emit(slots: &[Slot<'_, '_>], out: &mut Vec<u8>) -> Result<()> {
    let mut stack = vec![Write::Node(0)];
    while let Some(write) = stack.pop() {
        match write {
            Write::Close(byte) => {
                if out.last() == Some(&b',') {
                    out.pop();
                }
                out.extend_from_slice(&[byte, b',']);
            }
            Write::Field(key) => {
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
            }
            Write::Nulls(count) => {
                for _ in 0..count {
                    out.extend_from_slice(b"null,");
                }
            }
            Write::Node(id) => {
                let slot = &slots[id];
                let present = |child: usize| {
                    slots[child].remaining != 0 && slots[child].remaining < slot.remaining
                };
                match slot.value {
                    Shape::Leaf(raw) => {
                        out.extend_from_slice(raw.as_bytes());
                        out.push(b',');
                    }
                    Shape::StringLength(len) => {
                        out.extend_from_slice(br#"{"length":"#);
                        serde_json::to_writer(&mut *out, &len)?;
                        out.extend_from_slice(b"},");
                    }
                    Shape::Missing => return Err(unexpected_end()),
                    Shape::Object => {
                        out.push(b'{');
                        stack.push(Write::Close(b'}'));
                        for (&key, &child) in slot
                            .fields
                            .iter()
                            .rev()
                            .filter(|(_, child)| present(**child))
                        {
                            stack.push(Write::Node(child));
                            stack.push(Write::Field(key));
                        }
                    }
                    Shape::Array(len) => {
                        out.push(b'[');
                        stack.push(Write::Close(b']'));
                        let mut end = len;
                        for (&index, &child) in slot
                            .indices
                            .range(..len)
                            .rev()
                            .filter(|(_, child)| present(**child))
                        {
                            stack.push(Write::Nulls(end - index - 1));
                            stack.push(Write::Node(child));
                            end = index;
                        }
                        stack.push(Write::Nulls(end));
                    }
                }
            }
        }
    }
    out.pop(); // no trailing comma after the root
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_length_compacts_decoded_bytes_only_for_a_sole_terminal_read() -> Result<()> {
        let length = HydrationReadField {
            key: "length".into(),
            array_index: None,
        };
        let other = HydrationReadField {
            key: "other".into(),
            array_index: None,
        };
        let length_path = std::slice::from_ref(&length);
        let other_path = std::slice::from_ref(&other);
        let render = |text: &str, paths: &[&[HydrationReadField]], compact| -> Result<Vec<u8>> {
            let raw = serde_json::from_str::<&RawValue>(text)?;
            raw_json::validate_literals(raw)?;
            let mut out = Vec::new();
            project(raw, paths, compact, &mut out)?;
            Ok(out)
        };
        let long = format!(r#""{}""#, r#"\ud83d\ude42"#.repeat(1024));
        assert_eq!(render(&long, &[length_path], true)?, br#"{"length":4096}"#);
        let short = format!(r#""{}""#, r#"\u0061"#.repeat(200));
        assert_eq!(render(&short, &[length_path], true)?, short.as_bytes());
        assert_eq!(
            render(&long, &[length_path, other_path], true)?,
            long.as_bytes()
        );
        assert_eq!(render(&long, &[length_path], false)?, long.as_bytes());
        Ok(())
    }
}
