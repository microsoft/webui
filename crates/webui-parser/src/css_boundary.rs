// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compiler-owned CSS boundary for Light DOM components.
//!
//! Light DOM has no native style boundary, so the compiler builds one. Two
//! shapes are available, and [`compile`] picks the strongest one each
//! component's DOM permits — see [`LightScope`].
//!
//! **Stamped** (the fast path, used whenever a component's rendered DOM is
//! fully known at build time). Every element the template emits is stamped with
//! a per-component marker attribute (`data-wl-<id>`, applied in
//! `crate::light_scope`), and every selector is qualified so it can only match
//! a stamped element:
//!
//! ```text
//! .label      ->  .label:where([data-wl-a1b2c3])
//! .a > .b     ->  .a:where([data-wl-a1b2c3]) > .b:where([data-wl-a1b2c3])
//! :host       ->  my-card[data-wl]
//! ```
//!
//! `:where()` contributes zero specificity, so a scoped selector cascades
//! exactly as the developer wrote it.
//!
//! **Enclosed** (the general path). The authored rules are wrapped in a native
//! `@scope` enclosure and `:host` lowers to `:scope`:
//!
//! ```text
//! @scope (my-card[data-wl]) to (:scope [data-wl] > *) { .label { … } }
//! ```
//!
//! **The stamped shape was measured against the enclosed one it fast-paths.**
//! Both produce byte-identical computed styles across the commerce example
//! (~642,000 declarations over four routes, validated against a live A/B of two
//! builds), but stamping recalculates styles 13-15% faster at load and 7-9%
//! faster across a route change, because Blink never computes scope
//! activations. See `DESIGN.md` for the full comparison, including the shapes
//! that were rejected.
//!
//! Stamping can only mark elements a template declares, so a component that
//! renders opaque markup keeps the enclosed shape rather than silently losing
//! its styles. DOM a component builds imperatively from its own JavaScript is
//! outside both shapes' reach and is documented in the styling guide as the one
//! Light DOM authoring rule.

use crate::css_selector::for_each_compound;
use crate::diagnostic::{codes, Diagnostic};
use crate::{comment_policy, css_scan, ParserError, Result, LIGHT_DOM_MARKER_ATTR};
use css_scan::{
    block_comment_end, css_identifier_eq, ident_end, identifier_value, is_ident_start_byte,
    is_identifier_token_start, matching_paren_end, next_char_boundary, pseudo_name, quoted_end,
};
use std::fmt::Write;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Rules,
    Style,
    Keyframes,
}

struct KeyframeName {
    authored: String,
    compiled: String,
}

struct KeyframeRule {
    name_start: usize,
    name_end: usize,
    block_start: usize,
}

struct AnimationValue {
    range: Range<usize>,
    is_name_property: bool,
}

struct Declaration {
    property: Range<usize>,
    value: Range<usize>,
}

#[derive(Default)]
struct AnimationShorthandState {
    timing: bool,
    iteration: bool,
    direction: bool,
    fill: bool,
    play: bool,
    timeline: bool,
}

enum AtRule {
    Grouping { block_start: usize, is_scope: bool },
    Keyframes(KeyframeRule),
}

/// How a Light component's CSS is bounded to the DOM its own template owns.
///
/// The compiler picks the strongest boundary a component's DOM permits, so a
/// component pays for dynamic scoping only when it can actually render DOM the
/// compiler never sees.
#[derive(Clone, Copy)]
pub(crate) enum LightScope<'a> {
    /// Every element the component renders carries `marker`, so the boundary
    /// compiles away into ordinary selector matching. Requires the rendered DOM
    /// to be fully known at build time.
    ///
    /// This is also the shape a future minifier or dead-selector pass needs: a
    /// stamped rule is statically bounded to exactly one template, so whether
    /// it can ever match is decidable at build time.
    Stamped { marker: &'a str },
    /// The component can render markup the compiler never sees, so the boundary
    /// is a native `@scope` enclosure the engine resolves at match time.
    Enclosed,
}

/// Splices per-component scope markers into an authored selector list.
///
/// Owns its scratch buffers so a whole stylesheet is rewritten with a fixed
/// number of allocations regardless of how many rules it contains.
struct Stamper {
    /// Selector matching the component host: `my-card[data-wl]` when stamped,
    /// `:scope` when enclosed, where the `@scope` root is already the host.
    host: String,
    /// Zero-specificity qualifier, e.g. `:where([data-wl-a1b2c3])`. Empty when
    /// enclosed, where `@scope` bounds the selector instead.
    qualifier: String,
    /// Output offsets of host anchors written into the current prelude.
    host_anchors: Vec<usize>,
    edits: Vec<Edit>,
    scratch: String,
}

/// One splice into a selector, at offsets relative to the stamped range.
enum Edit {
    /// Append the scope qualifier at this offset.
    Qualify(usize),
    /// Replace this `:scope` token with the host selector.
    Host(Range<usize>),
}

impl Stamper {
    fn new(tag_name: &str, scope: LightScope<'_>) -> Self {
        let mut host = String::new();
        let mut qualifier = String::new();
        match scope {
            LightScope::Stamped { marker } => {
                host.reserve(tag_name.len() + LIGHT_DOM_MARKER_ATTR.len() + 2);
                host.push_str(tag_name);
                host.push('[');
                host.push_str(LIGHT_DOM_MARKER_ATTR);
                host.push(']');

                qualifier.reserve(marker.len() + 10);
                qualifier.push_str(":where([");
                qualifier.push_str(marker);
                qualifier.push_str("])");
            }
            // The `@scope` root is the host, so `:host` lowers to `:scope` and
            // the enclosure alone bounds every selector.
            LightScope::Enclosed => host.push_str(":scope"),
        }

        Self {
            host,
            qualifier,
            host_anchors: Vec::new(),
            edits: Vec::new(),
            scratch: String::new(),
        }
    }

    /// Whether selectors carry a marker, as opposed to being `@scope`-enclosed.
    fn is_stamped(&self) -> bool {
        !self.qualifier.is_empty()
    }

    /// Begin a new selector prelude, discarding the previous one's anchors.
    fn open_prelude(&mut self) {
        self.host_anchors.clear();
    }

    /// Record that a host anchor was written at `offset` in the output buffer.
    fn note_host_anchor(&mut self, offset: usize) {
        self.host_anchors.push(offset);
    }

