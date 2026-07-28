// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared FAST `<f-template>` source handling for the FAST parser plugins.
//!
//! This module owns everything FAST-specific about turning an authored
//! `<f-template>` component source into the WebUI parser view: scanning for the
//! single `<f-template>` element, resolving its `name` (falling back to the
//! filename-derived tag), invoking `microsoft-fast-convert`, retaining the
//! authored inner `<template>` for the client artifact, and constructing FAST
//! diagnostics. The FAST parser plugins expose it through the framework-neutral
//! [`ParserPlugin::component_source_transform`](super::ParserPlugin::component_source_transform)
//! hook; the parser core never references FAST.

use super::{ComponentSource, ComponentSourceResult, TransformedComponentSource};
use crate::diagnostic::{codes, Diagnostic};
use crate::html_parser::{Event, Walker};
use crate::{ParserError, Result};
use microsoft_fast_convert::{convert_template, ConvertError};
use std::borrow::Cow;
use std::ops::Range;

/// Diagnostic code for a component source with multiple `<f-template>` blocks.
const UNSUPPORTED_MULTIPLE_F_TEMPLATES: &str = "unsupported-multiple-f-templates";
/// Diagnostic code for a FAST template that cannot be converted to WebUI syntax.
const INVALID_FAST_TEMPLATE: &str = "invalid-fast-template";
/// Placeholder `<f-template name>` supplied to the converter when the authored
/// source omits a usable name; the resolved registry key is the filename.
const CONVERTER_FALLBACK_NAME: &str = "webui-fallback";
/// Target dialect requested from `microsoft-fast-convert`.
const WEBUI_CONVERTER_SYNTAX: &str = "webui-prerelease";
/// The FAST element name used by both the authoritative [`Walker`] match in
/// [`find_f_template_source`] and the cheap byte precheck in
/// [`contains_f_template_name`], preventing false negatives from name drift.
const F_TEMPLATE_NAME: &str = "f-template";

/// Located `<f-template>` element within an authored component source.
struct FTemplateSource {
    name: Option<String>,
    start: usize,
    inner: Range<usize>,
}

/// Component-source transform for FAST authored templates.
///
/// Returns [`ComponentSourceResult::Unchanged`] when the source contains no
/// `<f-template>`; otherwise converts the FAST declarative syntax to the WebUI
/// parser view, resolves the registry key from the `<f-template name>`
/// attribute (or the filename), and retains the authored inner `<template>` as
/// the client artifact.
///
/// A cheap byte precheck ([`contains_f_template_name`]) rules out sources that
/// cannot possibly contain an `<f-template>` element, so ordinary FAST-free
/// component sources never pay for the element walk.
///
/// # Errors
///
/// Returns a [`Diagnostic`]-carrying error when multiple `<f-template>` blocks
/// are present or the FAST template cannot be converted.
pub(crate) fn transform_component_source(
    source: ComponentSource<'_>,
) -> Result<ComponentSourceResult> {
    let html_content = source.html_content;
    if !contains_f_template_name(html_content.as_bytes()) {
        return Ok(ComponentSourceResult::Unchanged);
    }

    let Some(found) = find_f_template_source(html_content) else {
        return Ok(ComponentSourceResult::Unchanged);
    };

    let resolved_tag = found.name.as_deref().unwrap_or(source.tag_name).to_string();
    let converter_input = converter_input(html_content, &found);
    let converted = convert_template(&converter_input, WEBUI_CONVERTER_SYNTAX)
        .map_err(|error| converter_error(html_content, found.start, error))?;
    let artifact_content = html_content[found.inner].trim().to_string();

    Ok(ComponentSourceResult::Transformed(
        TransformedComponentSource {
            tag_name: resolved_tag,
            parser_content: converted,
            artifact_content: Some(artifact_content),
        },
    ))
}

/// Cheap case-insensitive precheck for the bare ASCII name `f-template`
/// anywhere in `haystack`, without the surrounding `<` delimiter.
///
/// The HTML [`Walker`] takes an opening element name to be the run of bytes
/// after `<` (skipping ASCII whitespace) and matches it
/// ASCII-case-insensitively, so every spelling it accepts contains these ten
/// bytes verbatim, case aside. Searching for the bare name therefore has no
/// false negatives, whereas searching for `"<f-template"` would. Non-ASCII
/// bytes are compared without case folding, so UTF-8 input can neither panic
/// nor be misread. False positives (the bytes appearing in text, comments,
/// attributes, or longer names) only cost the walk that would have happened
/// anyway: [`find_f_template_source`] remains authoritative and still reports
/// `Unchanged`.
#[inline]
fn contains_f_template_name(haystack: &[u8]) -> bool {
    haystack
        .windows(F_TEMPLATE_NAME.len())
        .any(|window| window.eq_ignore_ascii_case(F_TEMPLATE_NAME.as_bytes()))
}

