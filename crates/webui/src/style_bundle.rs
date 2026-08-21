// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time CSS bundling.
//!
//! Every component stylesheet is render-blocking, so shipping one file per
//! component costs a request each and forfeits cross-file compression. Bundling
//! merges component stylesheets into chunks, and splits chunks so a stylesheet
//! reached from several CSS trees is downloaded and cached once instead of
//! being duplicated into every route bundle.
//!
//! # Delivery is unchanged
//!
//! Bundling composes with the `CssStrategy` rather than replacing it: it decides
//! *how stylesheets are grouped*, while the strategy decides *how they reach the
//! page*. A Link build gets fewer `<link>` tags and a Style build fewer inline
//! blocks. Module delivery is already inlined and therefore rejects bundling.
//!
//! # Cascade order is preserved exactly
//!
//! Merging stylesheets rewrites the order the browser sees rules in, which can
//! silently change computed styles. Two rules make that impossible here.
//!
//! *Only components with an identical set of consuming CSS trees may merge.*
//! Every tree that needs one member therefore needs all of them, so no tree ever
//! receives a rule it would not have received unbundled.
//!
//! *A chunk's members must be contiguous and identically ordered in every
//! consuming closure.* Walking a closure and emitting each chunk at its first
//! member then reproduces the unbundled rule sequence exactly. Both properties
//! are checked rather than assumed: any incompatible chunk is split back into
//! single-component chunks. Correctness never depends on the grouping heuristic
//! being clever.
//!
//! # Authored-Shadow stylesheets are never merged
//!
//! Parsing resolves an authored-Shadow component's own stylesheet into artifacts
//! that address it by tag, long before chunks exist: the `css_href` a plugin
//! injects into the shadow template it builds roots from, and the module
//! specifier recorded on `shadowrootadoptedstylesheets`. Keeping those
//! stylesheets in single-component chunks keeps both references correct, because
//! such a chunk is named after its sole member and carries that member's exact
//! bytes. Light components carry no such parse-time identity and merge freely,
//! which under a Light-first default is nearly all of them.

use std::collections::HashMap;

use webui_parser::CssLinkOptions;
use webui_protocol::{web_ui_fragment, CssStrategy, StyleChunk, WebUIProtocol};

use crate::chunking::{group_runs, ConsumerMatrix};
use crate::WebUIError;

/// Plan, name, and install bundled CSS chunks on `protocol`.
///
/// `css_by_tag` supplies each component's compiled stylesheet. Link builds have
/// already moved that text into standalone files, so it cannot be read back off
/// the protocol.
///
/// Returns the chunk files a Link build must write. Style and Module builds
/// carry chunk bytes in the protocol and return nothing.
pub(crate) fn bundle_style_chunks(
    protocol: &mut WebUIProtocol,
    entry: &str,
    css_by_tag: &HashMap<&str, &str>,
    css_link_options: &CssLinkOptions,
) -> Result<Vec<(String, String)>, WebUIError> {
    let Some(plan) = plan_chunks(protocol, entry)? else {
        return Ok(Vec::new());
    };
    for chunk in &plan.chunks {
        for tag in &chunk.component_tags {
            let css = css_by_tag
                .get(tag.as_str())
                .ok_or_else(|| missing_css(tag))?;
            if contains_top_level_import(css) {
                return Err(css_bundle_import(tag));
            }
        }
    }

    let emit_files = protocol.css_strategy() == CssStrategy::Link;
    let mut files = Vec::new();
    let mut chunks = Vec::with_capacity(plan.chunks.len());
    for chunk in &plan.chunks {
        let css = concatenate(&chunk.component_tags, css_by_tag)?;
        let name = chunk_name(&chunk.component_tags);
        let mut record = StyleChunk {
            name,
            css: String::new(),
            css_href: String::new(),
            component_tags: chunk.component_tags.clone(),
        };
        if emit_files {
            let resolved = css_link_options.resolve_chunk(&record.name, &css);
            record.css_href = resolved.href;
            files.push((resolved.filename, css));
        } else {
            record.css = css;
        }
        chunks.push(record);
    }

    for (root, chunk_ids) in plan.root_chunks {
        if let Some(closure) = protocol.style_closures.get_mut(&root) {
            closure.style_chunks = chunk_ids;
        }
    }
    protocol.style_chunks = chunks;
    Ok(files)
}

