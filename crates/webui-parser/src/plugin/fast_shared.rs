// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared FAST `<f-template>` source handling for the FAST parser plugins.
//!
//! This module owns everything FAST-specific about turning an authored
//! `<f-template>` component source into the WebUI parser view: scanning for the
//! single `<f-template>` element, resolving its `name` (falling back to the
//! filename-derived tag), converting supported FAST directives, retaining the
//! authored `<f-template>` body for the client artifact, and constructing FAST
//! diagnostics. The FAST parser plugins expose it through the framework-neutral
//! [`ParserPlugin::component_source_transform`](super::ParserPlugin::component_source_transform)
//! hook; the parser core never references FAST.

use super::fast_convert::{convert_template, ConvertError, ConvertErrorKind, F_TEMPLATE_NAME};
use super::{ComponentSource, ComponentSourceResult, TransformedComponentSource};
use crate::diagnostic::{codes, Diagnostic};
use crate::{ParserError, Result};

/// Component-source transform for FAST authored templates.
///
/// Returns [`ComponentSourceResult::Unchanged`] when the source contains no
/// `<f-template>`; otherwise converts the FAST declarative syntax to the WebUI
/// parser view, resolves the registry key from the `<f-template name>`
/// attribute (or the filename), and retains the authored `<f-template>` body
/// as the client artifact.
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

    let Some(converted) =
        convert_template(html_content).map_err(|error| converter_error(html_content, &error))?
    else {
        return Ok(ComponentSourceResult::Unchanged);
    };

    let resolved_tag = converted.name.unwrap_or(source.tag_name).to_string();
    let artifact_content = html_content[converted.artifact].trim().to_string();

    Ok(ComponentSourceResult::Transformed(
        TransformedComponentSource {
            tag_name: resolved_tag,
            parser_content: converted.parser_content,
            artifact_content: Some(artifact_content),
        },
    ))
}

/// Cheap case-insensitive precheck for the bare ASCII name `f-template`
/// anywhere in `haystack`, without the surrounding `<` delimiter.
///
/// The converter takes an opening element name to be the run of bytes after `<`
/// (skipping ASCII whitespace) and matches it ASCII-case-insensitively, so every
/// spelling it accepts contains these ten bytes verbatim, case aside. Searching
/// for the bare name therefore has no false negatives, whereas searching for
/// `"<f-template"` would. Non-ASCII bytes are compared without case folding, so
/// UTF-8 input can neither panic nor be misread. False positives (the bytes
/// appearing in text, comments, attributes, or longer names) only cost the
/// authoritative conversion scan, which still reports `Unchanged`.
#[inline]
fn contains_f_template_name(haystack: &[u8]) -> bool {
    haystack
        .windows(F_TEMPLATE_NAME.len())
        .any(|window| window.eq_ignore_ascii_case(F_TEMPLATE_NAME.as_bytes()))
}

/// Build a [`Diagnostic`] error from a converter failure.
#[cold]
#[inline(never)]
fn converter_error(source: &str, error: &ConvertError<'_>) -> ParserError {
    if matches!(error.kind(), ConvertErrorKind::MultipleFTemplates { .. }) {
        return Diagnostic::error("multiple <f-template> elements are not supported")
            .code(codes::UNSUPPORTED_MULTIPLE_F_TEMPLATES)
            .at_offset(source, error.offset())
            .snippet("<f-template>")
            .help(
                "keep only one <f-template> per component file; multiple f-template blocks are not currently supported",
            )
            .into();
    }

    let help = match error.kind() {
        ConvertErrorKind::MissingInnerTemplate
        | ConvertErrorKind::MultipleInnerTemplates { .. } => {
            "keep exactly one inner <template> element inside <f-template>"
        }
        ConvertErrorKind::MissingValueAttribute { .. }
        | ConvertErrorKind::InvalidDirectiveValue { .. } => {
            "add the required value=\"{{expression}}\" attribute to the FAST directive"
        }
        ConvertErrorKind::InvalidRepeatExpression { .. } => {
            "use an f-repeat expression in \"item in items\" form"
        }
        ConvertErrorKind::UnclosedElement { .. } | ConvertErrorKind::UnclosedTag => {
            "close the reported FAST template element or opening tag"
        }
        ConvertErrorKind::UnsupportedFAttribute { .. }
        | ConvertErrorKind::UnsupportedFElement { .. } => {
            "remove the unsupported FAST construct or replace it with supported declarative syntax"
        }
        ConvertErrorKind::MultipleFTemplates { .. } => {
            "keep only one <f-template> per component file"
        }
    };

    let code = if matches!(
        error.kind(),
        ConvertErrorKind::UnclosedElement { .. } | ConvertErrorKind::UnclosedTag
    ) {
        codes::UNCLOSED_HTML_TAG
    } else {
        codes::INVALID_FAST_TEMPLATE
    };

    Diagnostic::error(format!("invalid FAST template: {error}"))
        .code(code)
        .at_offset(source, error.offset())
        .snippet(converter_error_snippet(error.kind()))
        .help(help)
        .into()
}