    /// Qualify every unbound compound in `output[range]`.
    ///
    /// `lower_scope` rewrites a top-level `:scope` to the host selector. It is
    /// off inside an authored `@scope` block, where `:scope` refers to the
    /// developer's own scoping root rather than the component host.
    fn qualify(&mut self, output: &mut String, range: Range<usize>, lower_scope: bool) {
        if !self.is_stamped() {
            return;
        }
        let selector = &output[range.clone()];
        self.edits.clear();
        let (host_anchors, edits) = (&self.host_anchors, &mut self.edits);
        for_each_compound(selector, |compound| {
            let bound = compound.nesting_parent
                || host_anchors.iter().any(|anchor| {
                    (range.start + compound.start..range.start + compound.insert_at)
                        .contains(anchor)
                });
            if bound {
                return;
            }
            match compound.scope_anchor {
                // A top-level `:scope` outside an authored `@scope` names the
                // component host, which is what `:host` also lowers to.
                Some(anchor) if lower_scope => edits.push(Edit::Host(anchor)),
                Some(_) => {}
                None => edits.push(Edit::Qualify(compound.insert_at)),
            }
        });
        if self.edits.is_empty() {
            return;
        }

        self.scratch.clear();
        self.scratch
            .reserve(selector.len() + self.edits.len() * self.qualifier.len());
        let mut copied = 0usize;
        for edit in &self.edits {
            match edit {
                Edit::Qualify(at) => {
                    self.scratch.push_str(&selector[copied..*at]);
                    self.scratch.push_str(&self.qualifier);
                    copied = *at;
                }
                Edit::Host(anchor) => {
                    self.scratch.push_str(&selector[copied..anchor.start]);
                    self.scratch.push_str(&self.host);
                    copied = anchor.end;
                }
            }
        }
        self.scratch.push_str(&selector[copied..]);
        output.replace_range(range, &self.scratch);
    }

    /// Qualify the selectors inside an authored `@scope (…) to (…)` prelude.
    ///
    /// Only the parenthesised selector lists are rewritten; the `to` keyword
    /// between them must survive untouched. Groups are stamped back-to-front so
    /// earlier offsets stay valid as the buffer grows.
    fn qualify_scope_prelude(&mut self, output: &mut String, prelude_start: usize) {
        let mut groups: Vec<Range<usize>> = Vec::new();
        let mut index = prelude_start;
        while let Some(open) = output[index..].find('(').map(|at| index + at) {
            let Some(close) = matching_paren_end(output, open) else {
                break;
            };
            groups.push(open + 1..close - 1);
            index = close;
        }
        for group in groups.into_iter().rev() {
            self.qualify(output, group, false);
        }
    }
}

/// Compile developer-authored component CSS for one Light DOM component.
///
/// `scope` selects the boundary shape; see [`LightScope`].
pub(crate) fn compile(tag_name: &str, scope: LightScope<'_>, source: &str) -> Result<String> {
    if source.trim().is_empty() {
        return Ok(String::new());
    }

    let keyframes = collect_keyframes_and_validate(tag_name, source)?;
    let mut stamper = Stamper::new(tag_name, scope);
    let rewritten = rewrite_css(tag_name, source, &keyframes, &mut stamper)?;
    if stamper.is_stamped() {
        return Ok(rewritten);
    }

    let mut output = String::with_capacity(tag_name.len() + rewritten.len() + 58);
    // The lower boundary is also the fastest `@scope` shape measured, not just
    // the isolating one. Dropping it, or replacing it with an implicit
    // `to ([data-wl] > *)`, measured 5.4% slower style recalculation: the
    // limit prunes nested component subtrees out of the scope, shrinking the
    // element set Blink computes scope activations for. Narrowing the root to
    // a bare tag, tightening the limit, or wrapping it in `:where()` all
    // measured 3-5% slower. Do not "simplify" this prelude without measuring.
    output.push_str("@scope (");
    output.push_str(tag_name);
    output.push_str("[data-wl]) to (:scope [data-wl] > *) {\n");
    output.push_str(&rewritten);
    output.push_str("\n}");
    Ok(output)
}

