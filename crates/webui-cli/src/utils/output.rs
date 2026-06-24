// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::OnceLock;

use webui::{Diagnostic, ParserError, Severity, WebUIError};

/// CLI output format, selected with the global `--format` flag.
///
/// `Human` is the default colorized terminal rendering. `Json` emits a single
/// machine-readable object per error on **stdout** (and suppresses decorative
/// human output) so editors, CI, and AI assistants can consume diagnostics
/// without scraping ANSI text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Colorized, human-readable terminal output (default).
    #[default]
    Human,
    /// Machine-readable JSON diagnostics on stdout.
    Json,
}

static FORMAT: OnceLock<OutputFormat> = OnceLock::new();

/// Set the process-wide CLI output format. Call once at startup, before any
/// output is produced; later calls are ignored.
pub fn set_format(format: OutputFormat) {
    let _ = FORMAT.set(format);
}

/// The configured output format (defaults to [`OutputFormat::Human`]).
#[must_use]
pub fn format() -> OutputFormat {
    FORMAT.get().copied().unwrap_or_default()
}

#[must_use]
fn is_json() -> bool {
    matches!(format(), OutputFormat::Json)
}

pub fn header(title: &str) {
    if is_json() {
        return;
    }
    eprintln!(
        "\n  {} {}\n",
        console::style("⚡").cyan().bold(),
        console::style(title).cyan().bold()
    );
}

pub fn field(label: &str, value: &dyn std::fmt::Display) {
    if is_json() {
        return;
    }
    eprintln!(
        "  {} {}",
        console::style(format!("▸ {label:<10}")).dim(),
        console::style(value).bold()
    );
}

pub fn success(message: &str) {
    if is_json() {
        return;
    }
    eprintln!("  {} {message}", console::style("✔").green());
}

pub fn finish(message: &str) {
    if is_json() {
        return;
    }
    eprintln!("\n  {} {message}\n", console::style("✨").green());
}

/// Find a build-time template [`Diagnostic`] within an error's source chain.
fn template_diagnostic(err: &anyhow::Error) -> Option<&Diagnostic> {
    err.chain()
        .find_map(|cause| match cause.downcast_ref::<WebUIError>() {
            Some(WebUIError::Parse {
                source: ParserError::Template(diag),
                ..
            }) => Some(&**diag),
            _ => None,
        })
}

/// Serialize an error as a single machine-readable JSON object.
///
/// A structured template [`Diagnostic`] contributes its `code`, `severity`,
/// owning `file`, `line`/`column`, `snippet`, and `help`; any error also
/// carries the flattened source `chain`. Fields that don't apply are `null`.
///
/// Built via the `serde_json::Map` API rather than the `json!` macro, whose
/// expansion trips the workspace `disallowed_methods` lint (it `unwrap`s on
/// internally-constructed values).
#[must_use]
fn error_json(err: &anyhow::Error) -> serde_json::Value {
    use serde_json::Value;

    let chain: Vec<Value> = err.chain().map(|c| Value::from(c.to_string())).collect();
    let str_or_null = |value: Option<&str>| value.map_or(Value::Null, Value::from);

    let mut map = serde_json::Map::new();
    match template_diagnostic(err) {
        Some(diag) => {
            let (line, column) = match diag.position_line_column() {
                Some((line, column)) => (Value::from(line), Value::from(column)),
                None => (Value::Null, Value::Null),
            };
            map.insert("severity".into(), Value::from(diag.severity().label()));
            map.insert("code".into(), str_or_null(diag.error_code()));
            map.insert("message".into(), Value::from(diag.title()));
            map.insert("file".into(), str_or_null(diag.component_name()));
            map.insert("line".into(), line);
            map.insert("column".into(), column);
            map.insert("snippet".into(), str_or_null(diag.snippet_text()));
            map.insert("help".into(), str_or_null(diag.help_text()));
        }
        None => {
            // Keep the same key set as the structured branch so `--format json`
            // has one stable shape; fields that don't apply are null.
            map.insert("severity".into(), Value::from("error"));
            map.insert("code".into(), Value::Null);
            map.insert("message".into(), Value::from(err.to_string()));
            map.insert("file".into(), Value::Null);
            map.insert("line".into(), Value::Null);
            map.insert("column".into(), Value::Null);
            map.insert("snippet".into(), Value::Null);
            map.insert("help".into(), Value::Null);
        }
    }
    map.insert("chain".into(), Value::Array(chain));
    Value::Object(map)
}