struct ChunkDraft {
    component_tags: Vec<String>,
}

struct BundlePlan {
    chunks: Vec<ChunkDraft>,
    root_chunks: Vec<(String, Vec<u32>)>,
}

/// Group every styled component into exactly one chunk.
///
/// Returns `None` when the build has no component stylesheets to bundle.
fn plan_chunks(protocol: &WebUIProtocol, entry: &str) -> Result<Option<BundlePlan>, WebUIError> {
    let roots = collect_roots(protocol, entry);
    let components = collect_components(protocol, &roots);
    if components.is_empty() {
        return Ok(None);
    }
    // A component belongs to exactly one chunk, so bounding the component count
    // bounds the chunk indices the protocol stores as `u32` handles.
    u32::try_from(components.len()).map_err(|_| style_bundle_graph_too_large())?;

    let index: HashMap<&str, usize> = components
        .iter()
        .enumerate()
        .map(|(id, tag)| (*tag, id))
        .collect();
    let mut consumers = ConsumerMatrix::new(components.len(), roots.len())
        .ok_or_else(style_bundle_graph_too_large)?;
    for (root_id, root) in roots.iter().enumerate() {
        for tag in closure_tags(protocol, root) {
            if let Some(component) = index.get(tag.as_str()) {
                consumers.insert(*component, root_id);
            }
        }
    }

    // Chunks form from closure order, so a chunk's members start out contiguous
    // in the closure that formed them. Every other consuming closure still has
    // to agree, which `split_interleaved_chunks` verifies.
    let isolated: Vec<bool> = components
        .iter()
        .map(|tag| protocol.component_uses_shadow_dom(tag))
        .collect();
    let mut owner = vec![usize::MAX; components.len()];
    let mut chunks: Vec<Vec<usize>> = Vec::new();
    for root in &roots {
        let unassigned: Vec<usize> = closure_tags(protocol, root)
            .iter()
            .filter_map(|tag| index.get(tag.as_str()).copied())
            .filter(|component| owner[*component] == usize::MAX)
            .collect();
        assign_chunks(&unassigned, &isolated, &consumers, &mut owner, &mut chunks);
    }

    split_incompatible_chunks(protocol, &roots, &index, &mut owner, &mut chunks);

    let chunk_drafts = chunks
        .iter()
        .map(|members| ChunkDraft {
            component_tags: members
                .iter()
                .map(|component| components[*component].to_string())
                .collect(),
        })
        .collect();
    let root_chunks = roots
        .iter()
        .map(|root| {
            (
                (*root).to_string(),
                chunk_sequence(protocol, root, &index, &owner),
            )
        })
        .collect();

    Ok(Some(BundlePlan {
        chunks: chunk_drafts,
        root_chunks,
    }))
}

/// Assign every still-unowned component of one closure to a chunk.
///
/// Components marked `isolated` each get a chunk of their own; the stretches
/// between them are grouped by identical consumer sets. Splitting on isolated
/// members rather than filtering them out keeps the surviving members in closure
/// order, so chunks stay contiguous and the cascade is preserved.
fn assign_chunks(
    unassigned: &[usize],
    isolated: &[bool],
    consumers: &ConsumerMatrix,
    owner: &mut [usize],
    chunks: &mut Vec<Vec<usize>>,
) {
    let mut start = 0;
    while start < unassigned.len() {
        let component = unassigned[start];
        if isolated[component] {
            owner[component] = chunks.len();
            chunks.push(vec![component]);
            start += 1;
            continue;
        }
        let mut end = start;
        while end < unassigned.len() && !isolated[unassigned[end]] {
            end += 1;
        }
        let segment = &unassigned[start..end];
        for run in group_runs(segment, consumers) {
            let members = segment[run].to_vec();
            for member in &members {
                owner[*member] = chunks.len();
            }
            chunks.push(members);
        }
        start = end;
    }
}

