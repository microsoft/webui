// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Centralized, structured diagnostics for build-time template-authoring
//! mistakes.
//!
//! Every "the developer wrote invalid template syntax" problem is described by
//! a [`Diagnostic`] so they share one consistent shape (title + location +
//! offending snippet + actionable `help`). A [`Diagnostic`] is **plain data**:
//! it carries no color and never decides whether output is a terminal.
//!
//! Presentation is the responsibility of the **entry point**, not the library:
//! - `webui-cli` reads the structured fields and colorizes them with
//!   `console::style()` for a friendly terminal report.
//! - `webui-ffi`, `webui-node`, and `webui-wasm` surface the plain
//!   [`fmt::Display`] text through their own error channel
//!   (`webui_last_error`, `napi::Error`, `JsValue`) so the host application can
//!   handle it however it likes.
//!
//! A [`Diagnostic`] reaches those consumers as the payload of
//! [`crate::ParserError::Template`]; its [`fmt::Display`] renders the plain
//! report below:
//!
//! ```text
//! error: invalid @pointerdown handler [invalid-event-handler]
//!   in component <mail-inbox-page> · element <button>
//!     @pointerdown="e.preventDefault()"
//!   help: use @pointerdown="{handler()}" or @pointerdown="{handler(e)}" to pass the event
//! ```

use std::fmt;
use std::fmt::Write as _;

/// Severity of a [`Diagnostic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// A fatal authoring error — the build cannot continue.
    Error,
    /// A non-fatal warning — the build continues. Surfaced e.g. for CSS tokens
    /// used only with a literal `var()` fallback and absent from every theme.
    Warning,
}

impl Severity {
    /// Human label printed before the title (`error` / `warning`).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// Stable, machine-readable diagnostic codes.
///
/// Each constant is a permanent identifier for one class of authoring mistake.
/// The code is rendered alongside every diagnostic and emitted as a structured
/// field by `webui build --format json`, so editors, CI, and AI assistants can
/// map an error to a deterministic fix or docs page regardless of the
/// human-readable wording. Treat these as a stable API surface — rename only
/// with a deliberate, documented migration.
pub mod codes {
    /// `<for>` element is missing its required `each` attribute.
    pub const MISSING_FOR_EACH: &str = "missing-for-each";
    /// `<for each>` value is not of the form `item in collection`.
    pub const INVALID_FOR_EACH: &str = "invalid-for-each";
    /// `<for each>` item or collection name uses disallowed characters.
    pub const INVALID_FOR_IDENTIFIER: &str = "invalid-for-identifier";
    /// `<if>` element is missing its required `condition` attribute.
    pub const MISSING_IF_CONDITION: &str = "missing-if-condition";
    /// `<if condition>` value is not a parseable expression.
    pub const INVALID_IF_CONDITION: &str = "invalid-if-condition";
    /// A component tag has no matching registered component.
    pub const UNKNOWN_COMPONENT: &str = "unknown-component";
    /// An `@event` handler value is not a valid `{handler()}` expression.
    pub const INVALID_EVENT_HANDLER: &str = "invalid-event-handler";
    /// A scriptless HTML component contains event bindings and needs authored JS.
    pub const SCRIPTLESS_EVENT_HANDLER: &str = "scriptless-event-handler";
    /// A `w-ref` binding is missing its required `{braces}`.
    pub const INVALID_W_REF: &str = "invalid-w-ref";
    /// An HTML element is missing its closing tag.
    pub const UNCLOSED_HTML_TAG: &str = "unclosed-html-tag";
    /// An HTML tag could not be parsed (missing `>`, stray `<`, …).
    pub const MALFORMED_HTML_TAG: &str = "malformed-html-tag";
    /// A closing tag has no matching opening tag.
    pub const UNEXPECTED_CLOSING_TAG: &str = "unexpected-closing-tag";
    /// An HTML comment was opened but never closed with `-->`.
    pub const UNTERMINATED_HTML_COMMENT: &str = "unterminated-html-comment";
    /// An HTML declaration was opened but never closed with `>`.
    pub const UNTERMINATED_HTML_DECLARATION: &str = "unterminated-html-declaration";
    /// Template nesting exceeds the parser's depth limit.
    pub const EXCESSIVE_NESTING: &str = "excessive-nesting";
    /// A template references itself (directly or transitively) at build time.
    pub const RECURSIVE_TEMPLATE: &str = "recursive-template";
    /// A `<style>` block contains malformed CSS.
    pub const INVALID_CSS: &str = "invalid-css";
    /// A CSS token required by parser output is missing from the configured theme.
    pub const MISSING_THEME_TOKEN: &str = "missing-theme-token";
    /// A CSS token used only with a literal `var()` fallback and absent from
    /// every theme — non-fatal, but usually a typo. Severity: warning.
    pub const UNTHEMED_TOKEN: &str = "unthemed-token";
}

/// A build-time template-authoring diagnostic.
///
/// Construct with [`Diagnostic::error`] / [`Diagnostic::warning`], attach
/// context with the builder methods, then return it inside
/// [`crate::ParserError::Template`]. The library never colors or prints it; the
/// CLI reads the fields via the getters and renders them, while other hosts use
/// the plain [`fmt::Display`] text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    severity: Severity,
    /// Stable, machine-readable code (e.g. `invalid-for-each`); see [`codes`].
    code: Option<&'static str>,
    title: String,
    component: Option<String>,
    element: Option<String>,
    snippet: Option<String>,
    help: Option<String>,
    /// 1-based `(line, column)` of the offending source location, if known.
    position: Option<(usize, usize)>,
}