pub fn error(err: &anyhow::Error) {
    if is_json() {
        // One machine-readable object on stdout; tools parse this instead of
        // scraping the colorized stderr rendering.
        println!("{}", error_json(err));
        return;
    }

    // Build-time template-authoring mistakes carry a structured diagnostic;
    // render it in a friendly, colorized format instead of the generic chain.
    if let Some(diag) = template_diagnostic(err) {
        diagnostic(diag);
        return;
    }

    eprintln!(
        "\n  {} {}",
        console::style("✘").red().bold(),
        console::style(err).red().bold()
    );
    for cause in err.chain().skip(1) {
        eprintln!("  {} {cause}", console::style("caused by:").dim());
    }
}

/// Render a build error for the dev-server rebuild loop into a
/// `(display, message)` pair.
///
/// - `display` is per-line colorized for the terminal [`RebuildReporter`].
/// - `message` is plain, color-free text for the browser overlay /
///   live-reload console (ANSI must never reach a browser).
///
/// A template authoring mistake renders as its self-contained [`Diagnostic`]
/// (`title` + `--> file:line:col` + snippet + `help:`), so it isn't buried
/// under redundant `Build failed: Failed to parse …:` context. Any other error
/// flattens the anyhow chain for both renderings.
///
/// [`RebuildReporter`]: webui_dev_server::RebuildReporter
#[must_use]
pub fn build_error_renderings(err: &anyhow::Error) -> (String, String) {
    match template_diagnostic(err) {
        Some(diag) => (styled_diagnostic_body(diag), diag.body()),
        None => {
            let flat = format!("{err:#}");
            (flat.clone(), flat)
        }
    }
}

/// Build the per-line-colorized body of `diag` for the dev-server reporter.
///
/// Mirrors [`diagnostic`], but returns a string because the reporter prepends
/// its own `✘ build error:` / `⚠ build warning:` marker. Each line opens and
/// closes its own SGR span so the output survives being re-prefixed line-by-line
/// (e.g. `[server]` under `xtask dev`); a single span across newlines would
/// bleed. The title is colored by severity (red for errors, yellow for
/// warnings); the location/snippet/help styling is shared.
pub(crate) fn styled_diagnostic_body(diag: &Diagnostic) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(128);
    let styled_title = match diag.severity() {
        Severity::Warning => console::style(diag.title()).yellow().bold(),
        Severity::Error => console::style(diag.title()).red().bold(),
    };
    let _ = write!(out, "{styled_title}");
    if let Some(code) = diag.error_code() {
        let _ = write!(out, " {}", console::style(format!("[{code}]")).dim());
    }
    if let Some(location) = diag.location() {
        let _ = write!(out, "\n  {}", console::style(location).dim());
    }
    if let Some(snippet) = diag.snippet_text() {
        let _ = write!(out, "\n    {}", console::style(snippet).yellow());
    }
    if let Some(help) = diag.help_text() {
        let _ = write!(out, "\n  {} {help}", console::style("help:").cyan().bold());
    }
    out
}

/// Render a structured build-time [`Diagnostic`] with rustc-style coloring.
///
/// Color is the entry point's responsibility: the parser produces the plain
/// structured data, and the CLI styles it here.
pub fn diagnostic(diag: &Diagnostic) {
    let code = diag
        .error_code()
        .map(|c| format!(" {}", console::style(format!("[{c}]")).dim()))
        .unwrap_or_default();
    eprintln!(
        "\n  {} {}{}",
        console::style("✘").red().bold(),
        console::style(format!("{}: {}", diag.severity().label(), diag.title()))
            .red()
            .bold(),
        code
    );
    if let Some(location) = diag.location() {
        eprintln!("  {}", console::style(location).dim());
    }
    if let Some(snippet) = diag.snippet_text() {
        eprintln!("    {}", console::style(snippet).yellow());
    }
    if let Some(help) = diag.help_text() {
        eprintln!("  {} {help}", console::style("help:").cyan().bold());
    }
}

pub fn hint(message: &str) {
    if is_json() {
        return;
    }
    eprintln!("\n  {} {message}", console::style("hint:").dim());
}