/// Break apart any chunk whose members are not contiguous and identically
/// ordered in every closure that consumes it.
///
/// A chunk is contiguous in a closure exactly when walking that closure visits
/// the chunk in one uninterrupted stretch. Member positions also have to advance
/// from zero in the chunk's canonical order; a different relative order would
/// silently change the cascade after concatenation.
fn split_incompatible_chunks(
    protocol: &WebUIProtocol,
    roots: &[&str],
    index: &HashMap<&str, usize>,
    owner: &mut [usize],
    chunks: &mut Vec<Vec<usize>>,
) {
    let mut incompatible = vec![false; chunks.len()];
    let mut seen = vec![false; chunks.len()];
    let mut next_member = vec![0usize; chunks.len()];
    let mut member_position = vec![usize::MAX; owner.len()];
    for members in chunks.iter() {
        for (position, member) in members.iter().enumerate() {
            member_position[*member] = position;
        }
    }

    let mut touched = Vec::with_capacity(chunks.len());
    for root in roots {
        let mut current = usize::MAX;
        touched.clear();
        for tag in closure_tags(protocol, root) {
            let Some(component) = index.get(tag.as_str()) else {
                continue;
            };
            let chunk = owner[*component];
            if chunk == usize::MAX {
                continue;
            }
            if chunk != current {
                if seen[chunk] {
                    incompatible[chunk] = true;
                } else {
                    seen[chunk] = true;
                    touched.push(chunk);
                }
                current = chunk;
            }
            if member_position[*component] != next_member[chunk] {
                incompatible[chunk] = true;
            }
            next_member[chunk] += 1;
        }
        for &chunk in &touched {
            seen[chunk] = false;
            next_member[chunk] = 0;
        }
    }
    if !incompatible.iter().any(|split| *split) {
        return;
    }

    let mut rebuilt: Vec<Vec<usize>> = Vec::with_capacity(chunks.len());
    for (chunk, members) in chunks.iter().enumerate() {
        if incompatible[chunk] {
            for member in members {
                owner[*member] = rebuilt.len();
                rebuilt.push(vec![*member]);
            }
        } else {
            for member in members {
                owner[*member] = rebuilt.len();
            }
            rebuilt.push(members.clone());
        }
    }
    *chunks = rebuilt;
}

/// Map a closure's component order onto chunk indices, collapsing runs.
fn chunk_sequence(
    protocol: &WebUIProtocol,
    root: &str,
    index: &HashMap<&str, usize>,
    owner: &[usize],
) -> Vec<u32> {
    let mut sequence: Vec<u32> = Vec::new();
    for tag in closure_tags(protocol, root) {
        let Some(component) = index.get(tag.as_str()) else {
            continue;
        };
        let chunk = owner[*component];
        if chunk == usize::MAX {
            continue;
        }
        // `plan_chunks` bounds the component count, and chunks never outnumber
        // components, so this conversion always succeeds.
        let Ok(chunk) = u32::try_from(chunk) else {
            continue;
        };
        if sequence.last() != Some(&chunk) {
            sequence.push(chunk);
        }
    }
    sequence
}

/// Order the CSS tree roots that chunks may be planned for.
///
/// Only closures the runtime installs *as a unit* qualify: the entry document,
/// every Shadow component (which owns its own `ShadowRoot`), and every route
/// root (installed at its route host into the inherited tree). A plain Light
/// component also keeps a stored closure for independent loading, but it does
/// not own a CSS tree: its normal ancestor tree installs the covering chunk
/// before the host connects. Treating every Light fallback as a consumer would
/// make every component unique and prevent all merging.
///
/// Excluded plain-Light closures keep an empty `style_chunks`; delivery resolves
/// a chunk only when its complete ordered membership is present in that
/// closure, otherwise it falls back to per-component resources. This prevents a
/// partial closure from receiving unrelated bundled CSS.
///
/// The entry comes first because chunks are shaped by whichever closure claims
/// their members first and the entry is what every request pays for. Remaining
/// roots are sorted so a build is reproducible.
fn collect_roots<'a>(protocol: &'a WebUIProtocol, entry: &'a str) -> Vec<&'a str> {
    let mut roots: Vec<&str> = Vec::new();
    for (tag, component) in &protocol.components {
        if tag != entry && component.uses_shadow_dom && protocol.style_closures.contains_key(tag) {
            roots.push(tag.as_str());
        }
    }
    for fragments in protocol.fragments.values() {
        for fragment in &fragments.fragments {
            let Some(web_ui_fragment::Fragment::Route(route)) = fragment.fragment.as_ref() else {
                continue;
            };
            collect_route_roots(protocol, route, entry, &mut roots);
        }
    }
    roots.sort_unstable();
    roots.dedup();
    if protocol.style_closures.contains_key(entry) {
        roots.insert(0, entry);
    }
    roots
}