fn collect_keyframes_and_validate(tag_name: &str, source: &str) -> Result<Vec<KeyframeName>> {
    let bytes = source.as_bytes();
    let mut keyframes = Vec::new();
    let mut blocks = vec![BlockKind::Rules];
    let mut pending_block = None;
    let mut index = 0usize;
    let mut segment_start = true;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut nested_brace_depth = 0usize;

    while index < bytes.len() {
        if segment_start && current_block(&blocks) == BlockKind::Style {
            if let Some(declaration) = declaration_at(source, index) {
                index = declaration.value.end;
                segment_start = false;
                continue;
            }
        }

        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'@' if segment_start => {
                match parse_at_rule(tag_name, source, index)? {
                    AtRule::Keyframes(rule) => {
                        let authored_token = &source[rule.name_start..rule.name_end];
                        let authored = identifier_value(authored_token);
                        if !keyframes
                            .iter()
                            .any(|keyframe: &KeyframeName| keyframe.authored == authored.as_str())
                        {
                            keyframes.push(KeyframeName {
                                authored,
                                compiled: compiled_keyframe_name(tag_name, authored_token),
                            });
                        }
                        pending_block = Some((rule.block_start, BlockKind::Keyframes));
                        index = rule.name_end;
                    }
                    AtRule::Grouping { block_start, .. } => {
                        pending_block = Some((block_start, current_block(&blocks)));
                        index += 1;
                    }
                }
                segment_start = false;
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
                segment_start = false;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
                segment_start = false;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth > 0 || bracket_depth > 0 => {
                nested_brace_depth += 1;
                index += 1;
            }
            b'{' => {
                blocks.push(block_for_open(&mut pending_block, index));
                index += 1;
                segment_start = true;
            }
            b'}' if nested_brace_depth > 0 => {
                nested_brace_depth -= 1;
                index += 1;
            }
            b'}' => {
                if blocks.len() > 1 {
                    blocks.pop();
                }
                index += 1;
                segment_start = true;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => {
                index += 1;
                segment_start = true;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                index = next_char_boundary(source, index);
                segment_start = false;
            }
        }
    }

    Ok(keyframes)
}

fn rewrite_css(
    tag_name: &str,
    source: &str,
    keyframes: &[KeyframeName],
    stamper: &mut Stamper,
) -> Result<String> {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len() + keyframes.len() * 12);
    let mut blocks = vec![BlockKind::Rules];
    // Parallel to `blocks`: whether each open block came from an authored
    // `@scope`, where `:scope` means the developer's root, not the host.
    let mut scope_blocks = vec![false];
    let mut authored_scope_depth = 0usize;
    let mut pending_block = None;
    // The at-rule prelude awaiting its `{`, and whether it is an authored
    // `@scope`. An at-rule prelude is never a selector list, so it must never
    // be qualified.
    let mut pending_at_prelude: Option<(usize, bool)> = None;
    let mut copy_start = 0usize;
    let mut index = 0usize;
    let mut segment_start = true;
    let mut prelude_start = None;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut nested_brace_depth = 0usize;

    while index < bytes.len() {
        if segment_start && current_block(&blocks) == BlockKind::Style {
            if let Some(declaration) = declaration_at(source, index) {
                if is_animation_property(&source[declaration.property.clone()]) {
                    let property = &source[declaration.property.clone()];
                    let is_name_property = css_identifier_eq(property, "animation-name")
                        || css_identifier_eq(property, "-webkit-animation-name");
                    output.push_str(&source[copy_start..declaration.value.start]);
                    rewrite_animation_value(
                        tag_name,
                        source,
                        AnimationValue {
                            range: declaration.value.clone(),
                            is_name_property,
                        },
                        keyframes,
                        &mut output,
                    )?;
                    copy_start = declaration.value.end;
                }
                index = declaration.value.end;
                segment_start = false;
                continue;
            }
        }

        // A selector prelude begins at the first non-blank byte of a segment.
        // Flushing here pins where the rewritten prelude starts in `output`, so
        // the stamper can splice into text that `:host` lowering already
        // rewrote rather than re-deriving it from the source.
        if segment_start && prelude_start.is_none() && !bytes[index].is_ascii_whitespace() {
            output.push_str(&source[copy_start..index]);
            copy_start = index;
            prelude_start = Some(output.len());
            stamper.open_prelude();
        }

        if bytes[index] == b'"' || bytes[index] == b'\'' {
            index = quoted_end(source, index);
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = block_comment_end(source, index);
            continue;
        }
        if bytes[index] == b'/' && comment_policy::is_css_line_comment_start(source, index) {
            index = comment_policy::find_css_line_comment_end(source, index + 2);
            continue;
        }

        if bytes[index] == b':' {
            if let Some(pseudo) = pseudo_name(source, index) {
                let raw_name = &source[pseudo.name.clone()];
                let decoded_name = raw_name.contains('\\').then(|| identifier_value(raw_name));
                let name = decoded_name.as_deref().unwrap_or(raw_name);
                if !pseudo.is_element && name.eq_ignore_ascii_case("host-context") {
                    return Err(unsupported_light_css_error(
                        tag_name,
                        source,
                        index,
                        "`:host-context()` cannot be isolated in Light DOM",
                        "move the contextual rule to entry CSS, or wrap the component in `<template shadowrootmode=\"open\">`",
                    ));
                }
                if pseudo.is_element && name.eq_ignore_ascii_case("slotted") {
                    return Err(unsupported_light_css_error(
                        tag_name,
                        source,
                        index,
                        "`::slotted()` requires Shadow DOM slot projection",
                        "wrap the component in `<template shadowrootmode=\"open\">` to use `::slotted()`",
                    ));
                }
                if !pseudo.is_element && name.eq_ignore_ascii_case("host") {
                    let end = rewrite_host_selector(
                        HostRewrite {
                            tag_name,
                            source,
                            pseudo: index..pseudo.name.end,
                            copy_start,
                        },
                        &mut output,
                        stamper,
                    )?;
                    copy_start = end;
                    index = end;
                    segment_start = false;
                    continue;
                }
            }
        }

        if bytes[index] == b'@' && segment_start {
            match parse_at_rule(tag_name, source, index)? {
                AtRule::Keyframes(rule) => {
                    let authored = &source[rule.name_start..rule.name_end];
                    if let Some(keyframe) = find_keyframe(keyframes, authored) {
                        output.push_str(&source[copy_start..rule.name_start]);
                        output.push_str(&keyframe.compiled);
                        copy_start = rule.name_end;
                    }
                    pending_block = Some((rule.block_start, BlockKind::Keyframes));
                    pending_at_prelude = Some((rule.block_start, false));
                    index = rule.name_end;
                }
                AtRule::Grouping {
                    block_start,
                    is_scope,
                } => {
                    pending_block = Some((block_start, current_block(&blocks)));
                    pending_at_prelude = Some((block_start, is_scope));
                    index += 1;
                }
            }
            segment_start = false;
            continue;
        }

        match bytes[index] {
            b'(' => {
                paren_depth += 1;
                index += 1;
                segment_start = false;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
                segment_start = false;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth > 0 || bracket_depth > 0 => {
                nested_brace_depth += 1;
                index += 1;
            }
            b'{' => {
                let kind = block_for_open(&mut pending_block, index);
                let at_prelude = match pending_at_prelude {
                    Some((block_start, is_scope)) if block_start == index => {
                        pending_at_prelude = None;
                        Some(is_scope)
                    }
                    _ => None,
                };
                let authored_scope = at_prelude == Some(true);
                if let Some(start) = prelude_start.take() {
                    output.push_str(&source[copy_start..index]);
                    copy_start = index;
                    let end = output.len();
                    if authored_scope {
                        stamper.qualify_scope_prelude(&mut output, start);
                    } else if at_prelude.is_none()
                        && kind == BlockKind::Style
                        && current_block(&blocks) != BlockKind::Keyframes
                    {
                        // Only a real selector list is qualified. A keyframe
                        // selector (`from`, `50%`) opens a Style block but is
                        // not a selector, and a nested at-rule inherits the
                        // enclosing Style kind, so qualifying its prelude
                        // would corrupt the at-rule itself.
                        stamper.qualify(&mut output, start..end, authored_scope_depth == 0);
                    }
                }
                blocks.push(kind);
                scope_blocks.push(authored_scope);
                authored_scope_depth += usize::from(authored_scope);
                index += 1;
                segment_start = true;
            }
            b'}' if nested_brace_depth > 0 => {
                nested_brace_depth -= 1;
                index += 1;
            }
            b'}' => {
                if blocks.len() > 1 {
                    blocks.pop();
                    authored_scope_depth -=
                        usize::from(scope_blocks.pop().is_some_and(|scope| scope));
                }
                index += 1;
                segment_start = true;
                prelude_start = None;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => {
                index += 1;
                segment_start = true;
                prelude_start = None;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                index = next_char_boundary(source, index);
                segment_start = false;
            }
        }
    }

    output.push_str(&source[copy_start..]);
    Ok(output)
}

/// Inputs for lowering one `:host` token to the Light host selector.
struct HostRewrite<'a> {
    tag_name: &'a str,
    source: &'a str,
    pseudo: Range<usize>,
    copy_start: usize,
}

fn rewrite_host_selector(
    rewrite: HostRewrite<'_>,
    output: &mut String,
    stamper: &mut Stamper,
) -> Result<usize> {
    let HostRewrite {
        tag_name,
        source,
        pseudo,
        copy_start,
    } = rewrite;
    output.push_str(&source[copy_start..pseudo.start]);
    // The compound is now bound to the host element, so the stamper must not
    // qualify it with the descendant marker the host itself never carries.
    stamper.note_host_anchor(output.len());
    let argument_start = pseudo.end;
    if source.as_bytes().get(argument_start) != Some(&b'(') {
        output.push_str(&stamper.host);
        return Ok(argument_start);
    }

    let Some(close) = matching_paren_end(source, argument_start) else {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            pseudo.start,
            "unterminated `:host()` selector",
            "close the `:host(...)` selector before building",
        ));
    };
    if !has_significant_content(source, argument_start + 1..close - 1) {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            pseudo.start,
            "empty `:host()` selector",
            "use `:host` without parentheses, or provide a compound selector",
        ));
    }
    validate_host_compound(tag_name, source, argument_start + 1..close - 1)?;
    validate_shadow_only_pseudos(tag_name, source, argument_start + 1..close - 1)?;
    output.push_str(&stamper.host);
    output.push_str(":is(");
    output.push_str(&source[argument_start + 1..close - 1]);
    output.push(')');
    Ok(close)
}

