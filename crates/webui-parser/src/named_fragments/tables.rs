// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared explicit table scaffolding keeps SSR range markers and client slots siblings.

use super::*;

const ROW: u8 = 1;
const COLUMN: u8 = 2;
const OTHER: u8 = 4;
const TABLE_AUX: u8 = 8;

#[derive(Default)]
struct Shape {
    kinds: u8,
    calls: Vec<usize>,
}

pub(super) fn collect(source: &str, graph: &FragmentDeclarations) -> Vec<TableWrapper> {
    if !source.match_indices('<').any(|(start, _)| {
        source.as_bytes()[start + 1..]
            .get(..5)
            .is_some_and(|name| name.eq_ignore_ascii_case(b"table"))
    }) {
        return Vec::new();
    }
    let kinds = declaration_kinds(source, graph);
    let mut wrappers = Vec::new();
    let mut pending = Vec::with_capacity(8);
    pending.push(0..source.len());
    while let Some(range) = pending.pop() {
        for event in Walker::new_range(source, range.start, range.end) {
            if let Event::Element(element) = event {
                if element.name().eq_ignore_ascii_case("table") {
                    collect_table(&element, graph, &kinds, &mut wrappers);
                }
                if !element.inner().is_empty()
                    && (!is_inert_context(element.name()) || element.inner() == graph.root_body)
                {
                    pending.push(element.inner());
                }
            }
        }
    }
    wrappers.sort_unstable_by_key(|wrapper| wrapper.range.start);
    wrappers
}

fn declaration_kinds(source: &str, graph: &FragmentDeclarations) -> Vec<u8> {
    let mut kinds = Vec::with_capacity(graph.declarations.len());
    let mut callers = vec![Vec::new(); graph.declarations.len()];
    let mut pending = Vec::new();
    for (index, declaration) in graph.declarations.iter().enumerate() {
        let shape = summarize(source, declaration.body.clone(), graph);
        kinds.push(shape.kinds);
        if shape.kinds != 0 {
            pending.push(index);
        }
        for target in shape.calls {
            callers[target].push(index);
        }
    }
    while let Some(target) = pending.pop() {
        for &caller in &callers[target] {
            let next = kinds[caller] | kinds[target];
            if next != kinds[caller] {
                kinds[caller] = next;
                pending.push(caller);
            }
        }
    }
    kinds
}

fn summarize(source: &str, range: Range<usize>, graph: &FragmentDeclarations) -> Shape {
    let mut shape = Shape::default();
    let mut pending = vec![range];
    while let Some(range) = pending.pop() {
        for event in Walker::new_range(source, range.start, range.end) {
            match event {
                Event::Element(element) => match element.name() {
                    "render" => {
                        if let Some(call) = graph.render_at(element.start) {
                            shape.calls.push(call.target);
                        }
                    }
                    "if" | "for" => pending.push(element.inner()),
                    "fragment" => {}
                    name if name.eq_ignore_ascii_case("style")
                        || name.eq_ignore_ascii_case("script") =>
                    {
                        shape.kinds |= TABLE_AUX;
                    }
                    name if name.eq_ignore_ascii_case("tr") => shape.kinds |= ROW,
                    name if name.eq_ignore_ascii_case("col") => shape.kinds |= COLUMN,
                    _ => shape.kinds |= OTHER,
                },
                Event::Text(text) if !text.trim().is_empty() => shape.kinds |= OTHER,
                Event::Declaration(_) => shape.kinds |= OTHER,
                _ => {}
            }
        }
    }
    shape
}

fn collect_table(
    table: &Element<'_>,
    graph: &FragmentDeclarations,
    kinds: &[u8],
    wrappers: &mut Vec<TableWrapper>,
) {
    let mut cursor = table.content_start;
    let mut current: Option<TableWrapper> = None;
    for event in Walker::new_range(table.source(), table.content_start, table.content_end) {
        let (end, kind) = match event {
            Event::Element(element) => {
                let shape = summarize(table.source(), element.start..element.close_end(), graph);
                let kind = shape
                    .calls
                    .iter()
                    .fold(shape.kinds, |bits, target| bits | kinds[*target]);
                (element.close_end(), kind)
            }
            Event::Text(text) => (
                cursor + text.len(),
                if text.trim().is_empty() { 0 } else { OTHER },
            ),
            Event::Comment(range) => (range.end, 0),
            Event::Declaration(range) | Event::ClosingTag(range) => (range.end, OTHER),
        };
        let tag = if kind & !TABLE_AUX == ROW {
            Some("tbody")
        } else if kind == COLUMN {
            Some("colgroup")
        } else {
            None
        };
        if let Some(run) = &mut current {
            if kind == 0 || tag == Some(run.tag) || kind == TABLE_AUX && run.tag == "tbody" {
                run.range.end = end;
                cursor = end;
                continue;
            }
        }
        if let Some(run) = current.take() {
            wrappers.push(run);
        }
        if let Some(tag) = tag {
            current = Some(TableWrapper {
                table_start: table.start,
                range: cursor..end,
                tag,
            });
        }
        cursor = end;
    }
    if let Some(run) = current {
        wrappers.push(run);
    }
}