/// Walk a route tree iteratively, collecting every component installed as its
/// own closure: route bodies plus their pending and error boundaries.
fn collect_route_roots<'a>(
    protocol: &'a WebUIProtocol,
    route: &'a webui_protocol::WebUIFragmentRoute,
    entry: &str,
    roots: &mut Vec<&'a str>,
) {
    let mut work = vec![route];
    while let Some(route) = work.pop() {
        for candidate in [
            route.fragment_id.as_str(),
            route.pending_component.as_str(),
            route.error_component.as_str(),
        ] {
            if !candidate.is_empty()
                && candidate != entry
                && protocol.style_closures.contains_key(candidate)
            {
                roots.push(candidate);
            }
        }
        work.extend(route.children.iter());
    }
}

/// Collect every component reachable from a closure, in a stable order.
fn collect_components<'a>(protocol: &'a WebUIProtocol, roots: &[&str]) -> Vec<&'a str> {
    let mut components: Vec<&str> = roots
        .iter()
        .flat_map(|root| closure_tags(protocol, root))
        .map(String::as_str)
        .collect();
    components.sort_unstable();
    components.dedup();
    components
}

fn closure_tags<'a>(protocol: &'a WebUIProtocol, root: &str) -> &'a [String] {
    protocol
        .style_closures
        .get(root)
        .map_or(&[], |closure| closure.component_tags.as_slice())
}

/// Name a chunk after its first member so emitted files stay attributable.
///
/// Multi-member IDs begin with `_`, which cannot begin a custom-element name, so
/// they cannot collide with component resource IDs. A single-member chunk keeps
/// its component's identity because authored-Shadow templates address it by tag.
fn chunk_name(component_tags: &[String]) -> String {
    match component_tags.first() {
        Some(first) if component_tags.len() > 1 => {
            format!("_chunk-{first}-{}", component_tags.len())
        }
        Some(first) => first.clone(),
        None => "chunk".to_string(),
    }
}

fn concatenate(
    component_tags: &[String],
    css_by_tag: &HashMap<&str, &str>,
) -> Result<String, WebUIError> {
    let mut total = 0usize;
    for tag in component_tags {
        let css = css_by_tag
            .get(tag.as_str())
            .ok_or_else(|| missing_css(tag))?;
        total += css.len() + 1;
    }
    let mut bundled = String::with_capacity(total);
    for tag in component_tags {
        let css = css_by_tag
            .get(tag.as_str())
            .ok_or_else(|| missing_css(tag))?;
        if !bundled.is_empty() && !bundled.ends_with('\n') {
            bundled.push('\n');
        }
        bundled.push_str(css);
    }
    Ok(bundled)
}

