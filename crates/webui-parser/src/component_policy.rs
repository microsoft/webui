// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time component rendering and hydration policy.

use crate::diagnostic::{codes, Diagnostic};
use crate::html_parser::{find_comment_close, find_declaration_close, parse_tag, Attr, Tag};
use crate::{ParserError, Result};

pub(crate) const RENDER_ATTR: &str = "w-render";
pub(crate) const HYDRATE_ATTR: &str = "w-hydrate";
pub(crate) const RESERVE_BLOCK_SIZE_ATTR: &str = "w-reserve-block-size";

/// Shared opt-in value for both policy attributes.
///
/// Deliberately vague, mirroring `loading="lazy"`: activation blends a viewport
/// lead margin, interaction, and the browser's own relevance signal, so a
/// concrete name such as `visible` would overclaim and freeze the heuristic.
const POLICY_LAZY: &str = "lazy";
const INSTANCE_EAGER: &str = "eager";

struct InvalidPolicyValue<'a> {
    name: &'a str,
    value: &'a str,
    expected: &'a str,
}

enum NestedPolicyIssue<'a> {
    Misplaced {
        tag_offset: usize,
        attr: Attr<'a>,
    },
    Duplicate {
        tag_offset: usize,
        tag_name: &'a str,
        attr: Attr<'a>,
    },
}

/// Compiler-owned component policy encoded into client template metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ComponentRenderPolicy {
    Eager,
    LazyHydration,
    LazyRender { reserve_block_size: String },
}

impl ComponentRenderPolicy {
    #[inline]
    pub(crate) fn metadata_code(&self) -> Option<u8> {
        match self {
            Self::Eager => None,
            Self::LazyHydration => Some(1),
            Self::LazyRender { .. } => Some(2),
        }
    }

    #[inline]
    pub(crate) fn is_authored(&self) -> bool {
        !matches!(self, Self::Eager)
    }

    #[inline]
    pub(crate) fn reserve_block_size(&self) -> Option<&str> {
        match self {
            Self::LazyRender { reserve_block_size } => Some(reserve_block_size),
            Self::Eager | Self::LazyHydration => None,
        }
    }

    pub(crate) fn append_declarations(&self, output: &mut String) {
        let Self::LazyRender { reserve_block_size } = self else {
            return;
        };
        output.push_str("content-visibility:auto;contain-intrinsic-block-size:auto ");
        output.push_str(reserve_block_size);
        output.push(';');
    }