/// Find the first `<f-template>` element anywhere in the authored source.
fn find_f_template_source(html_content: &str) -> Option<FTemplateSource> {
    let mut stack = Vec::with_capacity(1);
    stack.push(0..html_content.len());

    while let Some(range) = stack.pop() {
        for event in Walker::new_range(html_content, range.start, range.end) {
            let Event::Element(element) = event else {
                continue;
            };

            if element.name().eq_ignore_ascii_case(F_TEMPLATE_NAME) {
                return Some(FTemplateSource {
                    name: element
                        .attr("name")
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                    start: element.start,
                    inner: element.inner(),
                });
            }

            if !element.self_closing() && !element.is_void() {
                stack.push(element.inner());
            }
        }
    }

    None
}

/// Build the converter input, injecting a placeholder name when the authored
/// `<f-template>` omits a usable one so the converter's validation is satisfied
/// without leaking the placeholder into the resolved registry key.
fn converter_input<'a>(html_content: &'a str, source: &FTemplateSource) -> Cow<'a, str> {
    if source.name.is_some() {
        return Cow::Borrowed(html_content);
    }

    let mut normalized =
        String::with_capacity(html_content.len() + CONVERTER_FALLBACK_NAME.len() + 20);
    normalized.push_str(&html_content[..source.start]);
    normalized.push_str("<f-template name=\"");
    normalized.push_str(CONVERTER_FALLBACK_NAME);
    normalized.push_str("\">");
    normalized.push_str(&html_content[source.inner.start..]);
    Cow::Owned(normalized)
}