fn rewrite_animation_value(
    tag_name: &str,
    source: &str,
    value: AnimationValue,
    keyframes: &[KeyframeName],
    output: &mut String,
) -> Result<()> {
    let bytes = source.as_bytes();
    let mut copy_start = value.range.start;
    let mut index = value.range.start;
    let mut paren_depth = 0usize;
    let mut shorthand = AnimationShorthandState::default();

    while index < value.range.end {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = block_comment_end(source, index).min(value.range.end);
            continue;
        }
        if bytes[index] == b'/' && comment_policy::is_css_line_comment_start(source, index) {
            index =
                comment_policy::find_css_line_comment_end(source, index + 2).min(value.range.end);
            continue;
        }
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            let token_end = quoted_end(source, index).min(value.range.end);
            if paren_depth == 0 {
                if let Some(keyframe) = find_keyframe(keyframes, &source[index..token_end]) {
                    output.push_str(&source[copy_start..index]);
                    output.push_str(&keyframe.compiled);
                    copy_start = token_end;
                }
            }
            index = token_end;
            continue;
        }
        if bytes[index] == b'(' {
            paren_depth += 1;
            index += 1;
            continue;
        }
        if bytes[index] == b')' {
            paren_depth = paren_depth.saturating_sub(1);
            index += 1;
            continue;
        }
        if bytes[index] == b',' && paren_depth == 0 {
            shorthand = AnimationShorthandState::default();
            index += 1;
            continue;
        }
        if is_identifier_token_start(bytes, index, value.range.start) {
            let token_end = ident_end(bytes, index, value.range.end);
            let token = &source[index..token_end];
            let decoded_token = token.contains('\\').then(|| identifier_value(token));
            let semantic_token = decoded_token.as_deref().unwrap_or(token);
            let is_function = bytes.get(token_end) == Some(&b'(');
            if is_function && !keyframes.is_empty() && is_dynamic_css_function(semantic_token) {
                return Err(dynamic_keyframe_error(tag_name, source, index));
            }
            let may_be_name = paren_depth == 0
                && (value.is_name_property
                    || shorthand.identifier_may_be_name(semantic_token, is_function));
            if may_be_name && !is_function {
                if let Some(keyframe) = find_keyframe(keyframes, token) {
                    output.push_str(&source[copy_start..index]);
                    output.push_str(&keyframe.compiled);
                    copy_start = token_end;
                }
            }
            index = token_end;
            continue;
        }
        index = next_char_boundary(source, index);
    }

    output.push_str(&source[copy_start..value.range.end]);
    Ok(())
}

fn declaration_at(source: &str, start: usize) -> Option<Declaration> {
    let property_start = skip_whitespace_and_comments(source, start);
    let bytes = source.as_bytes();
    if !bytes
        .get(property_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return None;
    }
    let property_end = ident_end(bytes, property_start, bytes.len());
    let colon = skip_whitespace_and_comments(source, property_end);
    if bytes.get(colon) != Some(&b':') {
        return None;
    }
    let value_start = colon + 1;
    let value_end = declaration_value_end(source, value_start);
    // A standard declaration value never contains a top-level `{`. When one
    // appears this is a nested rule whose selector merely looks like a
    // property, e.g. `div:hover { … }`, and the caller must scan it as a
    // selector so it gets scoped like any other. A custom property is exempt:
    // its value is an arbitrary token stream that may legally contain braces.
    if !source[property_start..property_end].starts_with("--")
        && source[value_start..value_end].contains('{')
    {
        return None;
    }
    Some(Declaration {
        property: property_start..property_end,
        value: value_start..value_end,
    })
}

fn declaration_value_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'/' if comment_policy::is_css_line_comment_start(source, index) => {
                index = comment_policy::find_css_line_comment_end(source, index + 2);
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b'{' if paren_depth == 0 && bracket_depth == 0 => {
                brace_depth += 1;
                index += 1;
            }
            b'}' if paren_depth == 0 && bracket_depth == 0 && brace_depth > 0 => {
                brace_depth -= 1;
                index += 1;
            }
            b';' | b'}' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                break;
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    index
}

fn parse_at_rule(tag_name: &str, source: &str, start: usize) -> Result<AtRule> {
    let (name, keyword_end) = at_keyword(source, start);
    if is_keyframes_keyword(name) {
        return parse_keyframe_rule(tag_name, source, keyword_end).map(AtRule::Keyframes);
    }
    if !is_scoped_grouping_keyword(name) {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            start,
            "unsupported global at-rule in Light component CSS",
            "move global CSS to the entry stylesheet, or use a block @media, @supports, @container, @layer, or @scope rule",
        ));
    }
    match at_rule_terminator(source, keyword_end) {
        Some((block_start, b'{')) => Ok(AtRule::Grouping {
            block_start,
            is_scope: css_identifier_eq(name, "scope"),
        }),
        _ => Err(unsupported_light_css_error(
            tag_name,
            source,
            start,
            "unscopable at-rule statement in Light component CSS",
            "use the block form of this grouping rule, or move the statement to the entry stylesheet",
        )),
    }
}

fn parse_keyframe_rule(tag_name: &str, source: &str, keyword_end: usize) -> Result<KeyframeRule> {
    let name_start = skip_whitespace_and_comments(source, keyword_end);
    if matches!(source.as_bytes().get(name_start), Some(b'"' | b'\'')) {
        let name_end = quoted_end(source, name_start);
        let block_start = skip_whitespace_and_comments(source, name_end);
        if source.as_bytes().get(block_start) != Some(&b'{') {
            return Err(unsupported_light_css_error(
                tag_name,
                source,
                name_start,
                "unsupported keyframe name syntax in Light DOM",
                "use a static CSS identifier or string for the keyframe name",
            ));
        }
        return Ok(KeyframeRule {
            name_start,
            name_end,
            block_start,
        });
    }
    if !source
        .as_bytes()
        .get(name_start)
        .is_some_and(|byte| is_ident_start_byte(*byte))
    {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            keyword_end,
            "keyframes require a static identifier name in Light DOM",
            "use an identifier such as `@keyframes fade`, then reference that name statically",
        ));
    }
    let name_end = ident_end(source.as_bytes(), name_start, source.len());
    if name_start == name_end {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            keyword_end,
            "keyframes require a static identifier name in Light DOM",
            "use an identifier such as `@keyframes fade`, then reference that name statically",
        ));
    }
    let block_start = skip_whitespace_and_comments(source, name_end);
    if source.as_bytes().get(block_start) != Some(&b'{') {
        return Err(unsupported_light_css_error(
            tag_name,
            source,
            name_start,
            "unsupported keyframe name syntax in Light DOM",
            "use an unquoted CSS identifier for the keyframe name",
        ));
    }
    Ok(KeyframeRule {
        name_start,
        name_end,
        block_start,
    })
}

fn at_keyword(source: &str, start: usize) -> (&str, usize) {
    let end = ident_end(source.as_bytes(), start + 1, source.len());
    (&source[start + 1..end], end)
}

fn is_keyframes_keyword(name: &str) -> bool {
    css_identifier_eq(name, "keyframes") || css_identifier_eq(name, "-webkit-keyframes")
}

