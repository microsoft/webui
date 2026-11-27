// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use serde_json::{Map, Value};
use webui_protocol::{HydrationReadField, HydrationReadPath, HydrationReadPlan};

use super::{compact_string_length, invalid_plan, scope_is_active, ReadSelection, Selector};
use crate::{write_script_safe_json_str, HandlerError, KeyView, ResponseWriter, Result};

impl<'a> Selector<'a> {
    fn at(self, node: u32) -> Self {
        Self { node, ..self }
    }

    fn identity(self) -> (*const HydrationReadPlan, u32) {
        (std::ptr::from_ref(self.plan), self.node)
    }
}

impl<'a> ReadSelection<'a> {
    pub(crate) fn write(
        &self,
        writer: &mut dyn ResponseWriter,
        scratch: &mut Vec<u8>,
        state: &Value,
        whole: KeyView<'_>,
    ) -> Result<()> {
        let Some(source) = state.as_object() else {
            return writer.write("{}");
        };
        scratch.clear();
        let mut walk = Walk {
            output: scratch,
            active_lazy: self.active_lazy.as_ref(),
            compact_lengths: false,
            todo: Vec::new(),
            frames: Vec::new(),
            seen: HashSet::new(),
        };
        let mut all = whole.iter().peekable();
        let mut narrow = self.roots.iter().peekable();
        walk.output.push(b'{');
        let mut first = true;
        while all.peek().is_some() || narrow.peek().is_some() {
            let (name, selectors) = if all
                .peek()
                .is_some_and(|name| narrow.peek().is_none_or(|(key, _)| name <= *key))
            {
                (all.next().ok_or_else(invalid_plan)?, None)
            } else {
                let (name, selectors) = narrow.next().ok_or_else(invalid_plan)?;
                (*name, Some(selectors.as_slice()))
            };
            if name == crate::STATE_INJECT_KEY {
                continue;
            }
            if let Some(value) = source.get(name) {
                write_field(walk.output, &mut first, name)?;
                if let Some(selectors) = selectors {
                    walk.compact_lengths = self.can_compact_strings(name);
                    walk.write_value(value, selectors)?;
                } else {
                    json(walk.output, value)?;
                }
            }
        }
        walk.output.push(b'}');
        let json = std::str::from_utf8(scratch).map_err(json_utf8_error)?;
        write_script_safe_json_str(writer, json)
    }
}

#[derive(Clone, Copy)]
enum Action<'a> {
    Value,
    Truthy,
    Scope(Selector<'a>),
    Items(Selector<'a>),
}

#[derive(Clone, Copy)]
struct Read<'a> {
    path: &'a [HydrationReadField],
    action: Action<'a>,
}

impl<'a> Read<'a> {
    fn new(path: &'a [HydrationReadField], action: Action<'a>) -> Self {
        Self { path, action }
    }

    fn scope(selector: Selector<'a>) -> Self {
        Self::new(&[], Action::Scope(selector))
    }

    fn head(self) -> Option<&'a str> {
        self.path.first().map(|field| field.key.as_str())
    }

    fn array_slot(self) -> usize {
        self.path.first().map_or(0, |field| {
            field
                .array_index
                .map_or(usize::MAX, |index| (index as usize).saturating_add(1))
        })
    }

    fn child(self) -> Self {
        Self::new(&self.path[1..], self.action)
    }

    fn reads_length(&self) -> bool {
        is_length_read(self.path) && matches!(self.action, Action::Value | Action::Truthy)
    }
}

#[derive(Serialize)]
struct StringLength {
    length: usize,
}

struct Walk<'a, 's, 'o> {
    output: &'o mut Vec<u8>,
    active_lazy: Option<&'o HashSet<&'a str>>,
    compact_lengths: bool,
    todo: Vec<Read<'a>>,
    frames: Vec<Frame<'s>>,
    seen: HashSet<(*const HydrationReadPlan, u32)>,
}

