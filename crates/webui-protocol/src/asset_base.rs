// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Validation for the public asset base.
//!
//! The same rules apply to the build option, the value stored in the
//! protocol, and the per-request `$webui.assetBase` override, so a base that
//! one layer accepts is never rejected by another.

use std::fmt;
use thiserror::Error;

/// Longest prefix of a rejected value quoted in an error message.
const MAX_QUOTED_CHARS: usize = 120;

/// Why a value cannot be used as an asset base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetBaseProblem {
    /// The value is empty.
    Empty,
    /// The value contains whitespace, a control character, a quote, `<`, `>`,
    /// or a backslash.
    UnsafeCharacter,
    /// The value contains a `?` query or a `#` fragment, so appending a path
    /// would not produce a path.
    QueryOrFragment,
    /// The value is a URL whose scheme is not `http` or `https`, a URL without
    /// a host, or a protocol-relative `//host/` reference.
    UnsupportedUrl,
    /// The value does not end with `/`.
    MissingTrailingSlash,
}

impl AssetBaseProblem {
    fn describe(self) -> &'static str {
        match self {
            Self::Empty => "it is empty",
            Self::UnsafeCharacter => {
                "it must not contain whitespace, control characters, quotes, '<', '>', or backslashes"
            }
            Self::QueryOrFragment => "it must not contain '?' or '#'",
            Self::UnsupportedUrl => {
                "only http:// and https:// URLs with a host, absolute paths, and relative paths are supported"
            }
            Self::MissingTrailingSlash => "it must end with '/'",
        }
    }
}

/// A rejected asset base.
///
/// The message names the value, the rule it breaks, and an accepted example.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "invalid asset base {}: {}. Use a URL or path ending in '/', e.g. 'https://cdn.example.com/app/' or '/static/'",
    QuotedValue(.value),
    .problem.describe()
)]
pub struct AssetBaseError {
    value: String,
    problem: AssetBaseProblem,
}

impl AssetBaseError {
    #[cold]
    #[inline(never)]
    fn new(value: &str, problem: AssetBaseProblem) -> Self {
        Self {
            value: value.chars().take(MAX_QUOTED_CHARS + 1).collect(),
            problem,
        }
    }

    /// The rule the value breaks.
    #[must_use]
    pub fn problem(&self) -> AssetBaseProblem {
        self.problem
    }
}

/// Quote a rejected value with escapes, shortening very long values.
struct QuotedValue<'a>(&'a str);

impl fmt::Display for QuotedValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.char_indices().nth(MAX_QUOTED_CHARS) {
            Some((end, _)) => write!(f, "{:?}...", &self.0[..end]),
            None => write!(f, "{:?}", self.0),
        }
    }
}

/// Check that `base` can prefix the asset paths WebUI emits.
///
/// A base must end with `/` and be either an `http://` or `https://` URL with
/// a host, an absolute path such as `/static/`, or a relative path such as
/// `static/`. It must not contain whitespace, control characters, quotes,
/// `<`, `>`, backslashes, `?`, or `#`.
///
/// # Errors
///
/// Returns [`AssetBaseError`] describing the first rule the value breaks.
pub fn validate_asset_base(base: &str) -> Result<(), AssetBaseError> {
    match asset_base_problem(base) {
        None => Ok(()),
        Some(problem) => Err(AssetBaseError::new(base, problem)),
    }
}

fn asset_base_problem(base: &str) -> Option<AssetBaseProblem> {
    if base.is_empty() {
        return Some(AssetBaseProblem::Empty);
    }
    if base.chars().any(|ch| {
        ch.is_whitespace() || ch.is_control() || matches!(ch, '"' | '\'' | '<' | '>' | '\\')
    }) {
        return Some(AssetBaseProblem::UnsafeCharacter);
    }
    if base.contains(['?', '#']) {
        return Some(AssetBaseProblem::QueryOrFragment);
    }
    if !has_supported_form(base) {
        return Some(AssetBaseProblem::UnsupportedUrl);
    }
    if !base.ends_with('/') {
        return Some(AssetBaseProblem::MissingTrailingSlash);
    }
    None
}