fn contains_top_level_import(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut statement_has_token = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => {
                statement_has_token = true;
                index = skip_quoted(source, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(source, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = skip_line_comment(source, index);
            }
            b'\\' => {
                statement_has_token = true;
                index = (index + 2).min(bytes.len());
            }
            b'(' => {
                statement_has_token = true;
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                statement_has_token = true;
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'@' if paren_depth == 0
                && bracket_depth == 0
                && !statement_has_token
                && is_import_at_rule(bytes, index) =>
            {
                return true;
            }
            b';' | b'{' | b'}' if paren_depth == 0 && bracket_depth == 0 => {
                statement_has_token = false;
                index += 1;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                statement_has_token = true;
                index += 1;
            }
        }
    }
    false
}

fn is_import_at_rule(bytes: &[u8], start: usize) -> bool {
    const IMPORT: &[u8] = b"import";
    let Some(end) = start.checked_add(1 + IMPORT.len()) else {
        return false;
    };
    if end > bytes.len() {
        return false;
    }
    if !bytes[start + 1..end]
        .iter()
        .zip(IMPORT)
        .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    {
        return false;
    }
    !bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
}

fn skip_quoted(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_block_comment(source: &str, start: usize) -> usize {
    source[start + 2..]
        .find("*/")
        .map_or(source.len(), |offset| start + offset + 4)
}

fn skip_line_comment(source: &str, start: usize) -> usize {
    source[start + 2..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset + 2)
}

#[cold]
#[inline(never)]
fn missing_css(tag: &str) -> WebUIError {
    WebUIError::InvalidBuildOptions(format!(
        "CSS bundling found no compiled stylesheet for <{tag}>, but its style closure requires one"
    ))
}

#[cold]
#[inline(never)]
fn css_bundle_import(tag: &str) -> WebUIError {
    WebUIError::InvalidBuildOptions(format!(
        "CSS bundling cannot include @import in <{tag}>; move the import to entry CSS, inline the imported rules, or disable --css-bundle"
    ))
}

#[cold]
#[inline(never)]
fn style_bundle_graph_too_large() -> WebUIError {
    WebUIError::InvalidBuildOptions("CSS bundle graph is too large to index safely".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{ComponentData, ComponentStyleClosure, FragmentList, WebUIFragment};

    /// Build a protocol whose non-entry roots are genuine CSS trees.
    ///
    /// Only closures the runtime installs as a unit are chunked, so each
    /// non-entry root is declared as a route root off the entry — the shape a
    /// real routed app produces.
    fn protocol_with(closures: &[(&str, &[&str])], strategy: CssStrategy) -> WebUIProtocol {
        let mut fragments = HashMap::new();
        let mut entry_fragments = Vec::new();
        for (root, tags) in closures {
            if *root == "index.html" {
                continue;
            }
            entry_fragments.push(WebUIFragment::route(format!("/{root}"), *root));
            fragments.insert(
                (*root).to_string(),
                FragmentList {
                    fragments: tags
                        .iter()
                        .map(|tag| WebUIFragment::component(*tag))
                        .collect(),
                },
            );
        }
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: entry_fragments,
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(strategy);
        for (root, tags) in closures {
            protocol.style_closures.insert(
                (*root).to_string(),
                ComponentStyleClosure {
                    component_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                    style_chunks: Vec::new(),
                },
            );
            for tag in *tags {
                protocol
                    .components
                    .entry((*tag).to_string())
                    .or_insert_with(|| ComponentData {
                        css: format!(".{tag}{{}}"),
                        ..Default::default()
                    });
            }
        }
        protocol
    }

    fn bundle(protocol: &mut WebUIProtocol) -> Vec<(String, String)> {
        let css: HashMap<String, String> = protocol
            .components
            .iter()
            .map(|(tag, data)| (tag.clone(), data.css.clone()))
            .collect();
        let borrowed: HashMap<&str, &str> = css
            .iter()
            .map(|(tag, value)| (tag.as_str(), value.as_str()))
            .collect();
        bundle_style_chunks(
            protocol,
            "index.html",
            &borrowed,
            &CssLinkOptions::default(),
        )
        .expect("bundle")
    }

    /// The delivered rule sequence for a root, flattened back to component tags.
    fn delivered(protocol: &WebUIProtocol, root: &str) -> Vec<String> {
        protocol
            .style_closure_chunks(root)
            .expect("closure")
            .iter()
            .flat_map(|chunk| {
                protocol.style_chunks[*chunk as usize]
                    .component_tags
                    .clone()
            })
            .collect()
    }

    #[test]
    fn merges_a_contiguous_run_that_shares_one_consumer() {
        let mut protocol = protocol_with(
            &[("index.html", &["page-header", "page-body", "page-footer"])],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        assert_eq!(protocol.style_chunks.len(), 1);
        assert_eq!(
            protocol.style_chunks[0].component_tags,
            ["page-header", "page-body", "page-footer"]
        );
        assert_eq!(
            protocol.style_chunks[0].css,
            ".page-header{}\n.page-body{}\n.page-footer{}"
        );
        assert_eq!(
            protocol.style_closure_chunks("index.html"),
            Some([0].as_slice())
        );
    }

    #[test]
    fn splits_shared_components_away_from_route_local_ones() {
        let mut protocol = protocol_with(
            &[
                ("index.html", &["app-shell", "home-hero"]),
                ("about-page", &["app-shell", "about-body"]),
            ],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        // app-shell is reached by both roots, so it cannot merge into either
        // route's bundle without being downloaded twice.
        let shared: Vec<&[String]> = protocol
            .style_chunks
            .iter()
            .map(|chunk| chunk.component_tags.as_slice())
            .collect();
        assert!(shared.contains(&["app-shell".to_string()].as_slice()));
        assert_eq!(
            delivered(&protocol, "index.html"),
            ["app-shell", "home-hero"]
        );
        assert_eq!(
            delivered(&protocol, "about-page"),
            ["app-shell", "about-body"]
        );
    }

    #[test]
    fn never_delivers_a_component_a_closure_did_not_ask_for() {
        let mut protocol = protocol_with(
            &[
                ("index.html", &["app-shell", "home-hero", "home-list"]),
                ("about-page", &["app-shell", "about-body"]),
            ],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        for root in ["index.html", "about-page"] {
            let expected = protocol.style_closure(root).expect("closure").to_vec();
            assert_eq!(delivered(&protocol, root), expected);
        }
    }

    #[test]
    fn independently_loaded_light_closure_does_not_receive_partial_chunk() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.set_css_strategy(CssStrategy::Style);
        protocol.style_closures.insert(
            "index.html".to_string(),
            ComponentStyleClosure {
                component_tags: ["a-card", "b-card"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                style_chunks: Vec::new(),
            },
        );
        protocol.style_closures.insert(
            "lazy-panel".to_string(),
            ComponentStyleClosure {
                component_tags: vec!["a-card".to_string()],
                style_chunks: Vec::new(),
            },
        );
        for tag in ["a-card", "b-card"] {
            protocol.components.insert(
                tag.to_string(),
                ComponentData {
                    css: format!(".{tag}{{}}"),
                    ..Default::default()
                },
            );
        }
        let css = HashMap::from([("a-card", ".a-card{}"), ("b-card", ".b-card{}")]);

        bundle_style_chunks(
            &mut protocol,
            "index.html",
            &css,
            &CssLinkOptions::default(),
        )
        .expect("bundle");

        let chunks: Vec<Vec<String>> = protocol
            .style_chunks
            .iter()
            .map(|chunk| chunk.component_tags.clone())
            .collect();
        assert_eq!(
            chunks,
            vec![vec!["a-card".to_string(), "b-card".to_string()]]
        );
        assert_eq!(
            protocol.style_closure_chunks("lazy-panel"),
            Some([].as_slice())
        );
        let chunk_index = protocol.style_chunk_index();
        let closure = protocol.style_closures.get("lazy-panel").unwrap();
        let unit = protocol
            .style_closure_unit(closure, &chunk_index, 0)
            .expect("component closure unit");
        assert_eq!(unit.name, "a-card");
        assert_eq!(unit.chunk, None);
        assert_eq!(
            protocol.style_chunk_members(0),
            Some(["a-card".to_string(), "b-card".to_string()].as_slice())
        );
    }

    #[test]
    fn preserves_cascade_order_when_a_shared_run_is_interleaved() {
        // `index.html` orders the shared pair adjacently; `about-page` splits
        // them around a local component, so the pair must not merge.
        let mut protocol = protocol_with(
            &[
                ("index.html", &["shared-a", "shared-b"]),
                ("about-page", &["shared-a", "about-body", "shared-b"]),
            ],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        assert_eq!(delivered(&protocol, "index.html"), ["shared-a", "shared-b"]);
        assert_eq!(
            delivered(&protocol, "about-page"),
            ["shared-a", "about-body", "shared-b"]
        );
    }

    #[test]
    fn preserves_cascade_order_when_consumers_reverse_a_shared_run() {
        let mut protocol = protocol_with(
            &[
                ("index.html", &["shared-a", "shared-b"]),
                ("about-page", &["shared-b", "shared-a"]),
            ],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        assert_eq!(delivered(&protocol, "index.html"), ["shared-a", "shared-b"]);
        assert_eq!(delivered(&protocol, "about-page"), ["shared-b", "shared-a"]);
        assert!(
            protocol
                .style_chunks
                .iter()
                .all(|chunk| chunk.component_tags.len() == 1),
            "oppositely ordered members cannot share a chunk"
        );
    }

    #[test]
    fn emits_one_file_per_chunk_for_link_builds() {
        let mut protocol = protocol_with(
            &[("index.html", &["page-header", "page-body"])],
            CssStrategy::Link,
        );

        let files = bundle(&mut protocol);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "_chunk-page-header-2.css");
        assert_eq!(files[0].1, ".page-header{}\n.page-body{}");
        assert!(protocol.style_chunks[0].css.is_empty());
        assert_eq!(
            protocol.style_chunks[0].css_href,
            "_chunk-page-header-2.css"
        );
        assert_eq!(
            protocol.style_chunk_resource(0),
            Some(("_chunk-page-header-2", "_chunk-page-header-2.css"))
        );
    }

    #[test]
    fn rejects_top_level_imports_when_bundling() {
        let mut protocol = protocol_with(
            &[("index.html", &["page-header", "page-body"])],
            CssStrategy::Style,
        );
        protocol.components.get_mut("page-header").unwrap().css =
            "@import \"base.css\"; .header{}".to_string();
        let css = HashMap::from([
            ("page-header", "@import \"base.css\"; .header{}"),
            ("page-body", ".body{}"),
        ]);

        let error = bundle_style_chunks(
            &mut protocol,
            "index.html",
            &css,
            &CssLinkOptions::default(),
        )
        .expect_err("bundled @import must fail");
        assert!(error.to_string().contains("disable --css-bundle"));
    }

    #[test]
    fn import_detection_ignores_values_and_comments() {
        assert!(!contains_top_level_import(
            ".card{--url:@import;content:\"@import\"}/* @import \"x\" */"
        ));
        assert!(contains_top_level_import("@IMPORT \"base.css\";"));
    }

    #[test]
    fn names_a_single_component_chunk_after_that_component() {
        let mut protocol = protocol_with(
            &[
                ("index.html", &["shared-card"]),
                ("about-page", &["shared-card"]),
            ],
            CssStrategy::Link,
        );

        let files = bundle(&mut protocol);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "shared-card.css");
    }

    #[test]
    fn multi_member_chunk_ids_cannot_collide_with_component_tags() {
        let mut protocol = protocol_with(
            &[
                ("index.html", &["a-card", "b-card", "chunk-a-card-2"]),
                ("about-page", &["chunk-a-card-2"]),
            ],
            CssStrategy::Style,
        );

        bundle(&mut protocol);

        let names: Vec<&str> = protocol
            .style_chunks
            .iter()
            .map(|chunk| chunk.name.as_str())
            .collect();
        assert!(names.contains(&"_chunk-a-card-2"));
        assert!(names.contains(&"chunk-a-card-2"));
    }

    #[test]
    fn plans_nothing_when_no_component_has_a_stylesheet() {
        let mut protocol = protocol_with(&[("index.html", &[])], CssStrategy::Style);

        let files = bundle(&mut protocol);

        assert!(files.is_empty());
        assert!(protocol.style_chunks.is_empty());
        assert_eq!(
            protocol.style_closure_chunks("index.html"),
            Some([].as_slice())
        );
    }

    #[test]
    fn produces_the_same_plan_on_every_run() {
        let closures: &[(&str, &[&str])] = &[
            ("index.html", &["app-shell", "home-hero"]),
            ("about-page", &["app-shell", "about-body"]),
            ("contact-page", &["app-shell", "about-body", "contact-form"]),
        ];

        let mut first = protocol_with(closures, CssStrategy::Style);
        bundle(&mut first);
        for _ in 0..8 {
            let mut next = protocol_with(closures, CssStrategy::Style);
            bundle(&mut next);
            assert_eq!(next.style_chunks, first.style_chunks);
            assert_eq!(next.style_closures, first.style_closures);
        }
    }
}
