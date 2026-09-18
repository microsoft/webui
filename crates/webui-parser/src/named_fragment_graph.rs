// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compilation and graph consumers for owner-local named fragments.

use super::*;

pub(super) struct TokenGraphVisits<'a> {
    candidates: Vec<&'a str>,
    context: Vec<&'a str>,
    seen: HashMap<&'a str, HashSet<Vec<&'a str>>>,
}

impl<'a> TokenGraphVisits<'a> {
    pub(super) fn new(parser: &'a HtmlParser) -> Self {
        let mut candidates: Vec<&str> = parser
            .fragment_css_tokens
            .values()
            .flat_map(|css| &css.fallback_chains)
            .chain(
                parser
                    .component_registry
                    .get_all()
                    .flat_map(|component| &component.css_fallback_chains),
            )
            .flat_map(|chain| chain.tokens.iter().map(String::as_str))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        Self {
            context: Vec::with_capacity(candidates.len()),
            candidates,
            seen: HashMap::with_capacity(parser.fragment_records.len()),
        }
    }

    pub(super) fn insert(
        &mut self,
        fragment_id: &'a str,
        available: &HashMap<String, usize>,
    ) -> bool {
        self.context.clear();
        self.context.extend(
            self.candidates
                .iter()
                .copied()
                .filter(|token| available.contains_key(*token)),
        );
        let contexts = self.seen.entry(fragment_id).or_default();
        if contexts.contains(self.context.as_slice()) {
            return false;
        }
        contexts.insert(self.context.clone());
        true
    }
}