fn is_scoped_grouping_keyword(name: &str) -> bool {
    css_identifier_eq(name, "media")
        || css_identifier_eq(name, "supports")
        || css_identifier_eq(name, "container")
        || css_identifier_eq(name, "layer")
        || css_identifier_eq(name, "scope")
        || css_identifier_eq(name, "starting-style")
}

fn find_keyframe<'a>(keyframes: &'a [KeyframeName], name: &str) -> Option<&'a KeyframeName> {
    if !matches!(name.as_bytes().first(), Some(b'"' | b'\'')) && !name.contains('\\') {
        return keyframes.iter().find(|keyframe| keyframe.authored == name);
    }
    let decoded = identifier_value(name);
    keyframes
        .iter()
        .find(|keyframe| keyframe.authored == decoded)
}

/// Build the component-local keyframe name.
///
/// The tag length delimits the prefix so two components can never compile to
/// the same name: `<x-foo>`/`bar-baz` and `<x-foo-bar>`/`baz` would otherwise
/// both produce `wui-x-foo-bar-baz`. `@scope` does not isolate `@keyframes`,
/// so a collision would silently animate with another component's rule.
fn compiled_keyframe_name(tag_name: &str, authored: &str) -> String {
    let quote = authored
        .as_bytes()
        .first()
        .copied()
        .filter(|byte| matches!(*byte, b'"' | b'\''));
    let mut compiled = String::with_capacity(tag_name.len() + authored.len() + 8);
    if let Some(quote) = quote {
        compiled.push(char::from(quote));
    }
    compiled.push_str("wui");
    let _ = write!(compiled, "{}", tag_name.len());
    compiled.push('-');
    compiled.push_str(tag_name);
    compiled.push('-');
    if let Some(quote) = quote {
        compiled.push_str(&authored[1..authored.len() - 1]);
        compiled.push(char::from(quote));
    } else {
        compiled.push_str(authored);
    }
    compiled
}

fn current_block(blocks: &[BlockKind]) -> BlockKind {
    blocks.last().copied().unwrap_or(BlockKind::Rules)
}

fn block_for_open(pending: &mut Option<(usize, BlockKind)>, index: usize) -> BlockKind {
    match *pending {
        Some((block_start, block)) if block_start == index => {
            *pending = None;
            block
        }
        _ => BlockKind::Style,
    }
}

fn is_animation_property(property: &str) -> bool {
    css_identifier_eq(property, "animation")
        || css_identifier_eq(property, "animation-name")
        || css_identifier_eq(property, "-webkit-animation")
        || css_identifier_eq(property, "-webkit-animation-name")
}

fn is_dynamic_css_function(name: &str) -> bool {
    name.eq_ignore_ascii_case("var")
        || name.eq_ignore_ascii_case("attr")
        || name.eq_ignore_ascii_case("env")
}

impl AnimationShorthandState {
    fn identifier_may_be_name(&mut self, name: &str, is_function: bool) -> bool {
        if is_function {
            if ["cubic-bezier", "linear", "steps"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function))
            {
                self.timing = true;
            } else if ["scroll", "view"]
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function))
            {
                self.timeline = true;
            }
            return false;
        }

        if ["inherit", "initial", "revert", "revert-layer", "unset"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            return false;
        }

        let slot = if [
            "ease",
            "ease-in",
            "ease-in-out",
            "ease-out",
            "linear",
            "step-end",
            "step-start",
        ]
        .iter()
        .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.timing)
        } else if name.eq_ignore_ascii_case("infinite") {
            Some(&mut self.iteration)
        } else if ["alternate", "alternate-reverse", "normal", "reverse"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.direction)
        } else if ["backwards", "both", "forwards", "none"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.fill)
        } else if ["paused", "running"]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            Some(&mut self.play)
        } else if name.eq_ignore_ascii_case("auto") {
            Some(&mut self.timeline)
        } else {
            return true;
        };

        if let Some(consumed) = slot {
            let was_consumed = *consumed;
            *consumed = true;
            was_consumed
        } else {
            true
        }
    }
}

fn validate_shadow_only_pseudos(tag_name: &str, source: &str, range: Range<usize>) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = range.start;
    while index < range.end {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index).min(range.end),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index).min(range.end);
            }
            b':' => {
                if let Some(pseudo) = pseudo_name(source, index) {
                    let raw_name = &source[pseudo.name.clone()];
                    let decoded_name = raw_name.contains('\\').then(|| identifier_value(raw_name));
                    let name = decoded_name.as_deref().unwrap_or(raw_name);
                    if !pseudo.is_element && name.eq_ignore_ascii_case("host-context") {
                        return Err(unsupported_light_css_error(
                            tag_name,
                            source,
                            index,
                            "`:host-context()` cannot be isolated in Light DOM",
                            "move the contextual rule to entry CSS, or wrap the component in `<template shadowrootmode=\"open\">`",
                        ));
                    }
                    if pseudo.is_element && name.eq_ignore_ascii_case("slotted") {
                        return Err(unsupported_light_css_error(
                            tag_name,
                            source,
                            index,
                            "`::slotted()` requires Shadow DOM slot projection",
                            "wrap the component in `<template shadowrootmode=\"open\">` to use `::slotted()`",
                        ));
                    }
                }
                index += 1;
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    Ok(())
}

fn validate_host_compound(tag_name: &str, source: &str, range: Range<usize>) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = range.start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut saw_token = false;
    let mut whitespace_after_token = false;
    while index < range.end {
        match bytes[index] {
            b'"' | b'\'' => {
                saw_token = true;
                whitespace_after_token = false;
                index = quoted_end(source, index).min(range.end);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index).min(range.end);
            }
            b'(' => {
                paren_depth += 1;
                saw_token = true;
                whitespace_after_token = false;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                saw_token = true;
                whitespace_after_token = false;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            b',' if paren_depth == 0 && bracket_depth == 0 => {
                saw_token = false;
                whitespace_after_token = false;
                index += 1;
            }
            b'>' | b'+' | b'~' if paren_depth == 0 && bracket_depth == 0 => {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            b'|' if paren_depth == 0
                && bracket_depth == 0
                && bytes.get(index + 1) == Some(&b'|') =>
            {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            byte if byte.is_ascii_whitespace() && paren_depth == 0 && bracket_depth == 0 => {
                whitespace_after_token |= saw_token;
                index += 1;
            }
            _ if paren_depth == 0 && bracket_depth == 0 && whitespace_after_token => {
                return Err(non_compound_host_error(tag_name, source, index));
            }
            _ => {
                saw_token = true;
                whitespace_after_token = false;
                index = next_char_boundary(source, index);
            }
        }
    }
    Ok(())
}

fn non_compound_host_error(tag_name: &str, source: &str, offset: usize) -> ParserError {
    unsupported_light_css_error(
        tag_name,
        source,
        offset,
        "`:host()` requires a compound selector",
        "remove descendant or child combinators from `:host(...)`, and place them after the closing parenthesis",
    )
}

fn skip_whitespace_and_comments(source: &str, mut index: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            index = block_comment_end(source, index);
        } else {
            return index;
        }
    }
}