    pub(crate) fn append_scoped_css(&self, output: &mut String, tag_name: &str) {
        if !matches!(self, Self::LazyRender { .. }) {
            return;
        }
        output.push_str(":host(");
        output.push_str(tag_name);
        output.push_str(r#":not([w-render="eager"])),"#);
        output.push_str(tag_name);
        output.push_str(r#":not([w-render="eager"]){"#);
        self.append_declarations(output);
        output.push('}');
    }
}

/// Parse policy attributes from an authored root `<template>`.
pub(crate) fn parse_component_render_policy(
    component: &str,
    html: &str,
) -> Result<ComponentRenderPolicy> {
    let trimmed = html.trim_start();
    let source_offset = html.len() - trimmed.len();
    let Some(tag) = parse_tag(trimmed) else {
        return Ok(ComponentRenderPolicy::Eager);
    };

    let mut render = None;
    let mut hydrate = None;
    let mut reserve = None;
    for attr in tag.attrs() {
        match attr.name {
            RENDER_ATTR => set_directive(component, html, source_offset, &mut render, attr)?,
            HYDRATE_ATTR => set_directive(component, html, source_offset, &mut hydrate, attr)?,
            RESERVE_BLOCK_SIZE_ATTR => {
                set_directive(component, html, source_offset, &mut reserve, attr)?;
            }
            _ => {}
        }
    }

    let has_root_policy = render.is_some() || hydrate.is_some() || reserve.is_some();
    if has_root_policy && (tag.name != "template" || tag.closing) {
        let attr = render.as_ref().or(hydrate.as_ref()).or(reserve.as_ref());
        return Err(misplaced_policy_error(component, html, source_offset, attr));
    }
    if let Some(issue) = find_nested_policy_issue(html, source_offset + tag.close + 1) {
        return Err(match issue {
            NestedPolicyIssue::Misplaced { tag_offset, attr } => {
                misplaced_policy_error(component, html, tag_offset, Some(&attr))
            }
            NestedPolicyIssue::Duplicate {
                tag_offset,
                tag_name,
                attr,
            } => duplicate_instance_override_error(component, html, tag_offset, tag_name, &attr),
        });
    }
    if !has_root_policy {
        return Ok(ComponentRenderPolicy::Eager);
    }

    let render_value = directive_value(component, html, source_offset, render.as_ref())?;
    let hydrate_value = directive_value(component, html, source_offset, hydrate.as_ref())?;
    let reserve_value = directive_value(component, html, source_offset, reserve.as_ref())?;

    if let Some(value) = render_value {
        if value != POLICY_LAZY {
            return Err(invalid_policy_value(
                component,
                html,
                source_offset,
                render.as_ref(),
                InvalidPolicyValue {
                    name: RENDER_ATTR,
                    value,
                    expected: POLICY_LAZY,
                },
            ));
        }
    }
    if let Some(value) = hydrate_value {
        if value != POLICY_LAZY {
            return Err(invalid_policy_value(
                component,
                html,
                source_offset,
                hydrate.as_ref(),
                InvalidPolicyValue {
                    name: HYDRATE_ATTR,
                    value,
                    expected: POLICY_LAZY,
                },
            ));
        }
    }
    if render_value.is_some() && hydrate_value.is_some() {
        return Err(conflicting_policy_error(
            component,
            html,
            source_offset,
            hydrate.as_ref(),
        ));
    }

    if render_value.is_some() {
        let Some(value) = reserve_value else {
            return Err(missing_reservation_error(
                component,
                html,
                source_offset,
                render.as_ref(),
            ));
        };
        if !is_non_negative_css_length(value) {
            return Err(invalid_reservation_error(
                component,
                html,
                source_offset,
                reserve.as_ref(),
                value,
            ));
        }
        return Ok(ComponentRenderPolicy::LazyRender {
            reserve_block_size: value.to_string(),
        });
    }

    if let Some(value) = reserve_value {
        return Err(unused_reservation_error(
            component,
            html,
            source_offset,
            reserve.as_ref(),
            value,
        ));
    }
    if hydrate_value.is_some() {
        return Ok(ComponentRenderPolicy::LazyHydration);
    }
    Ok(ComponentRenderPolicy::Eager)
}

fn find_nested_policy_issue(source: &str, mut cursor: usize) -> Option<NestedPolicyIssue<'_>> {
    while cursor < source.len() {
        cursor += source[cursor..].find('<')?;
        let remaining = &source[cursor..];
        if remaining.starts_with("<!--") {
            cursor += find_comment_close(remaining).unwrap_or(remaining.len());
            continue;
        }
        if remaining.starts_with("<!") {
            cursor += find_declaration_close(remaining).unwrap_or(remaining.len());
            continue;
        }
        let Some(tag) = parse_tag(remaining) else {
            cursor += 1;
            continue;
        };
        if !tag.closing {
            if let Some(issue) = nested_policy_issue(cursor, &tag) {
                return Some(issue);
            }
            if is_raw_text_element(tag.name) {
                cursor += find_raw_text_end(remaining, tag.name, tag.close + 1);
                continue;
            }
        }
        cursor += tag.close + 1;
    }
    None
}

fn nested_policy_issue<'a>(tag_offset: usize, tag: &Tag<'a>) -> Option<NestedPolicyIssue<'a>> {
    let mut saw_render = false;
    let mut saw_hydrate = false;
    for attr in tag.attrs() {
        let seen = match attr.name {
            RENDER_ATTR => &mut saw_render,
            HYDRATE_ATTR => &mut saw_hydrate,
            RESERVE_BLOCK_SIZE_ATTR => {
                return Some(NestedPolicyIssue::Misplaced { tag_offset, attr });
            }
            _ => continue,
        };
        if *seen {
            return Some(NestedPolicyIssue::Duplicate {
                tag_offset,
                tag_name: tag.name,
                attr,
            });
        }
        *seen = true;
        if attr.value.map(str::trim) != Some(INSTANCE_EAGER) {
            return Some(NestedPolicyIssue::Misplaced { tag_offset, attr });
        }
    }
    None
}