impl HtmlParser {
    pub(super) fn schedule_named_table_body<'a>(
        &self,
        element: &Element<'a>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> bool {
        if !element.name().eq_ignore_ascii_case("table") {
            return false;
        }
        let Some(graph) = &self.named_fragments else {
            return false;
        };
        let start = graph
            .table_wrappers
            .partition_point(|wrapper| wrapper.range.start < element.content_start);
        let stop = graph
            .table_wrappers
            .partition_point(|wrapper| wrapper.range.start < element.content_end);
        let mut end = element.content_end;
        for wrapper in graph.table_wrappers[start..stop]
            .iter()
            .rev()
            .filter(|wrapper| wrapper.table_start == element.start)
        {
            if wrapper.range.end < end {
                ops.push(ParseOp::Parse {
                    range: wrapper.range.end..end,
                    depth,
                });
            }
            ops.push(ParseOp::EmitClose(wrapper.tag));
            ops.push(ParseOp::Parse {
                range: wrapper.range.clone(),
                depth,
            });
            ops.push(ParseOp::EmitTableOpen(wrapper.tag));
            end = wrapper.range.start;
        }
        if end == element.content_end {
            return false;
        }
        if element.content_start < end {
            ops.push(ParseOp::Parse {
                range: element.content_start..end,
                depth,
            });
        }
        true
    }

    #[cold]
    #[inline(never)]
    pub(super) fn reserved_fragment_id_error(id: &str) -> ParserError {
        Diagnostic::error("fragment identifier uses the compiler-owned namespace")
            .code(codes::RESERVED_FRAGMENT_ID)
            .component(id)
            .snippet(id)
            .help("choose an entry or explicit template identifier that does not start with `}}}webui:fragment:`")
            .into()
    }

    pub(super) fn emit_named_fragment_call(
        &mut self,
        element: &Element<'_>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        let graph = self.named_fragments.as_ref().ok_or_else(|| {
            self.html_error(
                codes::UNKNOWN_FRAGMENT,
                "render call has no local declaration table",
                element.source(),
                element.start,
            )
            .help("declare the referenced fragment in the owning component root or entry <body>")
        })?;
        let call = graph.render_at(element.start).ok_or_else(|| {
            self.html_error(
                codes::INVALID_RENDER,
                "render call was not resolved in the owning template",
                element.source(),
                element.start,
            )
            .help("move the render call into active HTML in its declaring owner")
        })?;
        let declaration = &graph.declarations[call.target];
        let target = (self.foster_context_depth != 0 || self.boundary_parent_scope.is_some())
            .then(|| declaration.fragment_id.clone());
        let fragment = WebUIFragment::render(
            declaration.fragment_id.clone(),
            call.scope.clone(),
            call.alias.clone(),
        );
        if let Some(target) = target {
            self.track_render_boundary_context(element, target);
        }
        self.add_fragment(fragment, fragments);
        Ok(())
    }

    fn track_render_boundary_context(&mut self, element: &Element<'_>, target: String) {
        let diagnostic = if self.foster_context_depth != 0 {
            self.authoring_error_at(
                codes::BOUNDARY_IN_FOSTER_CONTEXT,
                "a render call reaches a boundary in an HTML foster-parenting context",
                element,
            )
            .help("move the boundary-bearing render call outside the table/select context, or put it inside a normal table cell")
        } else if self.boundary_parent_scope.is_some() {
            self.authoring_error_at(
                codes::BOUNDARY_CROSSES_SCOPE,
                "a render call reaches a boundary across component or inert content",
                element,
            )
            .help("move the boundary-bearing render call outside component host children and inert content")
        } else {
            return;
        };
        self.boundary_render_sites.push((target, diagnostic));
    }

    pub(super) fn compile_named_fragment_bodies(&mut self, source: &str) -> Result<()> {
        let count = self
            .named_fragments
            .as_ref()
            .map_or(0, |graph| graph.declarations.len());
        if count == 0 {
            return Ok(());
        }
        let saved_record = std::mem::take(&mut self.current_record_id);
        let saved_body_depth = std::mem::replace(&mut self.body_depth, 1);
        let mut result = Ok(());
        for index in 0..count {
            let Some(declaration) = self
                .named_fragments
                .as_ref()
                .and_then(|graph| graph.declarations.get(index))
            else {
                break;
            };
            let range = declaration.body.clone();
            let id = declaration.fragment_id.clone();
            if let Some(sites) = &mut self.module_entry_sites {
                sites.remove(&id);
            }
            self.current_record_id.clone_from(&id);
            let mut fragments = Vec::new();
            result = self.parse_range(source, range, &mut fragments, 0);
            if result.is_err() {
                break;
            }
            self.flush_raw_buffer(&mut fragments);
            self.fragment_records.insert(
                id,
                FragmentList {
                    fragments,
                    contains_boundary: false,
                },
            );
        }
        self.current_record_id = saved_record;
        self.body_depth = saved_body_depth;
        result
    }

    pub(super) fn prune_named_fragment_graph(&mut self) {
        let mut reachable = HashSet::with_capacity(self.fragment_records.len());
        let mut pending: Vec<&str> = self.token_roots.iter().map(String::as_str).collect();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id.to_string()) {
                continue;
            }
            if let Some(list) = self.fragment_records.get(id) {
                push_child_records(list, &mut pending);
                for fragment in &list.fragments {
                    if let Some(Fragment::Attribute(attribute)) = &fragment.fragment {
                        if !attribute.template.is_empty() {
                            pending.push(&attribute.template);
                        }
                    }
                }
            }
        }
        self.fragment_records.retain(|id, _| reachable.contains(id));
        self.fragment_css_tokens
            .retain(|id, _| reachable.contains(id));
        self.component_dom_analyses
            .retain(|id, _| reachable.contains(id));
        if let Some(sites) = &mut self.module_entry_sites {
            sites.retain(|id, _| reachable.contains(id));
        }
    }

    pub(super) fn finalize_named_module_entries(&mut self) {
        let Some(sites) = &self.module_entry_sites else {
            return;
        };
        self.module_entry_srcs.clear();
        let mut visited = HashSet::new();
        let mut work = vec![ModuleWalk::Enter(self.current_fragment_id.as_str())];
        while let Some(op) = work.pop() {
            match op {
                ModuleWalk::Enter(id) => {
                    if !visited.insert(id) {
                        continue;
                    }
                    if let Some(list) = self.fragment_records.get(id) {
                        work.push(ModuleWalk::Continue(ModuleCursor {
                            id,
                            list,
                            position: 0,
                            site: 0,
                            in_boundary: false,
                        }));
                    }
                }
                ModuleWalk::Continue(mut cursor) => {
                    let entries = sites.get(cursor.id).map_or(&[][..], Vec::as_slice);
                    while let Some(site) = entries.get(cursor.site) {
                        if site.position > cursor.position {
                            break;
                        }
                        if !cursor.in_boundary && !self.module_entry_srcs.contains(&site.src) {
                            self.module_entry_srcs.push(site.src.clone());
                        }
                        cursor.site += 1;
                    }
                    let Some(fragment) = cursor.list.fragments.get(cursor.position) else {
                        continue;
                    };
                    cursor.position += 1;
                    if let Some(Fragment::Boundary(boundary)) = &fragment.fragment {
                        cursor.in_boundary = boundary.phase() == BoundaryPhase::Start;
                    }
                    let follow = !cursor.in_boundary;
                    work.push(ModuleWalk::Continue(cursor));
                    if follow {
                        push_module_children(fragment, &mut work);
                    }
                }
            }
        }
    }
}

struct ModuleCursor<'a> {
    id: &'a str,
    list: &'a FragmentList,
    position: usize,
    site: usize,
    in_boundary: bool,
}

enum ModuleWalk<'a> {
    Enter(&'a str),
    Continue(ModuleCursor<'a>),
}

fn push_module_children<'a>(fragment: &'a WebUIFragment, work: &mut Vec<ModuleWalk<'a>>) {
    match fragment.fragment.as_ref() {
        Some(Fragment::Render(render)) => work.push(ModuleWalk::Enter(&render.fragment_id)),
        Some(Fragment::Component(component)) => {
            work.push(ModuleWalk::Enter(&component.fragment_id))
        }
        Some(Fragment::ForLoop(for_loop)) => work.push(ModuleWalk::Enter(&for_loop.fragment_id)),
        Some(Fragment::IfCond(if_cond)) => work.push(ModuleWalk::Enter(&if_cond.fragment_id)),
        Some(Fragment::Route(route)) => {
            let mut targets = Vec::new();
            push_route_records(&mut targets, route);
            for target in targets.into_iter().rev() {
                work.push(ModuleWalk::Enter(target));
            }
        }
        _ => {}
    }
}