/// Print a non-fatal build advisory as a multi-line warning [`Diagnostic`]
/// (yellow `⚠ build warning:`), mirroring the error layout with its
/// `--> file:line:col`, source snippet, and `help:` suggestion. A leading
/// blank line frames it. Suppressed in JSON mode.
pub fn warning_diagnostic(diag: &Diagnostic) {
    if is_json() {
        return;
    }
    eprintln!(
        "\n  {} {} {}",
        console::style("⚠").yellow().bold(),
        console::style("build warning:").yellow().bold(),
        styled_diagnostic_body(diag),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_error() -> anyhow::Error {
        let diag = Diagnostic::error("invalid <for> each expression")
            .code("invalid-for-each")
            .component("index.html")
            .position(67, 5)
            .snippet(r#"each="person inpeople""#)
            .help(r#"use the form each="item in collection", e.g. each="todo in todos""#);
        WebUIError::Parse {
            context: "Failed to parse index.html".to_owned(),
            source: ParserError::Template(Box::new(diag)),
        }
        .into()
    }

    #[test]
    fn build_error_message_is_plain_for_browser() {
        // The browser receives `message` via live-reload and logs it with
        // `console.error`; ANSI escape codes would render as literal garbage
        // and break the single-line SSE frame. It must stay color-free even
        // when terminal color is forced on.
        let prev = console::colors_enabled();
        console::set_colors_enabled(true);
        let (_display, message) = build_error_renderings(&template_error());
        console::set_colors_enabled(prev);

        assert!(
            !message.contains('\u{1b}'),
            "browser message must not contain ANSI escapes: {message:?}"
        );
        assert!(message.contains("invalid <for> each expression"));
        assert!(message.contains("--> index.html:67:5"));
        assert!(message.contains(r#"each="person inpeople""#));
        assert!(message.contains("help:"));
    }

    #[test]
    fn build_error_display_is_colorized_for_terminal() {
        let prev = console::colors_enabled();
        console::set_colors_enabled(true);
        let (display, _message) = build_error_renderings(&template_error());
        console::set_colors_enabled(prev);

        // The terminal rendering carries color...
        assert!(
            display.contains('\u{1b}'),
            "terminal display should be colorized when color is enabled: {display:?}"
        );
        // ...applied per line: no SGR span may straddle a newline, or it would
        // bleed when each line is re-prefixed (e.g. `[server]`). Every line
        // that opens a color must reset before its end.
        for line in display.lines() {
            if line.contains('\u{1b}') {
                assert!(
                    line.contains("\u{1b}[0m"),
                    "colored line must reset within itself: {line:?}"
                );
            }
        }
        // Same content as the plain body, just styled.
        assert!(console::strip_ansi_codes(&display).contains("invalid <for> each expression"));
        assert!(console::strip_ansi_codes(&display).contains("--> index.html:67:5"));
    }

    #[test]
    fn build_error_falls_back_to_flat_chain_without_diagnostic() {
        let err: anyhow::Error = WebUIError::Serialization("bad state json".to_owned()).into();
        let (display, message) = build_error_renderings(&err);
        assert_eq!(display, message);
        assert!(message.contains("bad state json"));
        assert!(!message.contains('\u{1b}'));
    }

    #[test]
    fn error_json_for_diagnostic_has_structured_fields() {
        let value = error_json(&template_error());
        assert_eq!(value["severity"], "error");
        assert_eq!(value["code"], "invalid-for-each");
        assert_eq!(value["message"], "invalid <for> each expression");
        assert_eq!(value["file"], "index.html");
        assert_eq!(value["line"], 67);
        assert_eq!(value["column"], 5);
        assert_eq!(value["snippet"], "each=\"person inpeople\"");
        assert!(value["help"]
            .as_str()
            .is_some_and(|h| h.contains("item in collection")));
        // The flattened source chain is always present.
        assert!(value["chain"].is_array());
        // The serialized object never carries ANSI escapes.
        assert!(!value.to_string().contains('\u{1b}'));
    }

    #[test]
    fn error_json_without_diagnostic_has_consistent_shape() {
        let err: anyhow::Error = WebUIError::Serialization("bad state json".to_owned()).into();
        let value = error_json(&err);
        assert_eq!(value["severity"], "error");
        assert!(value["code"].is_null());
        assert!(value["message"]
            .as_str()
            .is_some_and(|m| m.contains("bad state json")));
        assert!(value["chain"].is_array());
        // Non-applicable fields are present as null, not omitted, so tools see
        // one stable object shape regardless of error kind.
        for key in ["file", "line", "column", "snippet", "help"] {
            assert!(
                value.get(key).is_some_and(serde_json::Value::is_null),
                "expected `{key}` to be present and null"
            );
        }
    }
}