impl Diagnostic {
    /// Start a fatal error diagnostic. Titles are short and lowercase by
    /// convention (e.g. `invalid @click handler`).
    #[must_use]
    pub fn error(title: impl Into<String>) -> Self {
        Self::new(Severity::Error, title)
    }

    /// Start a non-fatal warning diagnostic.
    ///
    /// Rendered with the same location/snippet/`help:` layout as an error, but
    /// the build continues (e.g. an unthemed literal-fallback CSS token).
    #[must_use]
    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(Severity::Warning, title)
    }

    fn new(severity: Severity, title: impl Into<String>) -> Self {
        Self {
            severity,
            code: None,
            title: title.into(),
            component: None,
            element: None,
            snippet: None,
            help: None,
            position: None,
        }
    }

    /// Attach a stable, machine-readable error [`code`](codes) (e.g.
    /// `invalid-for-each`).
    ///
    /// The code is a constant identifier that does not change with the wording
    /// of the message, so tools and AI assistants can recognize the error class
    /// and apply a deterministic fix. Prefer the constants in [`codes`].
    #[must_use]
    pub fn code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Name the owning component template (e.g. `mail-inbox-page`).
    #[must_use]
    pub fn component(mut self, component: impl Into<String>) -> Self {
        self.component = Some(component.into());
        self
    }

    /// Name the offending element tag (e.g. `button`), when known.
    #[must_use]
    pub fn element(mut self, element: impl Into<String>) -> Self {
        self.element = Some(element.into());
        self
    }

    /// Attach the offending source snippet (e.g. `@click="bad()"`).
    #[must_use]
    pub fn snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Attach a `help:` hint describing the fix.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attach the 1-based `(line, column)` of the offending source location.
    #[must_use]
    pub fn position(mut self, line: usize, column: usize) -> Self {
        self.position = Some((line, column));
        self
    }

    /// Attach the source location computed from a byte `offset` into `source`.
    ///
    /// Convenience over [`Diagnostic::position`] for callers that hold the
    /// source text and a byte offset (e.g. an element's start). Computes the
    /// 1-based line and column in a single forward scan.
    #[must_use]
    pub fn at_offset(self, source: &str, offset: usize) -> Self {
        let (line, column) = line_column(source, offset);
        self.position(line, column)
    }

    /// Severity of this diagnostic.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The stable, machine-readable error [`code`](codes), if set.
    #[must_use]
    pub fn error_code(&self) -> Option<&'static str> {
        self.code
    }

    /// The short error title (e.g. `invalid @click handler`).
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The owning component tag, if set.
    #[must_use]
    pub fn component_name(&self) -> Option<&str> {
        self.component.as_deref()
    }

    /// The offending element tag, if set.
    #[must_use]
    pub fn element_name(&self) -> Option<&str> {
        self.element.as_deref()
    }

    /// The offending source snippet, if set.
    #[must_use]
    pub fn snippet_text(&self) -> Option<&str> {
        self.snippet.as_deref()
    }

    /// The `help:` hint, if set.
    #[must_use]
    pub fn help_text(&self) -> Option<&str> {
        self.help.as_deref()
    }

    /// The 1-based `(line, column)` of the offending source location, if known.
    #[must_use]
    pub fn position_line_column(&self) -> Option<(usize, usize)> {
        self.position
    }

    /// Render the location line content (already including its leading marker),
    /// if any.
    ///
    /// When a source position is known it is reported rustc-style as
    /// `--> owner:line:column` (the `owner` is the entry file or component
    /// tag), which is compact and recognizable. Without a position it falls
    /// back to a descriptive `in component <c> · element <e>` form. Returned so
    /// callers (e.g. the CLI) can render the location on its own styled line.
    #[must_use]
    pub fn location(&self) -> Option<String> {
        if let Some((line, column)) = self.position {
            return Some(match &self.component {
                Some(c) => format!("--> {c}:{line}:{column}"),
                None => format!("--> {line}:{column}"),
            });
        }
        match (&self.component, &self.element) {
            (Some(c), Some(e)) => Some(format!("in component <{c}> · element <{e}>")),
            (Some(c), None) => Some(format!("in component <{c}>")),
            (None, Some(e)) => Some(format!("in element <{e}>")),
            (None, None) => None,
        }
    }

    /// Render the message body — everything after the severity label: title,
    /// location, offending snippet, and `help:` hint, each on its own line.
    ///
    /// Used by callers that supply their own leading label (e.g. the dev
    /// server prints `build error: {body}`) so the severity word isn't
    /// duplicated. [`fmt::Display`] prepends the severity to this body.
    #[must_use]
    pub fn body(&self) -> String {
        let mut out = String::with_capacity(96);
        out.push_str(&self.title);
        if let Some(code) = self.code {
            let _ = write!(out, " [{code}]");
        }
        if let Some(location) = self.location() {
            let _ = write!(out, "\n  {location}");
        }
        if let Some(snippet) = &self.snippet {
            let _ = write!(out, "\n    {snippet}");
        }
        if let Some(help) = &self.help {
            let _ = write!(out, "\n  help: {help}");
        }
        out
    }
}