#[cold]
#[inline(never)]
fn converter_error_snippet(error: &ConvertErrorKind<'_>) -> String {
    match error {
        ConvertErrorKind::MultipleFTemplates { .. }
        | ConvertErrorKind::MissingInnerTemplate
        | ConvertErrorKind::MultipleInnerTemplates { .. } => "<f-template>".to_string(),
        ConvertErrorKind::UnclosedElement { tag }
        | ConvertErrorKind::MissingValueAttribute { tag }
        | ConvertErrorKind::InvalidDirectiveValue { tag, .. }
        | ConvertErrorKind::UnsupportedFElement { tag } => {
            let mut snippet = String::with_capacity(tag.len() + 2);
            snippet.push('<');
            snippet.push_str(tag);
            snippet.push('>');
            snippet
        }
        ConvertErrorKind::InvalidRepeatExpression { expr } => {
            let mut snippet = String::with_capacity(expr.len() + 4);
            snippet.push_str("{{");
            snippet.push_str(expr);
            snippet.push_str("}}");
            snippet
        }
        ConvertErrorKind::UnsupportedFAttribute { attribute } => attribute.to_string(),
        ConvertErrorKind::UnclosedTag => "<".to_string(),
    }
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
    fn parser_extracts_inner_template_while_artifact_keeps_wrapper_body() {
        let html = r#"<f-template name="named-card">before<template><span>{{label}}</span></template>after</f-template>"#;
        let ComponentSourceResult::Transformed(result) =
            transform("file-card", html).expect("transform")
        else {
            panic!("expected a transformed FAST source");
        };

        assert_eq!(
            result.parser_content,
            "<template><span>{{label}}</span></template>"
        );
        assert_eq!(
            result.artifact_content.as_deref(),
            Some("before<template><span>{{label}}</span></template>after")
        );
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
        assert_eq!(
            diag.error_code(),
            Some(codes::UNSUPPORTED_MULTIPLE_F_TEMPLATES)
        );
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
        assert_eq!(diag.error_code(), Some(codes::INVALID_FAST_TEMPLATE));
        assert!(diag.to_string().contains("item in items"));
        assert!(diag.help_text().is_some());
    }

    #[test]
    fn converter_diagnostics_point_to_the_offending_construct() {
        let repeat = transform(
            "invalid-card",
            "<f-template name=\"invalid-card\">\n  <template>\n    <f-repeat value=\"{{items}}\"></f-repeat>\n  </template>\n</f-template>",
        )
        .expect_err("invalid repeat should error");
        let ParserError::Template(repeat_diag) = repeat else {
            panic!("expected template diagnostic");
        };
        assert_eq!(repeat_diag.error_code(), Some(codes::INVALID_FAST_TEMPLATE));
        assert_eq!(repeat_diag.position_line_column(), Some((3, 5)));
        assert_eq!(repeat_diag.snippet_text(), Some("{{items}}"));

        let attribute = transform(
            "invalid-card",
            "<f-template name=\"invalid-card\">\n  <template>\n    <span f-unknown=\"value\"></span>\n  </template>\n</f-template>",
        )
        .expect_err("unsupported FAST attribute should error");
        let ParserError::Template(attribute_diag) = attribute else {
            panic!("expected template diagnostic");
        };
        assert_eq!(
            attribute_diag.error_code(),
            Some(codes::INVALID_FAST_TEMPLATE)
        );
        assert_eq!(attribute_diag.position_line_column(), Some((3, 11)));
        assert_eq!(attribute_diag.snippet_text(), Some("f-unknown"));
    }

    #[test]
    fn missing_or_multiple_inner_templates_are_diagnostics() {
        for html in [
            r#"<f-template name="invalid-card"><span></span></f-template>"#,
            r#"<f-template name="invalid-card"><template></template><template></template></f-template>"#,
        ] {
            let err = transform("invalid-card", html)
                .expect_err("invalid inner template count should error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(codes::INVALID_FAST_TEMPLATE));
            assert_eq!(
                diag.help_text(),
                Some("keep exactly one inner <template> element inside <f-template>")
            );
        }
    }

    #[test]
    fn invalid_directives_and_unsupported_fast_constructs_are_diagnostics() {
        for html in [
            r#"<f-template name="invalid-card"><template><f-when>shown</f-when></template></f-template>"#,
            r#"<f-template name="invalid-card"><template><f-when value="visible">shown</f-when></template></f-template>"#,
            r#"<f-template name="invalid-card"><template><span f-unknown="value"></span></template></f-template>"#,
            r#"<f-template name="invalid-card"><template><f-choose></f-choose></template></f-template>"#,
        ] {
            let err =
                transform("invalid-card", html).expect_err("invalid FAST construct should error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(codes::INVALID_FAST_TEMPLATE));
            assert!(diag.help_text().is_some());
        }
    }

    #[test]
    fn unclosed_fast_elements_and_tags_use_unclosed_html_diagnostic() {
        for html in [
            r#"<f-template name="invalid-card"><template><f-when value="{{visible}}"><span>shown</span></template></f-template>"#,
            r#"<f-template name="invalid-card"><template><span title="unterminated></template></f-template>"#,
            r#"<f-template name="invalid-card" /><template></template></f-template>"#,
            r#"<f-template name="invalid-card"><template /></template></f-template>"#,
        ] {
            let err =
                transform("invalid-card", html).expect_err("unclosed FAST source should error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(codes::UNCLOSED_HTML_TAG));
            assert_eq!(
                diag.help_text(),
                Some("close the reported FAST template element or opening tag")
            );
        }
    }

    #[test]
    fn supported_fast_attributes_and_comments_are_preserved_for_parser_plugin() {
        let html = r#"<f-template name="binding-card"><template><!-- <f-when value="{{ignored}}"> --><div f-ref="{root}" f-children="{children}" f-slotted="{slotted}">{{label}}</div></template></f-template>"#;
        let ComponentSourceResult::Transformed(result) =
            transform("file-card", html).expect("transform")
        else {
            panic!("expected a transformed FAST source");
        };

        assert_eq!(
            result.parser_content,
            r#"<template><!-- <f-when value="{{ignored}}"> --><div f-ref="{root}" f-children="{children}" f-slotted="{slotted}">{{label}}</div></template>"#
        );
    }

    #[test]
    fn repeat_alias_and_dotted_source_grammar_is_preserved() {
        let html = r#"<f-template name="repeat-card"><template><f-repeat value="{{ $item in groups.items_2 }}"><span>{{$item.label}}</span></f-repeat></template></f-template>"#;
        let ComponentSourceResult::Transformed(result) =
            transform("file-card", html).expect("transform")
        else {
            panic!("expected a transformed FAST source");
        };

        assert_eq!(
            result.parser_content,
            r#"<template><for each="$item in groups.items_2"><span>{{$item.label}}</span></for></template>"#
        );
    }

    #[test]
    fn deeply_nested_fast_directives_convert_iteratively() {
        const DEPTH: usize = 2_048;
        let mut html = String::with_capacity(DEPTH * 48 + 128);
        html.push_str(r#"<f-template name="deep-card"><template>"#);
        for _ in 0..DEPTH {
            html.push_str(r#"<f-when value="{{visible}}">"#);
        }
        html.push_str("<span>{{title}}</span>");
        for _ in 0..DEPTH {
            html.push_str("</f-when>");
        }
        html.push_str("</template></f-template>");

        let converted = convert_template(&html)
            .expect("convert deeply nested FAST directives")
            .expect("f-template should be found");
        assert_eq!(
            converted.parser_content.matches("<if condition=").count(),
            DEPTH
        );
        assert_eq!(converted.parser_content.matches("</if>").count(), DEPTH);
    }

    #[test]
    fn precheck_matches_lower_upper_and_mixed_case_names() {
        for haystack in [
            "<f-template name=\"card\"><template></template></f-template>",
            "<F-TEMPLATE NAME=\"card\"><template></template></F-TEMPLATE>",
            "<F-template Name=\"card\"><template></template></f-Template>",
        ] {
            assert!(contains_f_template_name(haystack.as_bytes()));
            assert!(convert_template(haystack)
                .expect("convert mixed-case f-template")
                .is_some());
        }
    }

    #[test]
    fn whitespace_after_opening_angle_bracket_passes_precheck_and_converter() {
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
                convert_template(&html)
                    .expect("convert whitespace-prefixed f-template")
                    .is_some(),
                "converter rejected ASCII whitespace byte {:#04x}",
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
            r#"<template><span data-note="<f-template name='not-an-element'>">{{title}}</span></template>"#,
            r#"<template><f-templatex></f-templatex></template>"#,
        ] {
            // The bare bytes are present, so the precheck reports a possible
            // match, but no source contains an actual `<f-template>` element,
            // so the authoritative conversion scan still finds nothing.
            assert!(contains_f_template_name(html.as_bytes()));
            assert_eq!(
                transform("plain-card", html).expect("transform"),
                ComponentSourceResult::Unchanged
            );
        }
    }
}