fn is_raw_text_element(tag_name: &str) -> bool {
    tag_name.eq_ignore_ascii_case("script")
        || tag_name.eq_ignore_ascii_case("style")
        || tag_name.eq_ignore_ascii_case("textarea")
        || tag_name.eq_ignore_ascii_case("title")
        || tag_name.eq_ignore_ascii_case("xmp")
        || tag_name.eq_ignore_ascii_case("iframe")
        || tag_name.eq_ignore_ascii_case("noembed")
        || tag_name.eq_ignore_ascii_case("noframes")
        || tag_name.eq_ignore_ascii_case("noscript")
        || tag_name.eq_ignore_ascii_case("plaintext")
}

fn find_raw_text_end(source: &str, tag_name: &str, mut cursor: usize) -> usize {
    if tag_name.eq_ignore_ascii_case("plaintext") {
        return source.len();
    }
    while cursor < source.len() {
        let Some(relative) = source[cursor..].find('<') else {
            return source.len();
        };
        cursor += relative;
        if let Some(tag) = parse_tag(&source[cursor..]) {
            if tag.closing && tag.name.eq_ignore_ascii_case(tag_name) {
                return cursor + tag.close + 1;
            }
        }
        cursor += 1;
    }
    source.len()
}

fn set_directive<'a>(
    component: &str,
    source: &str,
    source_offset: usize,
    slot: &mut Option<Attr<'a>>,
    attr: Attr<'a>,
) -> Result<()> {
    if slot.is_some() {
        return Err(duplicate_policy_error(
            component,
            source,
            source_offset,
            &attr,
        ));
    }
    *slot = Some(attr);
    Ok(())
}

fn directive_value<'a>(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'a>>,
) -> Result<Option<&'a str>> {
    let Some(attr) = attr else {
        return Ok(None);
    };
    let Some(value) = attr.value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(missing_policy_value(component, source, source_offset, attr));
    };
    Ok(Some(value))
}

fn is_non_negative_css_length(value: &str) -> bool {
    if value == "0" {
        return true;
    }

    let bytes = value.as_bytes();
    let mut cursor = 0usize;
    let mut digits = 0usize;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
        digits += 1;
    }
    if cursor < bytes.len() && bytes[cursor] == b'.' {
        cursor += 1;
        let fraction_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
            digits += 1;
        }
        if cursor == fraction_start {
            return false;
        }
    }
    if digits == 0 || cursor == bytes.len() {
        return false;
    }

    matches!(
        &value[cursor..],
        "px" | "em"
            | "rem"
            | "ex"
            | "rex"
            | "cap"
            | "rcap"
            | "ch"
            | "rch"
            | "ic"
            | "ric"
            | "lh"
            | "rlh"
            | "vw"
            | "vh"
            | "vi"
            | "vb"
            | "vmin"
            | "vmax"
            | "svw"
            | "svh"
            | "svi"
            | "svb"
            | "svmin"
            | "svmax"
            | "lvw"
            | "lvh"
            | "lvi"
            | "lvb"
            | "lvmin"
            | "lvmax"
            | "dvw"
            | "dvh"
            | "dvi"
            | "dvb"
            | "dvmin"
            | "dvmax"
            | "cqw"
            | "cqh"
            | "cqi"
            | "cqb"
            | "cqmin"
            | "cqmax"
            | "cm"
            | "mm"
            | "q"
            | "in"
            | "pc"
            | "pt"
    )
}

#[cold]
#[inline(never)]
fn duplicate_policy_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: &Attr<'_>,
) -> ParserError {
    Diagnostic::error(format!("duplicate `{}` component directive", attr.name))
        .code(codes::INVALID_COMPONENT_RENDER_POLICY)
        .component(component)
        .element("template")
        .snippet(attr.raw)
        .at_offset(source, source_offset + attr.raw_range.start)
        .help(format!("keep exactly one `{}` attribute", attr.name))
        .into()
}

#[cold]
#[inline(never)]
fn duplicate_instance_override_error(
    component: &str,
    source: &str,
    tag_offset: usize,
    tag_name: &str,
    attr: &Attr<'_>,
) -> ParserError {
    Diagnostic::error(format!("duplicate `{}` instance override", attr.name))
        .code(codes::INVALID_COMPONENT_RENDER_POLICY)
        .component(component)
        .element(tag_name)
        .snippet(attr.raw)
        .at_offset(source, tag_offset + attr.raw_range.start)
        .help(format!(
            "keep exactly one `{}` attribute on <{tag_name}>",
            attr.name
        ))
        .into()
}

