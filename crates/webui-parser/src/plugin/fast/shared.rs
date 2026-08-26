// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared FAST source conversion and artifact handling.

use super::super::{AttributeAction, ComponentSource, TransformedComponentSource};
use super::convert::{convert_template, F_TEMPLATE_NAME};
use super::diagnostic::converter_error;
use crate::html_parser::{leading_content, parse_tag};
use crate::Result;
use webui_protocol::FastElementData;

// Classify client-only FAST attributes that SSR skips but hydration counts.
#[inline]
pub(crate) fn classify_attribute(attr_name: &str) -> AttributeAction {
    if attr_name.starts_with('@')
        || attr_name.starts_with(':')
        || attr_name.eq_ignore_ascii_case("f-ref")
        || attr_name.eq_ignore_ascii_case("f-slotted")
        || attr_name.eq_ignore_ascii_case("f-children")
    {
        AttributeAction::SkipAndCountBinding
    } else {
        AttributeAction::Keep
    }
}

// Encode binding metadata for dynamic attributes.
#[inline]
pub(crate) fn finish_element(binding_attribute_count: u32) -> Option<Vec<u8>> {
    if binding_attribute_count > 0 {
        Some(
            FastElementData {
                binding_count: binding_attribute_count,
            }
            .encode()
            .to_vec(),
        )
    } else {
        None
    }
}

// FAST reads these options from `<f-template>`; adopted styles remain inner.
#[inline]
pub(crate) fn is_hoisted_shadow_attr(name: &str) -> bool {
    const PREFIX: &[u8] = b"shadowroot";
    name.len() >= PREFIX.len()
        && name.as_bytes()[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
        && !name.eq_ignore_ascii_case("shadowrootadoptedstylesheets")
}

// Collect shadow-root options to hoist onto `<f-template>`.
pub(crate) fn hoisted_shadow_options(processed_template: &str) -> String {
    let (trimmed, _) = leading_content(processed_template);
    let Some(tag) = parse_tag(trimmed) else {
        return String::new();
    };
    if tag.closing || !tag.name.eq_ignore_ascii_case("template") {
        return String::new();
    }
    let mut options = String::new();
    for attr in tag.attrs() {
        if is_hoisted_shadow_attr(attr.name) {
            options.push(' ');
            options.push_str(attr.raw);
        }
    }
    options
}

// Strip hoisted options and return the consumed bytes when rebuilt.
pub(crate) fn strip_hoisted_shadow_options(tag_str: &str, result: &mut String) -> Option<usize> {
    let tag = parse_tag(tag_str)?;
    if tag.closing || !tag.name.eq_ignore_ascii_case("template") {
        return None;
    }

    // Avoid rebuilding templates without hoisted options.
    if !tag.attrs().any(|attr| is_hoisted_shadow_attr(attr.name)) {
        return None;
    }

    let bytes = tag_str.as_bytes();
    let mut cursor = 0usize;
    for attr in tag.attrs() {
        if !is_hoisted_shadow_attr(attr.name) {
            continue;
        }
        let mut removal_start = attr.raw_range.start;
        while removal_start > cursor && bytes[removal_start - 1].is_ascii_whitespace() {
            removal_start -= 1;
        }
        let segment = &tag_str[cursor..removal_start];
        if segment
            .as_bytes()
            .first()
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/')
            && result
                .as_bytes()
                .last()
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'<')
        {
            result.push(' ');
        }
        result.push_str(segment);
        cursor = attr.raw_range.end;
    }
    let suffix = &tag_str[cursor..=tag.close];
    if suffix
        .as_bytes()
        .first()
        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/')
        && result
            .as_bytes()
            .last()
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'<')
    {
        result.push(' ');
    }
    result.push_str(suffix);
    Some(tag.close + 1)
}

// Inject CSS after bindings when required by FAST's brace scanner.
pub(crate) fn push_body_with_css_injection(
    output: &mut String,
    body: &str,
    injection: Option<&str>,
    at_end: bool,
) {
    let Some(injection) = injection else {
        output.push_str(body);
        return;
    };
    if at_end {
        if let Some(close_start) = body.rfind("</template>") {
            output.push_str(&body[..close_start]);
            output.push_str(injection);
            output.push_str(&body[close_start..]);
            return;
        }
    }
    output.push_str(injection);
    output.push_str(body);
}