fn has_significant_content(source: &str, range: Range<usize>) -> bool {
    let bytes = source.as_bytes();
    let mut index = range.start;
    while index < range.end {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            index = block_comment_end(source, index).min(range.end);
        } else {
            return true;
        }
    }
    false
}

fn at_rule_terminator(source: &str, start: usize) -> Option<(usize, u8)> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = quoted_end(source, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(source, index);
            }
            b'(' => {
                paren_depth += 1;
                index += 1;
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += 1;
            }
            b'[' => {
                bracket_depth += 1;
                index += 1;
            }
            b']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += 1;
            }
            byte @ (b'{' | b';') if paren_depth == 0 && bracket_depth == 0 => {
                return Some((index, byte));
            }
            _ => index = next_char_boundary(source, index),
        }
    }
    None
}

#[cold]
#[inline(never)]
fn unsupported_light_css_error(
    tag_name: &str,
    source: &str,
    offset: usize,
    title: &str,
    help: &str,
) -> ParserError {
    Diagnostic::error(title)
        .code(codes::UNSUPPORTED_LIGHT_CSS)
        .component(tag_name)
        .at_offset(source, offset)
        .snippet(super::source_line_snippet(source, offset))
        .help(help)
        .into()
}

#[cold]
#[inline(never)]
fn dynamic_keyframe_error(tag_name: &str, source: &str, offset: usize) -> ParserError {
    Diagnostic::error("dynamic keyframe references cannot be namespaced for Light DOM")
        .code(codes::DYNAMIC_LIGHT_KEYFRAME)
        .component(tag_name)
        .at_offset(source, offset)
        .snippet(super::source_line_snippet(source, offset))
        .help("use a static animation name, or move the animation to entry CSS")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "data-wl-t3stid";

    fn compile(source: &str) -> Result<String> {
        super::compile("my-card", LightScope::Stamped { marker: MARKER }, source)
    }

    fn enclose(source: &str) -> Result<String> {
        super::compile("my-card", LightScope::Enclosed, source)
    }

    /// Modern CSS constructs whose preludes are *not* selector lists, plus the
    /// selector shapes they nest around.
    ///
    /// A nested at-rule inherits the enclosing style block's kind, so a scoping
    /// pass that keys off block kind alone will happily qualify `@media` as if
    /// it were a compound and destroy the rule. Every entry pins the exact
    /// output so that regression cannot return silently.
    const MODERN_CSS_CORPUS: &[(&str, &str)] = &[
        (
            ".a { color: red; @media (min-width: 1px) { color: blue } }",
            ".a:where([data-wl-t3stid]) { color: red; @media (min-width: 1px) { color: blue } }",
        ),
        (
            ".a { @supports (display: grid) { color: blue } }",
            ".a:where([data-wl-t3stid]) { @supports (display: grid) { color: blue } }",
        ),
        (
            ".a { @container (min-width: 1px) { color: blue } }",
            ".a:where([data-wl-t3stid]) { @container (min-width: 1px) { color: blue } }",
        ),
        (
            ".a { @layer overrides { color: blue } }",
            ".a:where([data-wl-t3stid]) { @layer overrides { color: blue } }",
        ),
        (
            ".a { @starting-style { opacity: 0 } }",
            ".a:where([data-wl-t3stid]) { @starting-style { opacity: 0 } }",
        ),
        (
            ".a { @media print { .b { color: blue } } }",
            ".a:where([data-wl-t3stid]) { @media print { .b:where([data-wl-t3stid]) { color: blue } } }",
        ),
        (
            ".a { .b { @media print { color: red } } }",
            ".a:where([data-wl-t3stid]) { .b:where([data-wl-t3stid]) { @media print { color: red } } }",
        ),
        // A nested `@scope` prelude is a selector list, so it is qualified —
        // but through the prelude-aware path, not as a bare compound.
        (
            ".a { @scope (.b) to (.c) { .d { color: red } } }",
            ".a:where([data-wl-t3stid]) { @scope (.b:where([data-wl-t3stid])) to (.c:where([data-wl-t3stid])) { .d:where([data-wl-t3stid]) { color: red } } }",
        ),
        // A custom property value is an arbitrary token stream and may hold
        // braces; it must not be mistaken for a nested rule.
        (
            ".a { --x: { color: red }; }",
            ".a:where([data-wl-t3stid]) { --x: { color: red }; }",
        ),
        (
            ".a:has(> .b) { color: red }",
            ".a:has(> .b):where([data-wl-t3stid]) { color: red }",
        ),
        (
            "my-child::part(label) { color: red }",
            "my-child:where([data-wl-t3stid])::part(label) { color: red }",
        ),
        (
            ".a:nth-child(2 of .b) { color: red }",
            ".a:nth-child(2 of .b):where([data-wl-t3stid]) { color: red }",
        ),
        (
            "[data-x=\"A\" i] { color: red }",
            "[data-x=\"A\" i]:where([data-wl-t3stid]) { color: red }",
        ),
        (
            ".a > * { color: red }",
            ".a:where([data-wl-t3stid]) > *:where([data-wl-t3stid]) { color: red }",
        ),
    ];

    /// An at-rule prelude is never a selector list, at any nesting depth.
    #[test]
    fn never_qualifies_an_at_rule_prelude() {
        for (source, expected) in MODERN_CSS_CORPUS {
            assert_eq!(
                &compile(source).expect("compile"),
                expected,
                "input: {source}"
            );
        }
    }

    /// Stamping may only *insert* qualifiers — it may never otherwise alter the
    /// authored bytes.
    ///
    /// Deleting every qualifier the stamper inserted must reproduce the
    /// enclosed shape's body exactly. This catches dropped, duplicated, or
    /// reordered source bytes, but *not* a qualifier inserted in the wrong
    /// place: stripping it would undo the mistake. Placement is covered by
    /// [`never_qualifies_an_at_rule_prelude`] and
    /// [`no_qualifier_lands_in_an_at_rule_prelude`].
    #[test]
    fn stamping_only_inserts_qualifiers() {
        let qualifier = format!(":where([{MARKER}])");
        for (source, _) in MODERN_CSS_CORPUS {
            assert!(
                !source.contains(":host"),
                "host lowering differs between shapes: {source}"
            );
            let stripped = compile(source).expect("compile").replace(&qualifier, "");
            let enclosed = enclose(source).expect("enclose");
            let body = enclosed
                .trim_start_matches(|ch| ch != '\n')
                .trim_start_matches('\n')
                .trim_end_matches('}')
                .trim_end();
            assert_eq!(stripped, body, "input: {source}");
        }
    }

    /// No qualifier may land between an at-keyword and the `{` it opens.
    ///
    /// This is the postcondition that the block-kind bookkeeping exists to
    /// uphold, checked on the output rather than trusted from the input, so a
    /// new corpus entry is covered without anyone remembering to assert it.
    /// `@scope` is the sole exception: its prelude really is a selector list.
    #[test]
    fn no_qualifier_lands_in_an_at_rule_prelude() {
        let qualifier = format!(":where([{MARKER}])");
        for (source, _) in MODERN_CSS_CORPUS {
            let compiled = compile(source).expect("compile");
            for prelude in at_rule_preludes(&compiled) {
                assert!(
                    !prelude.contains(&qualifier),
                    "qualifier landed in at-rule prelude `{prelude}` for input: {source}"
                );
            }
        }
    }

    /// Every non-`@scope` at-rule prelude in `css`, from `@` to its `{` or `;`.
    fn at_rule_preludes(css: &str) -> Vec<&str> {
        let bytes = css.as_bytes();
        let mut preludes = Vec::new();
        let mut index = 0usize;
        while index < bytes.len() {
            match bytes[index] {
                b'"' | b'\'' => index = quoted_end(css, index),
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    index = block_comment_end(css, index);
                }
                b'@' => {
                    let (name, keyword_end) = at_keyword(css, index);
                    let end = css[keyword_end..]
                        .find(['{', ';'])
                        .map_or(css.len(), |at| keyword_end + at);
                    if !css_identifier_eq(name, "scope") {
                        preludes.push(&css[index..end]);
                    }
                    index = end.max(index + 1);
                }
                _ => index = next_char_boundary(css, index),
            }
        }
        preludes
    }

    #[test]
    fn stamps_every_top_level_compound() {
        let css = compile(".label { color: red; }").expect("compile");
        assert_eq!(css, ".label:where([data-wl-t3stid]) { color: red; }");
    }

    /// An ancestor outside the component must not be able to satisfy a
    /// leading compound, so every compound carries the marker.
    #[test]
    fn stamps_descendant_and_grouped_compounds() {
        let css = compile(".a .b > .c, .d + .e ~ .f { color: red }").expect("compile");
        assert_eq!(
            css,
            concat!(
                ".a:where([data-wl-t3stid]) .b:where([data-wl-t3stid])",
                " > .c:where([data-wl-t3stid]),",
                " .d:where([data-wl-t3stid]) + .e:where([data-wl-t3stid])",
                " ~ .f:where([data-wl-t3stid]) { color: red }"
            )
        );
    }

    /// A qualifier must precede the pseudo-element, and functional
    /// pseudo-class arguments are matched globally, exactly as they were
    /// under the previous `@scope` shape.
    #[test]
    fn stamps_before_pseudo_elements_and_never_inside_functions() {
        let css =
            compile(".a::before { color:red } .b:is(.c, .d):hover { color:red }").expect("compile");
        assert!(css.contains(".a:where([data-wl-t3stid])::before"));
        assert!(css.contains(".b:is(.c, .d):hover:where([data-wl-t3stid])"));
    }

    #[test]
    fn stamps_nested_rules_whose_selector_resembles_a_declaration() {
        let css = compile(".card { color:red; div:hover { color:blue } }").expect("compile");
        assert!(css.contains(".card:where([data-wl-t3stid]) { color:red;"));
        assert!(css.contains("div:hover:where([data-wl-t3stid]) { color:blue }"));
    }

    /// `&` already resolves through the qualified parent selector.
    #[test]
    fn leaves_nesting_parent_selectors_unqualified() {
        let css = compile(".card { &:hover { color:red } }").expect("compile");
        assert!(css.contains("&:hover { color:red }"));
    }

    #[test]
    fn lowers_composed_host_selectors() {
        let source = concat!(
            ":host { display:block }",
            ":host:has(.error) { color:red }",
            ":host([disabled]):hover { opacity:.5 }",
            ":host:not([hidden])::before { content:\"x\" }",
            r":h\6fst(.escaped) { color:blue }"
        );
        let css = compile(source).expect("compile");
        assert!(css.contains("my-card[data-wl] { display:block }"));
        assert!(css.contains("my-card[data-wl]:has(.error) { color:red }"));
        assert!(css.contains("my-card[data-wl]:is([disabled]):hover { opacity:.5 }"));
        assert!(css.contains("my-card[data-wl]:not([hidden])::before { content:\"x\" }"));
        assert!(css.contains("my-card[data-wl]:is(.escaped) { color:blue }"));
    }

    /// The host owns the `data-wl` marker, never the descendant marker.
    #[test]
    fn never_stamps_the_host_compound() {
        let css = compile(":host .label { color:red }").expect("compile");
        assert_eq!(
            css,
            "my-card[data-wl] .label:where([data-wl-t3stid]) { color:red }"
        );
    }

    /// A component that can render markup the compiler never sees keeps the
    /// native enclosure, which resolves membership at match time and therefore
    /// covers elements no template declares.
    #[test]
    fn encloses_rules_when_the_dom_is_not_build_time_known() {
        let css = enclose(".label p { color:red }").expect("compile");
        assert_eq!(
            css,
            concat!(
                "@scope (my-card[data-wl]) to (:scope [data-wl] > *) {\n",
                ".label p { color:red }\n}"
            )
        );
    }

    /// `:host` stays the cross-mode host abstraction in both shapes; only what
    /// it lowers to differs.
    #[test]
    fn encloses_host_selectors_as_scope() {
        let source = concat!(
            ":host { display:block }",
            ":host([disabled]):hover { opacity:.5 }"
        );
        let css = enclose(source).expect("compile");
        assert!(css.contains(":scope { display:block }"));
        assert!(css.contains(":scope:is([disabled]):hover { opacity:.5 }"));
        assert!(!css.contains("my-card[data-wl] {"));
    }

    /// Keyframe namespacing is independent of the boundary shape: neither
    /// `@scope` nor stamping isolates `@keyframes`.
    #[test]
    fn namespaces_keyframes_in_both_shapes() {
        let source = "@keyframes fade{to{opacity:1}}.x{animation:fade 1s}";
        for css in [compile(source), enclose(source)] {
            let css = css.expect("compile");
            assert!(css.contains("@keyframes wui7-my-card-fade"));
            assert!(css.contains("animation:wui7-my-card-fade 1s"));
        }
    }

    #[test]
    fn leaves_host_text_in_strings_and_comments() {
        let source =
            ".x::before{content:\":host\";--selector: :host;background:url(:host)}/* :host */";
        let css = compile(source).expect("compile");
        assert!(css.contains("content:\":host\""));
        assert!(css.contains("--selector: :host"));
        assert!(css.contains("url(:host)"));
        assert!(css.contains("/* :host */"));
    }

    #[test]
    fn preserves_nested_grouping_rules() {
        let source =
            "@media (width > 10px) { @supports (display:grid) { :host { display:grid } } }";
        let css = compile(source).expect("compile");
        assert!(css.contains("@media (width > 10px)"));
        assert!(css.contains("@supports (display:grid)"));
        assert!(css.contains("my-card[data-wl] { display:grid }"));
    }

    /// Inside an authored `@scope`, `:scope` keeps its platform meaning and is
    /// left alone, while `:host` still names the component host.
    #[test]
    fn preserves_authored_scope_and_non_ascii_selectors() {
        let source = "@scope (.é) { @layer card { :host(.wide) > .标题 { color:red } } }";
        let css = compile(source).expect("compile");
        assert!(css.contains("@scope (.é:where([data-wl-t3stid]))"));
        assert!(css.contains("my-card[data-wl]:is(.wide) > .标题:where([data-wl-t3stid])"));
    }

    #[test]
    fn stamps_both_ends_of_an_authored_scope_prelude() {
        let source = "@scope (.root) to (.limit) { .x { color:red } }";
        let css = compile(source).expect("compile");
        assert!(css.contains(
            "@scope (.root:where([data-wl-t3stid])) to (.limit:where([data-wl-t3stid]))"
        ));
        assert!(css.contains(".x:where([data-wl-t3stid]) { color:red }"));
    }

    /// A bare `:scope` outside an authored `@scope` used to resolve to the
    /// compiler's generated scoping root, so it must keep naming the host.
    #[test]
    fn lowers_top_level_scope_to_the_host() {
        let css = compile(":scope .label { color:red }").expect("compile");
        assert_eq!(
            css,
            "my-card[data-wl] .label:where([data-wl-t3stid]) { color:red }"
        );
    }

    #[test]
    fn rejects_shadow_only_selectors() {
        for selector in [
            ":host-context(body)",
            "::slotted(*)",
            ":HOST-CONTEXT(main)",
            ":host(::slotted(*))",
        ] {
            let error =
                compile(&format!("{selector}{{color:red}}")).expect_err("selector must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
            ));
        }
    }

    #[test]
    fn rejects_non_compound_host_argument() {
        let error = compile(":host(.card > .label){color:red}").expect_err("must fail");
        assert!(matches!(
            error,
            ParserError::Template(ref diagnostic)
                if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
                    && diagnostic.help_text().is_some()
        ));
    }

    #[test]
    fn namespaces_keyframes_and_static_references() {
        let source = concat!(
            "@keyframes fade { from { opacity:0 } to { opacity:1 } }",
            ".x { animation: fade 1s ease; animation-name: fade; }"
        );
        let css = compile(source).expect("compile");
        assert!(css.contains("@keyframes wui7-my-card-fade"));
        assert!(css.contains("animation: wui7-my-card-fade 1s ease"));
        assert!(css.contains("animation-name: wui7-my-card-fade"));
    }

    /// Keyframe selectors are not element selectors and must never be stamped.
    #[test]
    fn never_stamps_keyframe_selectors() {
        let css =
            compile("@keyframes fade { from { opacity:0 } 50% { opacity:.5 } to { opacity:1 } }")
                .expect("compile");
        assert!(!css.contains(MARKER));
    }

    #[test]
    fn rewrites_comma_separated_and_vendor_animation_references() {
        let source = concat!(
            "@keyframes fade{to{opacity:1}}",
            "@-webkit-keyframes spin{to{rotate:1turn}}",
            ".x{-webkit-animation:spin 1s, fade 2s ease;",
            "animation-name:fade, spin;content:\"fade\";--animation:fade}"
        );
        let css = compile(source).expect("compile");
        assert!(css.contains("-webkit-animation:wui7-my-card-spin 1s, wui7-my-card-fade 2s ease"));
        assert!(css.contains("animation-name:wui7-my-card-fade, wui7-my-card-spin"));
        assert!(css.contains("content:\"fade\";--animation:fade"));
    }

    #[test]
    fn rewrites_quoted_keyframe_names_only_in_animation_syntax() {
        let source = concat!(
            "@keyframes \"fade in\"{to{opacity:1}}",
            ".x{animation-name:'fade in';content:\"fade in\"}"
        );
        let css = compile(source).expect("compile");
        assert!(css.contains("@keyframes \"wui7-my-card-fade in\""));
        assert!(css.contains("animation-name:\"wui7-my-card-fade in\""));
        assert!(css.contains("content:\"fade in\""));
    }

    #[test]
    fn matches_equivalent_escaped_keyframe_identifiers() {
        let source = r"@keyframes f\61 de{to{opacity:1}}.x{animation-name:fa\64 e}";
        let css = compile(source).expect("compile");
        assert!(css.contains(r"@keyframes wui7-my-card-f\61 de"));
        assert!(css.contains(r"animation-name:wui7-my-card-f\61 de"));
    }

    #[test]
    fn does_not_rewrite_shorthand_keywords_or_dimension_units() {
        let source = concat!(
            "@keyframes linear{to{opacity:1}}",
            "@keyframes s{to{opacity:1}}",
            ".x{animation:other 1s linear;",
            "animation:1s linear linear;",
            "animation-name:linear,s}"
        );
        let css = compile(source).expect("compile");
        assert!(css.contains("animation:other 1s linear"));
        assert!(css.contains("animation:1s linear wui7-my-card-linear"));
        assert!(css.contains("animation-name:wui7-my-card-linear,wui7-my-card-s"));
    }

    /// Stamping does not isolate `@keyframes`, so two components must never
    /// compile the same animation name.
    #[test]
    fn keyframe_names_never_collide_across_components() {
        let outer = super::compile(
            "x-foo",
            LightScope::Stamped { marker: MARKER },
            "@keyframes bar-baz{to{opacity:1}}",
        )
        .expect("compile");
        let inner = super::compile(
            "x-foo-bar",
            LightScope::Stamped { marker: MARKER },
            "@keyframes baz{to{opacity:1}}",
        )
        .expect("compile");
        assert!(outer.contains("@keyframes wui5-x-foo-bar-baz"));
        assert!(inner.contains("@keyframes wui9-x-foo-bar-baz"));
    }

    #[test]
    fn rejects_dynamic_keyframe_references() {
        for function in ["var(--animation)", "ATTR(data-animation)"] {
            let source = format!("@keyframes fade{{to{{opacity:1}}}}.x{{animation:{function}}}");
            let error = compile(&source).expect_err("dynamic reference must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::DYNAMIC_LIGHT_KEYFRAME)
                        && diagnostic.component_name() == Some("my-card")
                        && diagnostic.help_text().is_some()
                        && diagnostic.snippet_text().is_some()
                        && diagnostic.position_line_column().is_some()
            ));
        }
    }

    #[test]
    fn allows_dynamic_global_animation_without_local_keyframes() {
        let css = compile(".x{animation:var(--animation)}").expect("compile");
        assert!(css.contains("animation:var(--animation)"));
    }

    #[test]
    fn rejects_global_at_rules() {
        for source in [
            "@font-face{font-family:x;src:url(x)}",
            "@layer reset;",
            "@import url(global.css);",
        ] {
            let error = compile(source).expect_err("must fail");
            assert!(matches!(
                error,
                ParserError::Template(ref diagnostic)
                    if diagnostic.error_code() == Some(codes::UNSUPPORTED_LIGHT_CSS)
            ));
        }
    }

    /// Everything outside a selector prelude is copied through untouched.
    #[test]
    fn preserves_surrounding_source_bytes() {
        let css = compile(" \n:host { color:red }\n ").expect("compile");
        assert_eq!(css, " \nmy-card[data-wl] { color:red }\n ");
    }
}