#[cold]
#[inline(never)]
fn misplaced_policy_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
) -> ParserError {
    let snippet = attr.map_or(RENDER_ATTR, |value| value.raw);
    let offset = attr.map_or(source_offset, |value| source_offset + value.raw_range.start);
    Diagnostic::error("component rendering directive is not on the root <template>")
        .code(codes::INVALID_COMPONENT_RENDER_POLICY)
        .component(component)
        .snippet(snippet)
        .at_offset(source, offset)
        .help("put rendering directives on one root `<template ...>` wrapper")
        .into()
}

#[cold]
#[inline(never)]
fn missing_policy_value(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: &Attr<'_>,
) -> ParserError {
    Diagnostic::error(format!("missing value for `{}`", attr.name))
        .code(codes::INVALID_COMPONENT_RENDER_POLICY)
        .component(component)
        .element("template")
        .snippet(attr.raw)
        .at_offset(source, source_offset + attr.raw_range.start)
        .help(match attr.name {
            RENDER_ATTR => "use `w-render=\"lazy\"`",
            HYDRATE_ATTR => "use `w-hydrate=\"lazy\"`",
            _ => "provide a non-negative CSS length such as `18rem`",
        })
        .into()
}

#[cold]
#[inline(never)]
fn invalid_policy_value(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
    invalid: InvalidPolicyValue<'_>,
) -> ParserError {
    let offset = attr.map_or(source_offset, |item| source_offset + item.raw_range.start);
    Diagnostic::error(format!(
        "invalid `{}` value `{}`",
        invalid.name, invalid.value
    ))
    .code(codes::INVALID_COMPONENT_RENDER_POLICY)
    .component(component)
    .element("template")
    .snippet(attr.map_or(invalid.name, |item| item.raw))
    .at_offset(source, offset)
    .help(format!("use `{}=\"{}\"`", invalid.name, invalid.expected))
    .into()
}

#[cold]
#[inline(never)]
fn conflicting_policy_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
) -> ParserError {
    let offset = attr.map_or(source_offset, |item| source_offset + item.raw_range.start);
    Diagnostic::error("component declares both rendering and hydration policies")
        .code(codes::INVALID_COMPONENT_RENDER_POLICY)
        .component(component)
        .element("template")
        .snippet(attr.map_or(HYDRATE_ATTR, |item| item.raw))
        .at_offset(source, offset)
        .help(format!(
            "remove `{HYDRATE_ATTR}`; `{RENDER_ATTR}=\"{POLICY_LAZY}\"` already includes lazy hydration"
        ))
        .into()
}

#[cold]
#[inline(never)]
fn missing_reservation_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
) -> ParserError {
    let offset = attr.map_or(source_offset, |item| source_offset + item.raw_range.start);
    Diagnostic::error("lazy rendering needs an intrinsic block-size reservation")
        .code(codes::MISSING_RENDER_RESERVATION)
        .component(component)
        .element("template")
        .snippet(attr.map_or(RENDER_ATTR, |item| item.raw))
        .at_offset(source, offset)
        .help("add `w-reserve-block-size=\"<length>\"`, using the component's typical rendered block size")
        .into()
}

#[cold]
#[inline(never)]
fn invalid_reservation_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
    value: &str,
) -> ParserError {
    let offset = attr.map_or(source_offset, |item| source_offset + item.raw_range.start);
    Diagnostic::error(format!(
        "invalid intrinsic block-size reservation `{value}`"
    ))
    .code(codes::INVALID_RENDER_RESERVATION)
    .component(component)
    .element("template")
    .snippet(attr.map_or(RESERVE_BLOCK_SIZE_ATTR, |item| item.raw))
    .at_offset(source, offset)
    .help("use one non-negative CSS length such as `72px`, `18rem`, or `40vh`")
    .into()
}