/// Build a [`Diagnostic`] error from a converter failure.
#[cold]
#[inline(never)]
fn converter_error(source: &str, offset: usize, error: ConvertError) -> ParserError {
    if matches!(error, ConvertError::MultipleFTemplates { .. }) {
        return Diagnostic::error("multiple <f-template> elements are not supported")
            .code(UNSUPPORTED_MULTIPLE_F_TEMPLATES)
            .at_offset(source, offset)
            .snippet("<f-template>")
            .help(
                "keep only one <f-template> per component file; multiple f-template blocks are not currently supported",
            )
            .into();
    }

    let help = match &error {
        ConvertError::MissingFTemplateName | ConvertError::EmptyFTemplateName => {
            "add a non-empty name attribute to <f-template>, for example <f-template name=\"my-component\">"
        }
        ConvertError::MissingInnerTemplate | ConvertError::MultipleInnerTemplates { .. } => {
            "keep exactly one inner <template> element inside <f-template>"
        }
        ConvertError::MissingValueAttribute { .. }
        | ConvertError::InvalidDirectiveValue { .. } => {
            "add the required value=\"{{expression}}\" attribute to the FAST directive"
        }
        ConvertError::InvalidRepeatExpression { .. } => {
            "use an f-repeat expression in \"item in items\" form"
        }
        ConvertError::UnclosedElement { .. } | ConvertError::UnclosedTag { .. } => {
            "close the reported FAST template element or opening tag"
        }
        ConvertError::UnsupportedFAttribute { .. }
        | ConvertError::UnsupportedFElement { .. } => {
            "remove the unsupported FAST construct or replace it with supported declarative syntax"
        }
        _ => "fix the reported FAST declarative template syntax",
    };

    let code = if matches!(
        error,
        ConvertError::UnclosedElement { .. } | ConvertError::UnclosedTag { .. }
    ) {
        codes::UNCLOSED_HTML_TAG
    } else {
        INVALID_FAST_TEMPLATE
    };

    Diagnostic::error(format!("invalid FAST template: {error}"))
        .code(code)
        .at_offset(source, offset)
        .snippet("<f-template>")
        .help(help)
        .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    fn transform(tag: &str, html: &str) -> Result<ComponentSourceResult> {
        transform_component_source(ComponentSource {
            tag_name: tag,
            html_content: html,
        })
    }

    #[test]
    fn plain_source_without_f_template_is_unchanged() {
        let html = r#"<template><if condition="visible"><span>{{title}}</span></if></template>"#;
        assert_eq!(
            transform("plain-card", html).expect("transform"),
            ComponentSourceResult::Unchanged
        );
    }

    #[test]
    fn f_template_name_overrides_tag_and_converts_for_webui() {
        let html = r#"<f-template name="named-card"><template><f-when value="{{visible && count > 0}}"><f-repeat value="{{item in items}}"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></f-repeat></f-when></template></f-template>"#;

        let ComponentSourceResult::Transformed(result) =
            transform("file-card", html).expect("transform")
        else {
            panic!("expected a transformed FAST source");
        };

        assert_eq!(result.tag_name, "named-card");
        // The converter output is the SSR parser view; FAST client attributes
        // are retained here and stripped later by `classify_attribute`.
        assert_eq!(
            result.parser_content,
            r#"<template><if condition="visible && count > 0"><for each="item in items"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></for></if></template>"#
        );
        // The client artifact retains the authored FAST syntax verbatim.
        assert_eq!(
            result.artifact_content.as_deref(),
            Some(
                r#"<template><f-when value="{{visible && count > 0}}"><f-repeat value="{{item in items}}"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></f-repeat></f-when></template>"#
            )
        );
    }

    #[test]
    fn absent_or_blank_name_falls_back_to_filename() {
        for html in [
            "<f-template><template></template></f-template>",
            r#"<f-template name=" "><template></template></f-template>"#,
        ] {
            let ComponentSourceResult::Transformed(result) =
                transform("file-card", html).expect("transform")
            else {
                panic!("expected a transformed FAST source");
            };
            assert_eq!(result.tag_name, "file-card");
            assert_eq!(result.parser_content, "<template></template>");
            assert_eq!(
                result.artifact_content.as_deref(),
                Some("<template></template>")
            );
        }
    }

    #[test]
    fn multiple_f_templates_are_rejected() {
        let err = transform(
            "multi-card",
            r#"<f-template name="one"><template></template></f-template><f-template name="two"><template></template></f-template>"#,
        )
        .expect_err("multiple f-template blocks should error");

        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(UNSUPPORTED_MULTIPLE_F_TEMPLATES));
        assert!(diag.to_string().contains("not currently supported"));
    }

    #[test]
    fn invalid_converter_input_is_a_diagnostic() {
        let err = transform(
            "invalid-card",
            r#"<f-template name="invalid-card"><template><f-repeat value="{{items}}"></f-repeat></template></f-template>"#,
        )
        .expect_err("invalid repeat should error");

        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert!(diag.to_string().contains("item in items"));
        assert!(diag.help_text().is_some());
    }

    #[test]
    fn precheck_matches_lower_upper_and_mixed_case_names() {
        for haystack in [
            "<f-template name=\"card\"><template></template></f-template>",
            "<F-TEMPLATE NAME=\"card\"><template></template></F-TEMPLATE>",
            "<F-template Name=\"card\"><template></template></f-Template>",
        ] {
            assert!(contains_f_template_name(haystack.as_bytes()));
            assert!(find_f_template_source(haystack).is_some());
        }
    }

    #[test]
    fn whitespace_after_opening_angle_bracket_passes_precheck_and_walker() {
        let html =
            r#"< f-template name="card"><template><span>{{title}}</span></template></f-template>"#;
        assert!(contains_f_template_name(html.as_bytes()));

        let ComponentSourceResult::Transformed(result) =
            transform("file-card", html).expect("transform")
        else {
            panic!("expected a transformed FAST source");
        };
        assert_eq!(result.tag_name, "card");
    }

    #[test]
    fn precheck_covers_every_ascii_whitespace_accepted_before_the_name() {
        for byte in (0u8..=127).filter(|byte| byte.is_ascii_whitespace()) {
            let whitespace = char::from(byte);
            let html =
                format!("<{whitespace}f-template name=\"card\"><template></template></f-template>");
            assert!(contains_f_template_name(html.as_bytes()));
            assert!(
                find_f_template_source(&html).is_some(),
                "walker rejected ASCII whitespace byte {:#04x}",
                byte
            );
        }
    }

    #[test]
    fn precheck_rejects_sources_without_the_bare_name() {
        for haystack in [
            "<f-templat name=\"card\"></f-templat>",
            "<f_template name=\"card\"></f_template>",
            "<ftemplate name=\"card\"></ftemplate>",
            "<template><span>{{title}}</span></template>",
            "<template><span>ƒ-template ☃</span></template>",
            "",
            "f-templat",
        ] {
            assert!(!contains_f_template_name(haystack.as_bytes()));
        }
    }

    #[test]
    fn precheck_false_positive_in_text_comment_or_attribute_is_harmless() {
        for html in [
            r#"<template><span>the f-template directive is unused here</span></template>"#,
            r#"<template><!-- f-template mentioned only in a comment --><span>{{title}}</span></template>"#,
            r#"<template><span data-note="f-template">{{title}}</span></template>"#,
            r#"<template><f-templatex></f-templatex></template>"#,
        ] {
            // The bare bytes are present, so the precheck reports a possible
            // match, but no source contains an actual `<f-template>` element,
            // so the authoritative walker still finds nothing.
            assert!(contains_f_template_name(html.as_bytes()));
            assert_eq!(
                transform("plain-card", html).expect("transform"),
                ComponentSourceResult::Unchanged
            );
        }
    }
}
