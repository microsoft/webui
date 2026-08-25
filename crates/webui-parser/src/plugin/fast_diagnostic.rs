// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST-owned diagnostic codes and builders shared by versioned FAST plugins.

use super::fast_convert::{ConvertError, ConvertErrorKind};
use crate::diagnostic::{codes, Diagnostic};
use crate::ParserError;

// FAST cannot mount a Light DOM component faithfully.
pub(crate) const FAST_LIGHT_DOM_UNSUPPORTED: &str = "fast-light-dom-unsupported";
// A FAST source contains more than one `<f-template>`.
pub(crate) const UNSUPPORTED_MULTIPLE_F_TEMPLATES: &str = "unsupported-multiple-f-templates";
// A FAST source uses malformed or unsupported syntax.
pub(crate) const INVALID_FAST_TEMPLATE: &str = "invalid-fast-template";

// Build a diagnostic from a FAST conversion failure.
#[cold]
#[inline(never)]
pub(super) fn converter_error(
    source: &str,
    tag_name: &str,
    error: &ConvertError<'_>,
) -> ParserError {
    if matches!(error.kind(), ConvertErrorKind::MultipleFTemplates { .. }) {
        return Diagnostic::error("multiple <f-template> elements are not supported")
            .code(UNSUPPORTED_MULTIPLE_F_TEMPLATES)
            .component(tag_name)
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
        ConvertErrorKind::ContentAroundInnerTemplate => {
            "keep only a single inner <template> inside <f-template> (surrounding content may only be whitespace or comments)"
        }
        ConvertErrorKind::ContentOutsideTemplate => {
            "keep <f-template> as the only top-level authored content (outside content may only be whitespace or comments)"
        }
        ConvertErrorKind::MissingValueAttribute { .. }
        | ConvertErrorKind::InvalidDirectiveValue { .. } => {
            "add the required value=\"{{expression}}\" attribute to the FAST directive"
        }
        ConvertErrorKind::UnexpectedDirectiveAttribute { .. } => {
            "FAST directives (<f-when>/<f-repeat>) accept only a value=\"{{expression}}\" attribute; remove the others"
        }
        ConvertErrorKind::UnsupportedWrapperAttribute { .. } => {
            "the <f-template> wrapper accepts only 'name' and 'shadowroot*' shadow options (e.g. shadowrootmode, shadowrootdelegatesfocus); remove the other attribute"
        }
        ConvertErrorKind::ConditionQuoteConflict { .. } => {
            "rewrite the f-when condition to use a single quote style (only single or only double quotes)"
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
        ConvertErrorKind::UnexpectedClosingDirective { .. } => {
            "remove the stray closing tag or add its matching opening FAST directive"
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
        INVALID_FAST_TEMPLATE
    };

    Diagnostic::error(format!("invalid FAST template: {error}"))
        .code(code)
        .component(tag_name)
        .at_offset(source, error.offset())
        .snippet(converter_error_snippet(error.kind()))
        .help(help)
        .into()
}

// Build the diagnostic for unsupported effective Light DOM.
#[cold]
#[inline(never)]
pub(super) fn light_dom_unsupported(tag_name: &str) -> ParserError {
    Diagnostic::error("FAST plugins require effective Shadow DOM")
        .code(FAST_LIGHT_DOM_UNSUPPORTED)
        .component(tag_name)
        .help(
            "build with `dom: \"shadow\"`, author an open declarative Shadow root for this component, or use the WebUI plugin for global Light DOM",
        )
        .into()
}

#[cold]
#[inline(never)]
fn converter_error_snippet(error: &ConvertErrorKind<'_>) -> String {
    match error {
        ConvertErrorKind::MultipleFTemplates { .. }
        | ConvertErrorKind::MissingInnerTemplate
        | ConvertErrorKind::MultipleInnerTemplates { .. }
        | ConvertErrorKind::ContentOutsideTemplate => "<f-template>".to_string(),
        ConvertErrorKind::ContentAroundInnerTemplate => "<template>".to_string(),
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
        ConvertErrorKind::UnexpectedClosingDirective { tag } => {
            let mut snippet = String::with_capacity(tag.len() + 3);
            snippet.push_str("</");
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
        ConvertErrorKind::ConditionQuoteConflict { value } => value.to_string(),
        ConvertErrorKind::UnsupportedFAttribute { attribute }
        | ConvertErrorKind::UnexpectedDirectiveAttribute { attribute, .. } => attribute.to_string(),
        ConvertErrorKind::UnsupportedWrapperAttribute { attribute } => attribute.to_string(),
        ConvertErrorKind::UnclosedTag => "<".to_string(),
    }
}