impl<'a, 's> Walk<'a, 's, '_> {
    fn write_value(&mut self, source: &'s Value, selectors: &[Selector<'a>]) -> Result<()> {
        self.todo.extend(selectors.iter().copied().map(Read::scope));
        self.descend(source, 0)?;
        while let Some(frame) = self.frames.last_mut() {
            let start = self.todo.len();
            if let Some(child) = frame.next(&mut self.todo, self.output)? {
                self.descend(child, start)?;
            } else {
                frame.close(self.output);
                self.todo.truncate(frame.start);
                self.frames.pop();
            }
        }
        Ok(())
    }

    fn expand(&mut self, start: usize) -> Result<(bool, bool)> {
        self.seen.clear();
        let mut truthy = false;
        let mut cursor = start;
        let mut kept = start;
        while let Some(read) = self.todo.get(cursor).copied() {
            cursor += 1;
            if read.path.is_empty() {
                match read.action {
                    Action::Value => return Ok((true, truthy)),
                    Action::Truthy => {
                        truthy = true;
                        continue;
                    }
                    Action::Scope(selector) => {
                        self.expand_scope(selector)?;
                        continue;
                    }
                    Action::Items(_) => {}
                }
            }
            self.todo[kept] = read;
            kept += 1;
        }
        self.todo.truncate(kept);
        Ok((false, truthy))
    }

    fn expand_scope(&mut self, selector: Selector<'a>) -> Result<()> {
        if !self.seen.insert(selector.identity()) {
            return Ok(());
        }
        let node = selector
            .plan
            .nodes
            .get(selector.node as usize)
            .ok_or_else(invalid_plan)?;
        self.todo.extend(node.values.iter().map(|value| {
            Read::new(
                &value.path,
                if value.truthy {
                    Action::Truthy
                } else {
                    Action::Value
                },
            )
        }));
        self.todo.extend(
            node.repeats
                .iter()
                .map(|edge| Read::new(&edge.collection, Action::Items(selector.at(edge.item)))),
        );
        self.todo.extend(
            node.scopes
                .iter()
                .filter(|edge| scope_is_active(edge, self.active_lazy))
                .map(|edge| Read::scope(selector.at(edge.node))),
        );
        Ok(())
    }

    fn descend(&mut self, value: &'s Value, start: usize) -> Result<()> {
        let (full, truthy) = self.expand(start)?;
        if full {
            self.todo.truncate(start);
            return json(self.output, value);
        }
        let reads = &mut self.todo[start..];
        let kind = match value {
            Value::Object(map) => {
                reads.sort_unstable_by_key(|read| read.head());
                self.output.push(b'{');
                FrameKind::Object {
                    map,
                    cursor: start,
                    first: true,
                    truthy,
                }
            }
            Value::Array(items) => {
                if let Some(fields) = flat_item_fields(reads, items)? {
                    self.todo.truncate(start);
                    return json(
                        self.output,
                        &FlatItems {
                            items,
                            fields,
                            compact_lengths: self.compact_lengths && fields.len() == 1,
                        },
                    );
                }
                reads.sort_unstable_by_key(|read| read.array_slot());
                let each_end = start + reads.partition_point(|read| read.path.is_empty());
                self.output.push(b'[');
                FrameKind::Array {
                    items,
                    each_end,
                    cursor: each_end,
                    index: 0,
                }
            }
            Value::String(text) if !truthy && reads.iter().any(Read::reads_length) => {
                let compact = self.compact_lengths
                    && compact_string_length(text.len())
                    && reads.iter().all(Read::reads_length);
                self.todo.truncate(start);
                return if compact {
                    json(self.output, &StringLength { length: text.len() })
                } else {
                    json(self.output, text)
                };
            }
            _ => {
                self.todo.truncate(start);
                return json(self.output, if truthy { value } else { &Value::Null });
            }
        };
        self.frames.push(Frame { start, kind });
        Ok(())
    }
}

fn is_length_read(path: &[HydrationReadField]) -> bool {
    matches!(path, [field] if field.key == "length")
}

fn flat_item_fields<'a>(
    reads: &[Read<'a>],
    items: &[Value],
) -> Result<Option<&'a [HydrationReadPath]>> {
    let [read] = reads else {
        return Ok(None);
    };
    let Action::Items(selector) = read.action else {
        return Ok(None);
    };
    if !read.path.is_empty() {
        return Ok(None);
    }
    let node = selector
        .plan
        .nodes
        .get(selector.node as usize)
        .ok_or_else(invalid_plan)?;
    // Flat repeats reuse one sorted field set instead of expanding the same
    // projection for every row.
    Ok((node.scopes.is_empty()
        && node.repeats.is_empty()
        && node
            .values
            .iter()
            .all(|read| !read.truthy && read.path.len() == 1)
        && items.iter().all(|item| !item.is_array()))
    .then_some(&node.values))
}

struct FlatItems<'a> {
    items: &'a [Value],
    fields: &'a [HydrationReadPath],
    compact_lengths: bool,
}

impl Serialize for FlatItems<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.items.len()))?;
        let keep_strings = self.fields.iter().any(|read| is_length_read(&read.path));
        for item in self.items {
            seq.serialize_element(&FlatFields {
                item,
                fields: self.fields,
                keep_strings,
                compact_lengths: self.compact_lengths,
            })?;
        }
        seq.end()
    }
}