#[cold]
#[inline(never)]
fn unused_reservation_error(
    component: &str,
    source: &str,
    source_offset: usize,
    attr: Option<&Attr<'_>>,
    value: &str,
) -> ParserError {
    let offset = attr.map_or(source_offset, |item| source_offset + item.raw_range.start);
    Diagnostic::error(format!(
        "`{RESERVE_BLOCK_SIZE_ATTR}` has no effect without `{RENDER_ATTR}=\"{POLICY_LAZY}\"`"
    ))
    .code(codes::INVALID_RENDER_RESERVATION)
    .component(component)
    .element("template")
    .snippet(attr.map_or(value, |item| item.raw))
    .at_offset(source, offset)
    .help(format!(
        "add `{RENDER_ATTR}=\"{POLICY_LAZY}\"` or remove `{RESERVE_BLOCK_SIZE_ATTR}`"
    ))
    .into()
}

#[cfg(test)]
mod tests {
    use super::{is_non_negative_css_length, parse_component_render_policy, ComponentRenderPolicy};
    use crate::diagnostic::codes;
    use crate::ParserError;

    #[test]
    fn validates_supported_non_negative_lengths() {
        for value in ["0", "0px", ".5rem", "72px", "18rem", "40dvh", "25cqb"] {
            assert!(is_non_negative_css_length(value), "{value}");
        }
        for value in [
            "",
            "-1px",
            "auto",
            "50%",
            "calc(10px)",
            "1.px",
            "1px;display:none",
        ] {
            assert!(!is_non_negative_css_length(value), "{value}");
        }
    }

    #[test]
    fn rejects_policy_directives_below_the_root_template() {
        let result = parse_component_render_policy(
            "bad-row",
            concat!(
                "<template>\n",
                "  <div w-render=\"lazy\" w-reserve-block-size=\"72px\"></div>\n",
                "</template>",
            ),
        );
        let Err(ParserError::Template(diagnostic)) = result else {
            panic!("nested policy directive must fail");
        };
        assert_eq!(
            diagnostic.error_code(),
            Some(codes::INVALID_COMPONENT_RENDER_POLICY)
        );
        assert_eq!(
            diagnostic.position_line_column().map(|position| position.0),
            Some(2)
        );
    }

    #[test]
    fn ignores_policy_text_inside_comments() {
        let result = parse_component_render_policy(
            "plain-row",
            "<template><!-- <div w-render=\"lazy\"> --><p>row</p></template>",
        );
        assert!(matches!(result, Ok(ComponentRenderPolicy::Eager)));
    }

    #[test]
    fn ignores_policy_text_inside_raw_text_elements() {
        let result = parse_component_render_policy(
            "plain-row",
            concat!(
                "<template><script>",
                r#"const example = '<script><div w-render="lazy">';"#,
                "</script>",
                r#"<noscript><div w-render="lazy"></div></noscript>"#,
                "</template>",
            ),
        );
        assert!(matches!(result, Ok(ComponentRenderPolicy::Eager)));
    }

    #[test]
    fn permits_nested_eager_instance_overrides() {
        let result = parse_component_render_policy(
            "parent-row",
            concat!(
                "<template>",
                "<child-row w-hydrate=\"eager\"></child-row>",
                "<child-row w-render=\"eager\"></child-row>",
                "</template>",
            ),
        );
        assert!(matches!(result, Ok(ComponentRenderPolicy::Eager)));
    }

    #[test]
    fn rejects_duplicate_nested_eager_instance_overrides() {
        let result = parse_component_render_policy(
            "parent-row",
            concat!(
                "<template>",
                "<child-row w-render=\"eager\" w-render=\"eager\"></child-row>",
                "</template>",
            ),
        );
        let Err(ParserError::Template(diagnostic)) = result else {
            panic!("duplicate instance override must fail");
        };
        assert_eq!(
            diagnostic.error_code(),
            Some(codes::INVALID_COMPONENT_RENDER_POLICY)
        );
        assert!(diagnostic.title().contains("duplicate"));
    }

    #[test]
    fn rejects_conflicting_root_policies() {
        let result = parse_component_render_policy(
            "conflicted-row",
            concat!(
                "<template w-render=\"lazy\" w-reserve-block-size=\"72px\" ",
                "w-hydrate=\"lazy\"></template>",
            ),
        );
        let Err(ParserError::Template(diagnostic)) = result else {
            panic!("conflicting root policies must fail");
        };
        assert_eq!(
            diagnostic.error_code(),
            Some(codes::INVALID_COMPONENT_RENDER_POLICY)
        );
        assert!(diagnostic.help_text().is_some());
    }
}