// Convert an authored FAST source into parser and client-artifact views.
pub(crate) fn transform_component_source(
    source: ComponentSource<'_>,
) -> Result<Option<TransformedComponentSource>> {
    let html_content = source.html_content;
    if !contains_f_template_name(html_content.as_bytes()) {
        return Ok(None);
    }

    let Some(converted) = convert_template(html_content)
        .map_err(|error| converter_error(html_content, source.tag_name, &error))?
    else {
        return Ok(None);
    };

    let resolved_tag = converted.name.unwrap_or(source.tag_name).to_string();

    Ok(Some(TransformedComponentSource {
        tag_name: resolved_tag,
        parser_content: converted.parser_content,
        artifact_content: Some(converted.artifact_content),
    }))
}

// Case-insensitive byte precheck; false positives use the authoritative scan.
#[inline]
fn contains_f_template_name(haystack: &[u8]) -> bool {
    haystack
        .windows(F_TEMPLATE_NAME.len())
        .any(|window| window.eq_ignore_ascii_case(F_TEMPLATE_NAME.as_bytes()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use crate::diagnostic::codes;
    use crate::plugin::fast::diagnostic::{
        INVALID_FAST_TEMPLATE, UNSUPPORTED_MULTIPLE_F_TEMPLATES,
    };
    use crate::ParserError;

    // --- is_hoisted_shadow_attr / hoisted_shadow_options / strip_hoisted_shadow_options ---
    //
    // These three are the hoist/strip pair the FAST v2/v3 client-artifact
    // generators rely on to move declarative-shadow-root options from the
    // inner `<template>` onto the `<f-template>` wrapper without ever
    // duplicating one. Covering them here (once, for the shared predicate and
    // both shared functions) is what keeps v2.rs and v3.rs from
    // silently reintroducing separate, divergent implementations.

    #[test]
    fn is_hoisted_shadow_attr_matches_every_shadowroot_option_except_adoptedstylesheets() {
        assert!(is_hoisted_shadow_attr("shadowrootmode"));
        assert!(is_hoisted_shadow_attr("shadowrootdelegatesfocus"));
        assert!(is_hoisted_shadow_attr("shadowrootclonable"));
        assert!(is_hoisted_shadow_attr("shadowrootserializable"));
        // Case-insensitive, matching HTML attribute-name semantics.
        assert!(is_hoisted_shadow_attr("ShadowRootMode"));
        assert!(!is_hoisted_shadow_attr("shadowrootadoptedstylesheets"));
        assert!(!is_hoisted_shadow_attr("SHADOWROOTADOPTEDSTYLESHEETS"));
        assert!(!is_hoisted_shadow_attr("shadow"));
        assert!(!is_hoisted_shadow_attr("@click"));
    }

    fn strip(tag_str: &str) -> (Option<usize>, String) {
        let mut result = String::new();
        let consumed = strip_hoisted_shadow_options(tag_str, &mut result);
        (consumed, result)
    }

    #[test]
    fn strip_no_shadow_attrs_is_a_no_op_fast_path() {
        // No `shadowroot*` attribute at all: bail without writing to `result`.
        let (consumed, result) = strip(r#"<template @click="{onClick(e)}"><div></div>"#);
        assert_eq!(consumed, None);
        assert!(result.is_empty());
    }

    #[test]
    fn strip_non_template_tag_returns_none() {
        let (consumed, result) = strip(r#"<div shadowrootmode="open"></div>"#);
        assert_eq!(consumed, None);
        assert!(result.is_empty());
    }

    #[test]
    fn strip_mode_only_existing_path() {
        let (consumed, result) = strip(r#"<template shadowrootmode="open">"#);
        assert!(consumed.is_some());
        assert_eq!(result, r#"<template>"#);
    }

    #[test]
    fn strip_delegatesfocus_only_without_mode() {
        // Regression: a `shadowroot*` option other than `shadowrootmode` must
        // still be stripped, since `hoisted_shadow_options` hoists it too.
        let (consumed, result) = strip(r#"<template shadowrootdelegatesfocus>"#);
        assert!(consumed.is_some());
        assert_eq!(result, r#"<template>"#);
    }

    #[test]
    fn strip_clonable_and_serializable_only() {
        let (consumed, result) = strip(r#"<template shadowrootclonable shadowrootserializable>"#);
        assert!(consumed.is_some());
        assert_eq!(result, r#"<template>"#);
    }

    #[test]
    fn strip_adoptedstylesheets_only_is_kept_not_hoisted() {
        // `shadowrootadoptedstylesheets` is excluded by `is_hoisted_shadow_attr`,
        // so this tag carries no hoisted option at all and the fast guard bails
        // without allocating or rebuilding the tag; callers keep the tag
        // unchanged via the ordinary (non-shadow) code path.
        let (consumed, result) = strip(r#"<template shadowrootadoptedstylesheets="my-comp">"#);
        assert_eq!(consumed, None);
        assert!(result.is_empty());
    }

    #[test]
    fn strip_mixed_attrs_removes_only_hoisted_ones_exactly_once() {
        let input = concat!(
            r#"<template shadowrootmode="open" shadowrootdelegatesfocus "#,
            r#"shadowrootclonable shadowrootserializable "#,
            r#"shadowrootadoptedstylesheets="my-comp" @click="{onClick(e)}">"#,
        );
        let (consumed, result) = strip(input);
        assert!(consumed.is_some());
        assert_eq!(
            result,
            r#"<template shadowrootadoptedstylesheets="my-comp" @click="{onClick(e)}">"#
        );
        for hoisted in [
            "shadowrootmode",
            "shadowrootdelegatesfocus",
            "shadowrootclonable",
            "shadowrootserializable",
        ] {
            assert_eq!(
                result.matches(hoisted).count(),
                0,
                "{hoisted} must not remain on the inner template: {result}"
            );
        }
    }

    fn transform(tag: &str, html: &str) -> Result<Option<TransformedComponentSource>> {
        transform_component_source(ComponentSource {
            tag_name: tag,
            html_content: html,
        })
    }

    #[test]
    fn plain_source_without_f_template_is_unchanged() {
        let html = r#"<template><if condition="visible"><span>{{title}}</span></if></template>"#;
        assert_eq!(transform("plain-card", html).expect("transform"), None);
    }

    #[test]
    fn f_template_name_overrides_tag_and_converts_for_webui() {
        let html = r#"<f-template name="named-card"><template><f-when value="{{visible && count > 0}}"><f-repeat value="{{item in items}}"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></f-repeat></f-when></template></f-template>"#;

        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");

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
    fn html_names_are_ascii_case_insensitive() {
        let result = transformed(
            "file-card",
            r#"<F-TEMPLATE NAME="named-card"><TEMPLATE><F-WHEN VALUE="{{visible}}"><button F-REF="{button}">save</button></F-WHEN></TEMPLATE></F-TEMPLATE>"#,
        );

        assert_eq!(result.tag_name, "named-card");
        assert!(result
            .parser_content
            .contains(r#"<if condition="visible">"#));
        assert!(!result.parser_content.contains("<F-WHEN"));
        assert_eq!(
            classify_attribute("F-REF"),
            AttributeAction::SkipAndCountBinding
        );

        let error = transform(
            "invalid-card",
            r#"<f-template name="invalid-card"><template><F-BOGUS></F-BOGUS></template></f-template>"#,
        )
        .expect_err("unsupported uppercase FAST element should error");
        let ParserError::Template(diagnostic) = error else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diagnostic.error_code(), Some(INVALID_FAST_TEMPLATE));
    }

    #[test]
    fn absent_or_blank_name_falls_back_to_filename() {
        for html in [
            "<f-template><template></template></f-template>",
            r#"<f-template name=" "><template></template></f-template>"#,
        ] {
            let result = transform("file-card", html)
                .expect("transform")
                .expect("transformed FAST source");
            assert_eq!(result.tag_name, "file-card");
            assert_eq!(result.parser_content, "<template></template>");
            assert_eq!(
                result.artifact_content.as_deref(),
                Some("<template></template>")
            );
        }
    }

    #[test]
    fn artifact_is_the_authored_inner_template_not_the_wrapper_body() {
        // The artifact retains exactly the inner <template> (with its
        // client-only bindings), so it always begins with `<template` and can
        // never be re-wrapped in a synthetic <template>. Comments may surround
        // the inner template but are inert and excluded from the artifact.
        let html = r#"<f-template name="named-card"><!-- lead --><template><button @click="{save()}">{{label}}</button></template><!-- tail --></f-template>"#;
        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");

        assert_eq!(
            result.parser_content,
            r#"<template><button @click="{save()}">{{label}}</button></template>"#
        );
        let artifact = result.artifact_content.as_deref().expect("artifact");
        assert!(
            artifact.starts_with("<template"),
            "artifact must begin with the inner <template>: {artifact:?}"
        );
        assert_eq!(
            artifact,
            r#"<template><button @click="{save()}">{{label}}</button></template>"#
        );
    }

    #[test]
    fn meaningful_siblings_inside_f_template_are_rejected() {
        // Content inside <f-template> but around the inner <template> would be
        // silently dropped from the SSR view; reject it instead of preserving
        // it only in the client artifact. Covers a leading sibling, a trailing
        // sibling, and a sibling element.
        for html in [
            r#"<f-template name="named-card">before<template><span>{{label}}</span></template></f-template>"#,
            r#"<f-template name="named-card"><template><span>{{label}}</span></template>after</f-template>"#,
            r#"<f-template name="named-card"><aside>x</aside><template><span>{{label}}</span></template></f-template>"#,
        ] {
            let err = transform("file-card", html)
                .expect_err("meaningful siblings around the inner template should error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
            assert_eq!(diag.component_name(), Some("file-card"));
            assert!(diag
                .help_text()
                .is_some_and(|help| help.contains("single inner <template>")));
        }
    }

    #[test]
    fn content_outside_f_template_is_a_diagnostic() {
        let err = transform(
            "invalid-card",
            r#"<div>before</div><f-template name="invalid-card"><template><span>{{label}}</span></template></f-template>"#,
        )
        .expect_err("outside top-level content should error");

        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert_eq!(
            diag.help_text(),
            Some(
                "keep <f-template> as the only top-level authored content (outside content may only be whitespace or comments)"
            )
        );
    }

    #[test]
    fn whitespace_and_comments_outside_f_template_are_allowed() {
        let html = " \n<!-- lead -->\n<f-template name=\"named-card\"><template><span>{{label}}</span></template></f-template>\n<!-- tail -->\n ";
        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");
        assert_eq!(result.tag_name, "named-card");
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
    fn converter_diagnostics_point_to_the_offending_construct() {
        let repeat = transform(
            "invalid-card",
            "<f-template name=\"invalid-card\">\n  <template>\n    <f-repeat value=\"{{items}}\"></f-repeat>\n  </template>\n</f-template>",
        )
        .expect_err("invalid repeat should error");
        let ParserError::Template(repeat_diag) = repeat else {
            panic!("expected template diagnostic");
        };
        assert_eq!(repeat_diag.error_code(), Some(INVALID_FAST_TEMPLATE));
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
        assert_eq!(attribute_diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert_eq!(attribute_diag.position_line_column(), Some((3, 11)));
        assert_eq!(attribute_diag.snippet_text(), Some("f-unknown"));
    }

    #[test]
    fn missing_inner_template_is_a_diagnostic() {
        let err = transform(
            "invalid-card",
            r#"<f-template name="invalid-card"><span></span></f-template>"#,
        )
        .expect_err("missing inner template should error");
        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert_eq!(
            diag.help_text(),
            Some("keep exactly one inner <template> element inside <f-template>")
        );
    }

    #[test]
    fn sibling_inner_templates_are_a_diagnostic() {
        let err = transform(
            "invalid-card",
            r#"<f-template name="invalid-card"><template></template><template></template></f-template>"#,
        )
        .expect_err("sibling inner templates should error");
        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert!(diag
            .help_text()
            .is_some_and(|help| help.contains("single inner <template>")));
    }

    #[test]
    fn nested_inert_template_is_preserved() {
        let result = transformed(
            "file-card",
            r#"<f-template name="named-card"><template><div><template id="row"><span>{{label}}</span></template></div></template></f-template>"#,
        );

        assert!(result
            .parser_content
            .contains(r#"<template id="row"><span>{{label}}</span></template>"#));
        assert!(result
            .artifact_content
            .as_deref()
            .is_some_and(|artifact| artifact
                .contains(r#"<template id="row"><span>{{label}}</span></template>"#)));
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
            assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
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
        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");

        assert_eq!(
            result.parser_content,
            r#"<template><!-- <f-when value="{{ignored}}"> --><div f-ref="{root}" f-children="{children}" f-slotted="{slotted}">{{label}}</div></template>"#
        );
    }

    #[test]
    fn generator_styles_marker_is_removed_from_parser_and_artifact() {
        // The `{{styles}}` marker that `generate-templates` injects immediately
        // after the inner <template> opening is a build placeholder, not
        // component state: WebUI removes it (its CSS strategy injects styles at
        // the same position) so it is not rendered as a `styles` text signal or
        // counted as a hydration binding on either the server or client.
        let html = concat!(
            r#"<f-template name="custom-button" shadowrootmode="open">"#,
            "<template @click=\"{clickHandler($e)}\">\n    {{styles}}\n    ",
            r#"<slot name="start" f-ref="{start}"></slot></template></f-template>"#,
        );
        let result = transform("button.template", html)
            .expect("transform")
            .expect("transformed FAST source");
        assert_eq!(result.tag_name, "custom-button");
        assert!(
            !result.parser_content.contains("{{styles}}"),
            "parser view must not retain the generator marker: {}",
            result.parser_content
        );
        assert!(
            !result
                .artifact_content
                .as_deref()
                .expect("artifact")
                .contains("{{styles}}"),
            "client artifact must not retain the generator marker",
        );
    }

    #[test]
    fn legitimate_styles_binding_outside_generator_position_is_preserved() {
        // A `{{styles}}` interpolation that is not the leading generator marker
        // (here inside a child element) is a real binding and must survive.
        let result = transformed(
            "file-card",
            r#"<f-template name="styled-card"><template><span>{{styles}}</span></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><span>{{styles}}</span></template>"#
        );
    }

    #[test]
    fn wrapper_shadow_options_are_baked_onto_inner_template() {
        // shadowrootmode and shadowrootdelegatesfocus live on the outer
        // <f-template> wrapper; the converter moves them onto the inner
        // <template> (the WebUI declarative shadow root) in both the SSR view
        // and the client artifact, matching `fTemplateToWebui`.
        let result = transformed(
            "field.template",
            concat!(
                r#"<f-template name="custom-field" shadowrootmode="open" shadowrootdelegatesfocus>"#,
                r#"<template @click="{clickHandler($e)}"><slot></slot></template></f-template>"#,
            ),
        );
        assert_eq!(result.tag_name, "custom-field");
        assert_eq!(
            result.parser_content,
            r#"<template shadowrootmode="open" shadowrootdelegatesfocus @click="{clickHandler($e)}"><slot></slot></template>"#
        );
        assert_eq!(
            result.artifact_content.as_deref(),
            Some(
                r#"<template shadowrootmode="open" shadowrootdelegatesfocus @click="{clickHandler($e)}"><slot></slot></template>"#
            )
        );
    }

    #[test]
    fn wrapper_without_shadow_options_leaves_inner_template_unchanged() {
        let result = transformed(
            "file-card",
            r#"<f-template name="plain-card"><template><span>{{label}}</span></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><span>{{label}}</span></template>"#
        );
    }

    #[test]
    fn unknown_wrapper_attribute_is_rejected() {
        // A non-name, non-shadowroot* attribute on the wrapper is unsupported;
        // rejecting it avoids silently dropping it from SSR while it survives in
        // the client artifact.
        let err = transform(
            "file-card",
            r#"<f-template name="custom-button" class="danger"><template><slot></slot></template></f-template>"#,
        )
        .expect_err("unknown wrapper attribute should error");
        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert_eq!(diag.snippet_text(), Some("class"));
        assert!(diag
            .to_string()
            .contains("does not support the 'class' attribute"));
    }

    #[test]
    fn repeat_alias_and_dotted_source_grammar_is_preserved() {
        let html = r#"<f-template name="repeat-card"><template><f-repeat value="{{ $item in groups.items_2 }}"><span>{{$item.label}}</span></f-repeat></template></f-template>"#;
        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");

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

        let result = transform("file-card", html)
            .expect("transform")
            .expect("transformed FAST source");
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
            assert_eq!(transform("plain-card", html).expect("transform"), None);
        }
    }

    fn transformed(tag: &str, html: &str) -> TransformedComponentSource {
        transform(tag, html)
            .expect("transform")
            .expect("transformed FAST source")
    }

    #[test]
    fn when_condition_with_double_quoted_literal_uses_single_quote_delimiter() {
        let result = transformed(
            "file-card",
            r#"<f-template name="status-card"><template><f-when value='{{status == "ready"}}'><span>{{label}}</span></f-when></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><if condition='status == "ready"'><span>{{label}}</span></if></template>"#
        );
        // The authored FAST syntax is retained verbatim for the client artifact.
        assert_eq!(
            result.artifact_content.as_deref(),
            Some(
                r#"<template><f-when value='{{status == "ready"}}'><span>{{label}}</span></f-when></template>"#
            )
        );
    }

    #[test]
    fn when_condition_with_single_quoted_literal_keeps_double_quote_delimiter() {
        let result = transformed(
            "file-card",
            r#"<f-template name="status-card"><template><f-when value="{{status == 'ready'}}"><span>{{label}}</span></f-when></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><if condition="status == 'ready'"><span>{{label}}</span></if></template>"#
        );
    }

    #[test]
    fn when_condition_without_quotes_is_byte_identical_double_quoted() {
        let result = transformed(
            "file-card",
            r#"<f-template name="status-card"><template><f-when value="{{visible && count > 0}}"><span>{{label}}</span></f-when></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><if condition="visible && count > 0"><span>{{label}}</span></if></template>"#
        );
    }

    #[test]
    fn when_condition_mixing_quote_styles_is_a_diagnostic() {
        let err = transform(
            "file-card",
            r#"<f-template name="status-card"><template><f-when value={{a=="x"&&b=='y'}}><span>{{label}}</span></f-when></template></f-template>"#,
        )
        .expect_err("mixed quote condition should error");
        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert!(diag.to_string().contains("mixes single and double quotes"));
        assert!(diag.help_text().is_some());
    }

    #[test]
    fn uppercase_f_template_resolves_name_attribute_case_insensitively() {
        let result = transformed(
            "file-card",
            r#"<F-TEMPLATE NAME="named-card"><template><span>{{label}}</span></template></F-TEMPLATE>"#,
        );
        assert_eq!(result.tag_name, "named-card");
        assert_eq!(
            result.parser_content,
            "<template><span>{{label}}</span></template>"
        );
    }

    #[test]
    fn fake_fast_wrappers_inside_top_level_raw_text_are_ignored() {
        // A <script>/<style> whose body mentions FAST markup is not a wrapper;
        // with no real <f-template> the source is unchanged. Complete and
        // incomplete fake tags are both covered.
        for html in [
            r#"<script>const t = "<f-template name='fake'><template></template></f-template>";</script>"#,
            r#"<script>const t = "<f-template name=";</script>"#,
            r#"<style>/* <f-template><template></template></f-template> */ .a{color:red}</style>"#,
        ] {
            assert!(contains_f_template_name(html.as_bytes()));
            assert_eq!(transform("plain-card", html).expect("transform"), None);
        }
    }

    #[test]
    fn fake_fast_wrapper_in_script_does_not_trigger_multiple_wrapper_error() {
        // The top-level wrapper scan must skip the <script> body so the fake
        // <f-template> inside is not counted alongside the real wrapper.
        let result = transformed(
            "file-card",
            concat!(
                r#"<f-template name="real-card"><template>"#,
                r#"<script>var x = "<f-template name='fake'></f-template>";</script>"#,
                r#"<span>{{label}}</span></template></f-template>"#,
            ),
        );
        assert_eq!(result.tag_name, "real-card");
    }

    #[test]
    fn fake_fast_directives_inside_raw_text_are_copied_verbatim() {
        // <script>/<style> bodies inside the wrapper are opaque: markup-shaped
        // text is copied verbatim and never converted to WebUI directives.
        let result = transformed(
            "file-card",
            concat!(
                r#"<f-template name="script-card"><template>"#,
                r#"<script>if (a) { render("<f-when value='{{x}}'>"); }</script>"#,
                r#"<style>/* <f-repeat value="{{i in items}}"> */ .a{color:red}</style>"#,
                r#"<span>{{label}}</span>"#,
                r#"</template></f-template>"#,
            ),
        );
        assert_eq!(
            result.parser_content,
            concat!(
                r#"<template>"#,
                r#"<script>if (a) { render("<f-when value='{{x}}'>"); }</script>"#,
                r#"<style>/* <f-repeat value="{{i in items}}"> */ .a{color:red}</style>"#,
                r#"<span>{{label}}</span>"#,
                r#"</template>"#,
            )
        );
        assert!(!result.parser_content.contains("<if condition"));
        assert!(!result.parser_content.contains("<for each"));
    }

    #[test]
    fn incomplete_fake_directive_inside_raw_text_is_copied_verbatim() {
        // An unterminated FAST-looking tag inside a raw-text body must not
        // become an unclosed-tag diagnostic; the whole element is opaque.
        let result = transformed(
            "file-card",
            concat!(
                r#"<f-template name="script-card"><template>"#,
                r#"<script>const s = "<f-when value=";</script>"#,
                r#"<span>{{label}}</span>"#,
                r#"</template></f-template>"#,
            ),
        );
        assert!(result
            .parser_content
            .contains(r#"<script>const s = "<f-when value=";</script>"#));
        assert!(!result.parser_content.contains("<if condition"));
    }

    #[test]
    fn literal_opening_template_tokens_inside_raw_text_do_not_perturb_boundaries() {
        // A raw-text body containing an unbalanced literal opening
        // `<template>`/`<f-template>`-shaped string (no matching literal
        // close) must not make the boundary matcher demand an extra real
        // closing tag that does not exist. Both the outer `<f-template>` and
        // inner `<template>` boundary searches are exercised because both
        // tag names appear here.
        let html = concat!(
            r#"<f-template name="script-card"><template>"#,
            r#"<script>const s = "<template>";</script>"#,
            r#"<style>/* <f-template> */ .a{color:red}</style>"#,
            r#"<span>{{label}}</span>"#,
            r#"</template></f-template>"#,
        );
        let result = transformed("file-card", html);
        assert_eq!(result.tag_name, "script-card");
        let expected = concat!(
            r#"<template>"#,
            r#"<script>const s = "<template>";</script>"#,
            r#"<style>/* <f-template> */ .a{color:red}</style>"#,
            r#"<span>{{label}}</span>"#,
            r#"</template>"#,
        );
        assert_eq!(result.parser_content, expected);
        assert_eq!(result.artifact_content.as_deref(), Some(expected));
    }

    #[test]
    fn literal_closing_template_tokens_inside_raw_text_do_not_truncate() {
        // A raw-text body containing an unbalanced literal closing
        // `</template>`/`</f-template>`-shaped string (no matching literal
        // open) must not be mistaken for the real closing tag and truncate
        // the match early.
        let html = concat!(
            r#"<f-template name="script-card"><template>"#,
            r#"<script>const s = "</template>";</script>"#,
            r#"<style>/* </f-template> */ .a{color:red}</style>"#,
            r#"<span>{{label}}</span>"#,
            r#"</template></f-template>"#,
        );
        let result = transformed("file-card", html);
        assert_eq!(result.tag_name, "script-card");
        let expected = concat!(
            r#"<template>"#,
            r#"<script>const s = "</template>";</script>"#,
            r#"<style>/* </f-template> */ .a{color:red}</style>"#,
            r#"<span>{{label}}</span>"#,
            r#"</template>"#,
        );
        assert_eq!(result.parser_content, expected);
        assert_eq!(result.artifact_content.as_deref(), Some(expected));
    }

    #[test]
    fn unterminated_raw_text_body_with_literal_closing_tags_is_unclosed_element() {
        // A genuinely unterminated <script>/<style> body (no real closing
        // tag anywhere in the source) consumes the remainder of the document
        // as opaque raw text, per `find_raw_text_end`. A literal
        // `</f-template>`/`</template>`-shaped string inside it must not be
        // mistaken for the true wrapper close: the correct diagnostic is an
        // unclosed-element error, not silent truncation or a misleading
        // "content outside template" report.
        for html in [
            concat!(
                r#"<f-template name="script-card"><template>"#,
                r#"<script>const s = "</f-template>"; const t = "</template>";"#,
                r#"<span>{{label}}</span></template></f-template>"#,
            ),
            concat!(
                r#"<f-template name="script-card"><template>"#,
                r#"<style>/* </f-template> */ const t = "</template>";"#,
                r#"<span>{{label}}</span></template></f-template>"#,
            ),
        ] {
            let err = transform("file-card", html)
                .expect_err("unterminated raw-text body should be an unclosed-element error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(codes::UNCLOSED_HTML_TAG));
            assert_eq!(diag.snippet_text(), Some("<f-template>"));
            assert_eq!(
                diag.help_text(),
                Some("close the reported FAST template element or opening tag")
            );
        }
    }

    #[test]
    fn meaningful_content_after_f_template_is_a_diagnostic() {
        // Explicit after-sibling coverage: meaningful content following
        // </f-template> is rejected the same as content before it.
        let err = transform(
            "invalid-card",
            r#"<f-template name="invalid-card"><template><span>{{label}}</span></template></f-template><div>after</div>"#,
        )
        .expect_err("trailing top-level content should error");

        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
        assert_eq!(diag.component_name(), Some("invalid-card"));
        assert_eq!(
            diag.help_text(),
            Some(
                "keep <f-template> as the only top-level authored content (outside content may only be whitespace or comments)"
            )
        );
    }

    #[test]
    fn ordinary_attributes_on_directives_are_rejected() {
        // A FAST directive accepts only `value`; ordinary attributes such as
        // `id`, `class`, and `data-*` must not be silently dropped. Covers both
        // <f-when> and <f-repeat>, and the valid single-`value` counterpart.
        for (attr, offending) in [
            (r#"id="x""#, "id"),
            (r#"class="c""#, "class"),
            (r#"data-role="btn""#, "data-role"),
        ] {
            for directive in ["f-when", "f-repeat"] {
                let value = if directive == "f-repeat" {
                    "{{item in items}}"
                } else {
                    "{{visible}}"
                };
                let html = format!(
                    r#"<f-template name="attr-card"><template><{directive} {attr} value="{value}"><span>{{{{label}}}}</span></{directive}></template></f-template>"#
                );
                let err = transform("attr-card", &html)
                    .expect_err("ordinary directive attribute should error");
                let ParserError::Template(diag) = err else {
                    panic!("expected template diagnostic");
                };
                assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
                assert_eq!(diag.component_name(), Some("attr-card"));
                assert_eq!(diag.snippet_text(), Some(offending));
                assert!(diag
                    .to_string()
                    .contains(&format!("does not support the '{offending}' attribute")));
            }
        }
    }

    #[test]
    fn directive_with_only_a_value_attribute_is_accepted() {
        let result = transformed(
            "attr-card",
            r#"<f-template name="attr-card"><template><f-when value="{{visible}}"><f-repeat value="{{item in items}}"><span>{{item.label}}</span></f-repeat></f-when></template></f-template>"#,
        );
        assert_eq!(
            result.parser_content,
            r#"<template><if condition="visible"><for each="item in items"><span>{{item.label}}</span></for></if></template>"#
        );
    }

    #[test]
    fn orphan_and_unsupported_closing_f_tags_are_rejected() {
        // Stray FAST closing tags must not leak into the WebUI parser view.
        // A directive close with no matching open and an unsupported `</f-*>`
        // element both report `invalid-fast-template` at the closing offset.
        for (html, snippet) in [
            (
                r#"<f-template name="orphan-card"><template><span>{{label}}</span></f-when></template></f-template>"#,
                "</f-when>",
            ),
            (
                r#"<f-template name="orphan-card"><template><span>{{label}}</span></f-repeat></template></f-template>"#,
                "</f-repeat>",
            ),
            (
                r#"<f-template name="orphan-card"><template><div></f-foo></div></template></f-template>"#,
                "<f-foo>",
            ),
        ] {
            let err =
                transform("orphan-card", html).expect_err("stray closing f-* tag should error");
            let ParserError::Template(diag) = err else {
                panic!("expected template diagnostic");
            };
            assert_eq!(diag.error_code(), Some(INVALID_FAST_TEMPLATE));
            assert_eq!(diag.component_name(), Some("orphan-card"));
            assert_eq!(diag.snippet_text(), Some(snippet));
        }
    }

    #[test]
    fn converter_diagnostics_carry_the_filename_component_for_json_output() {
        // Every FAST conversion diagnostic names the filename-derived component
        // so `webui build --format json` reports `--> file:line:col`, even when
        // the authored `<f-template name>` was never resolved.
        let err = transform(
            "file-card",
            "<f-template name=\"file-card\">\n  <template>\n    <f-choose></f-choose>\n  </template>\n</f-template>",
        )
        .expect_err("unsupported FAST element should error");
        let ParserError::Template(diag) = err else {
            panic!("expected template diagnostic");
        };
        assert_eq!(diag.component_name(), Some("file-card"));
        assert_eq!(diag.position_line_column(), Some((3, 5)));
        assert_eq!(
            diag.location().as_deref(),
            Some("--> file-card:3:5"),
            "structured location must survive as `--> file:line:col`"
        );
    }
}