struct FlatFields<'a> {
    item: &'a Value,
    fields: &'a [HydrationReadPath],
    keep_strings: bool,
    compact_lengths: bool,
}

impl Serialize for FlatFields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let item = match self.item {
            Value::Object(item) => item,
            Value::String(text) if self.keep_strings => {
                return if self.compact_lengths && compact_string_length(text.len()) {
                    StringLength { length: text.len() }.serialize(serializer)
                } else {
                    serializer.serialize_str(text)
                };
            }
            _ => return serializer.serialize_none(),
        };
        let mut map = serializer.serialize_map(None)?;
        for read in self.fields {
            let field = &read.path[0].key;
            if let Some(value) = item.get(field) {
                map.serialize_entry(field, value)?;
            }
        }
        map.end()
    }
}

struct Frame<'s> {
    start: usize,
    kind: FrameKind<'s>,
}

enum FrameKind<'s> {
    Object {
        map: &'s Map<String, Value>,
        cursor: usize,
        first: bool,
        truthy: bool,
    },
    Array {
        items: &'s [Value],
        each_end: usize,
        cursor: usize,
        index: usize,
    },
}

impl<'s> Frame<'s> {
    fn next(&mut self, todo: &mut Vec<Read<'_>>, out: &mut Vec<u8>) -> Result<Option<&'s Value>> {
        match &mut self.kind {
            FrameKind::Object {
                map, cursor, first, ..
            } => next_field(map, cursor, first, todo, out),
            FrameKind::Array {
                items,
                each_end,
                cursor,
                index,
            } => next_item(items, self.start..*each_end, cursor, index, (todo, out)),
        }
    }

    fn close(&self, out: &mut Vec<u8>) {
        match self.kind {
            FrameKind::Object {
                map, first, truthy, ..
            } => {
                // The condition evaluator distinguishes an empty object from a
                // nonempty one. No authored path can read the placeholder key.
                if truthy && first && !map.is_empty() {
                    out.extend_from_slice(br#""":null"#);
                }
                out.push(b'}');
            }
            FrameKind::Array { .. } => out.push(b']'),
        }
    }
}

fn next_field<'s>(
    map: &'s Map<String, Value>,
    cursor: &mut usize,
    first: &mut bool,
    todo: &mut Vec<Read<'_>>,
    out: &mut Vec<u8>,
) -> Result<Option<&'s Value>> {
    let end = todo.len();
    while *cursor < end {
        let start = *cursor;
        let head = todo[start].head();
        *cursor += todo[start..end].partition_point(|read| read.head() == head);
        let Some(head) = head else {
            continue;
        };
        if let Some(value) = map.get(head) {
            write_field(out, first, head)?;
            for i in start..*cursor {
                todo.push(todo[i].child());
            }
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn next_item<'s>(
    items: &'s [Value],
    each: std::ops::Range<usize>,
    cursor: &mut usize,
    index: &mut usize,
    (todo, out): (&mut Vec<Read<'_>>, &mut Vec<u8>),
) -> Result<Option<&'s Value>> {
    let end = todo.len();
    while let Some(value) = items.get(*index) {
        if *index != 0 {
            out.push(b',');
        }
        let key = *index + 1;
        *index += 1;
        let start = *cursor;
        *cursor += todo[start..end].partition_point(|read| read.array_slot() == key);
        if each.is_empty() && start == *cursor {
            out.extend_from_slice(b"null");
            continue;
        }
        for i in each {
            let Action::Items(selector) = todo[i].action else {
                return Err(invalid_plan());
            };
            todo.push(Read::scope(selector));
        }
        for i in start..*cursor {
            todo.push(todo[i].child());
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn write_field(out: &mut Vec<u8>, first: &mut bool, name: &str) -> Result<()> {
    if !std::mem::replace(first, false) {
        out.push(b',');
    }
    json(out, name)?;
    out.push(b':');
    Ok(())
}

fn json(out: &mut Vec<u8>, value: &(impl Serialize + ?Sized)) -> Result<()> {
    serde_json::to_writer(out, value).map_err(json_writer_error)
}

#[cold]
#[inline(never)]
fn json_writer_error(error: serde_json::Error) -> HandlerError {
    HandlerError::Rendering(format!("failed to serialize hydration state: {error}"))
}

#[cold]
#[inline(never)]
fn json_utf8_error(error: std::str::Utf8Error) -> HandlerError {
    HandlerError::Rendering(format!("invalid hydration state UTF-8: {error}"))
}