/// Compute the 1-based `(line, column)` of a byte `offset` into `source`.
///
/// A single forward scan over the bytes preceding `offset` (no regex, no
/// allocation); the column counts characters within the line so multi-byte
/// UTF-8 is reported correctly.
#[must_use]
pub(crate) fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1usize;
    let mut line_start = 0usize;
    for (i, &b) in source.as_bytes()[..offset].iter().enumerate() {
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let column = source[line_start..offset].chars().count() + 1;
    (line, column)
}

impl fmt::Display for Diagnostic {
    /// Render the plain, color-free report. Hosts that want color (the CLI)
    /// read the fields via the getters instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.severity.label(), self.body())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_full_error_compact() {
        let diag = Diagnostic::error("invalid @pointerdown handler")
            .component("mail-inbox-page")
            .element("button")
            .snippet(r#"@pointerdown="e.preventDefault()""#)
            .help(r#"use @pointerdown="{handler()}" to pass the event"#);

        let expected = "error: invalid @pointerdown handler\n  \
             in component <mail-inbox-page> · element <button>\n    \
             @pointerdown=\"e.preventDefault()\"\n  \
             help: use @pointerdown=\"{handler()}\" to pass the event";

        assert_eq!(diag.to_string(), expected);
    }

    #[test]
    fn omits_optional_sections_when_absent() {
        let diag = Diagnostic::warning("something minor").component("my-card");
        assert_eq!(
            diag.to_string(),
            "warning: something minor\n  in component <my-card>"
        );
    }

    #[test]
    fn element_only_location() {
        let diag = Diagnostic::error("x").element("button");
        assert_eq!(diag.to_string(), "error: x\n  in element <button>");
    }

    #[test]
    fn getters_expose_structured_fields() {
        let diag = Diagnostic::error("boom")
            .component("c")
            .element("e")
            .snippet("snip")
            .help("do this");
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.title(), "boom");
        assert_eq!(diag.component_name(), Some("c"));
        assert_eq!(diag.element_name(), Some("e"));
        assert_eq!(diag.snippet_text(), Some("snip"));
        assert_eq!(diag.help_text(), Some("do this"));
        assert_eq!(
            diag.location().as_deref(),
            Some("in component <c> · element <e>")
        );
    }

    #[test]
    fn position_renders_rustc_style_location() {
        let diag = Diagnostic::error("boom")
            .component("index.html")
            .element("for")
            .position(3, 7);
        assert_eq!(diag.position_line_column(), Some((3, 7)));
        // A known position overrides the descriptive form with `--> file:l:c`.
        assert_eq!(diag.location().as_deref(), Some("--> index.html:3:7"));
        assert!(diag.to_string().contains("--> index.html:3:7"));
    }

    #[test]
    fn bare_position_renders_without_component_or_element() {
        let diag = Diagnostic::error("boom").position(5, 1);
        assert_eq!(diag.location().as_deref(), Some("--> 5:1"));
    }

    #[test]
    fn line_column_counts_lines_and_chars() {
        let src = "<div>\n  <for each=\"x\">\n</div>";
        // Offset of the '<' in "<for" — line 2, column 3 (after two spaces).
        let offset = src.find("<for").unwrap();
        assert_eq!(line_column(src, offset), (2, 3));
        // Start of file.
        assert_eq!(line_column(src, 0), (1, 1));
    }

    #[test]
    fn line_column_uses_char_columns_for_multibyte() {
        // "📚x" — the 'x' is one char in, though the emoji is 4 bytes.
        let src = "📚x";
        let offset = src.find('x').unwrap();
        assert_eq!(line_column(src, offset), (1, 2));
    }
}
