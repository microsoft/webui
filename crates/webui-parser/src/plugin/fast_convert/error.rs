// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fmt;

/// FAST conversion failure and the source offset used for its diagnostic.
#[derive(Debug)]
pub(crate) struct ConvertError<'a> {
    kind: ConvertErrorKind<'a>,
    offset: usize,
}

impl<'a> ConvertError<'a> {
    #[cold]
    #[inline(never)]
    pub(super) fn new(kind: ConvertErrorKind<'a>, offset: usize) -> Self {
        Self { kind, offset }
    }

    #[inline]
    pub(crate) fn kind(&self) -> &ConvertErrorKind<'a> {
        &self.kind
    }

    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }
}

impl fmt::Display for ConvertError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ConvertErrorKind::MultipleFTemplates { count } => write!(
                f,
                "template validation error: expected exactly one '<f-template>' element, found {count}"
            ),
            ConvertErrorKind::MissingInnerTemplate => write!(
                f,
                "template validation error: '<f-template>' must contain exactly one inner '<template>' element"
            ),
            ConvertErrorKind::MultipleInnerTemplates { count } => write!(
                f,
                "template validation error: '<f-template>' must contain exactly one inner '<template>' element, found {count}"
            ),
            ConvertErrorKind::ContentOutsideTemplate => write!(
                f,
                "template validation error: '<f-template>' must be the only top-level authored content in the file"
            ),
            ConvertErrorKind::ContentAroundInnerTemplate => write!(
                f,
                "template validation error: '<f-template>' may only contain a single inner '<template>' element; surrounding markup is not supported"
            ),
            ConvertErrorKind::UnclosedElement { tag } => write!(
                f,
                "unclosed element '<{tag}>': no matching '</{tag}>' closing tag was found"
            ),
            ConvertErrorKind::UnclosedTag => {
                f.write_str("unclosed tag: no closing '>' was found")
            }
            ConvertErrorKind::MissingValueAttribute { tag } => write!(
                f,
                "directive '<{tag}>' is missing a valid 'value=\"{{{{…}}}}\"' attribute"
            ),
            ConvertErrorKind::InvalidDirectiveValue { tag, value } => write!(
                f,
                "directive '<{tag}>' has invalid value '{}': expected 'value=\"{{{{…}}}}\"'",
                value
            ),
            ConvertErrorKind::UnexpectedDirectiveAttribute { tag, attribute } => write!(
                f,
                "directive '<{tag}>' does not support the '{attribute}' attribute; only 'value' is allowed"
            ),
            ConvertErrorKind::UnsupportedWrapperAttribute { attribute } => write!(
                f,
                "'<f-template>' does not support the '{attribute}' attribute; only 'name' and 'shadowroot*' shadow options are allowed"
            ),
            ConvertErrorKind::ConditionQuoteConflict { value } => write!(
                f,
                "f-when condition '{value}' mixes single and double quotes, which cannot be represented in a generated '<if condition>' attribute"
            ),
            ConvertErrorKind::InvalidRepeatExpression { expr } => write!(
                f,
                "invalid repeat expression '{{{{{expr}}}}}': expected 'item in items' format"
            ),
            ConvertErrorKind::UnsupportedFAttribute { attribute } => {
                write!(f, "unsupported f-* attribute '{attribute}'")
            }
            ConvertErrorKind::UnsupportedFElement { tag } => {
                write!(f, "unsupported f-* element '<{tag}>'")
            }
            ConvertErrorKind::UnexpectedClosingDirective { tag } => write!(
                f,
                "unexpected closing '</{tag}>': no matching opening '<{tag}>' was found"
            ),
        }
    }
}

/// FAST conversion error categories used to select stable WebUI diagnostics.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConvertErrorKind<'a> {
    MultipleFTemplates {
        count: usize,
    },
    MissingInnerTemplate,
    MultipleInnerTemplates {
        count: usize,
    },
    ContentOutsideTemplate,
    ContentAroundInnerTemplate,
    UnclosedElement {
        tag: &'a str,
    },
    UnclosedTag,
    MissingValueAttribute {
        tag: &'static str,
    },
    InvalidDirectiveValue {
        tag: &'static str,
        value: &'a str,
    },
    UnexpectedDirectiveAttribute {
        tag: &'static str,
        attribute: &'a str,
    },
    UnsupportedWrapperAttribute {
        attribute: &'a str,
    },
    ConditionQuoteConflict {
        value: &'a str,
    },
    InvalidRepeatExpression {
        expr: &'a str,
    },
    UnsupportedFAttribute {
        attribute: &'a str,
    },
    UnsupportedFElement {
        tag: &'a str,
    },
    UnexpectedClosingDirective {
        tag: &'static str,
    },
}