/// Accept paths and `http(s)://host` URLs; reject other schemes and
/// protocol-relative references.
fn has_supported_form(base: &str) -> bool {
    match base.find([':', '/']) {
        // A colon before any slash starts a URL scheme.
        Some(colon) if base.as_bytes()[colon] == b':' => {
            let scheme = &base[..colon];
            let host = base[colon + 1..].strip_prefix("//");
            (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
                && host.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
        }
        _ => !base.starts_with("//"),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_urls_and_paths_ending_in_a_slash() {
        for base in [
            "https://cdn.example.com/app/",
            "http://localhost:3000/",
            "HTTPS://cdn.example.com/b123/",
            "https://cdn.example.com/",
            "/static/",
            "/",
            "static/",
            "./",
            "../assets/",
            "_webui/",
            "https://cdn.example.com/a%20b/",
            "/caf\u{e9}/",
        ] {
            assert_eq!(validate_asset_base(base), Ok(()), "{base}");
        }
    }

    #[test]
    fn rejects_each_rule_with_its_own_problem() {
        for (base, problem) in [
            ("", AssetBaseProblem::Empty),
            ("static", AssetBaseProblem::MissingTrailingSlash),
            (
                "https://cdn.example.com",
                AssetBaseProblem::MissingTrailingSlash,
            ),
            ("/static/app.css", AssetBaseProblem::MissingTrailingSlash),
            ("a b/", AssetBaseProblem::UnsafeCharacter),
            ("a\tb/", AssetBaseProblem::UnsafeCharacter),
            ("a\u{0}b/", AssetBaseProblem::UnsafeCharacter),
            ("a\u{a0}b/", AssetBaseProblem::UnsafeCharacter),
            ("x\"/", AssetBaseProblem::UnsafeCharacter),
            ("x'/", AssetBaseProblem::UnsafeCharacter),
            ("<script>/", AssetBaseProblem::UnsafeCharacter),
            ("a>b/", AssetBaseProblem::UnsafeCharacter),
            ("a\\b/", AssetBaseProblem::UnsafeCharacter),
            ("/static/?v=1/", AssetBaseProblem::QueryOrFragment),
            ("/static/#x/", AssetBaseProblem::QueryOrFragment),
            ("javascript:alert(1)/", AssetBaseProblem::UnsupportedUrl),
            ("data:text/css,/", AssetBaseProblem::UnsupportedUrl),
            ("ftp://cdn.example.com/", AssetBaseProblem::UnsupportedUrl),
            ("C:/assets/", AssetBaseProblem::UnsupportedUrl),
            ("//cdn.example.com/app/", AssetBaseProblem::UnsupportedUrl),
            ("https:///app/", AssetBaseProblem::UnsupportedUrl),
            ("https://", AssetBaseProblem::UnsupportedUrl),
            ("https:/cdn.example.com/", AssetBaseProblem::UnsupportedUrl),
        ] {
            let error = validate_asset_base(base).expect_err(base);
            assert_eq!(error.problem(), problem, "{base:?}");
        }
    }

    #[test]
    fn errors_are_actionable_and_quote_bounded_values() {
        let message = validate_asset_base("packs").unwrap_err().to_string();
        assert_eq!(
            message,
            "invalid asset base \"packs\": it must end with '/'. Use a URL or path ending in '/', e.g. 'https://cdn.example.com/app/' or '/static/'"
        );

        let escaped = validate_asset_base("a\nb/").unwrap_err().to_string();
        assert!(
            escaped.starts_with("invalid asset base \"a\\nb/\":"),
            "{escaped}"
        );

        let long = "a".repeat(10_000);
        let message = validate_asset_base(&long).unwrap_err().to_string();
        assert!(message.len() < 400, "{message}");
        assert!(message.contains("\"..."), "{message}");
    }
}
