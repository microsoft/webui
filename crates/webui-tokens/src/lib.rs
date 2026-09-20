// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Design token loading, filtering, and CSS generation for WebUI.
//!
//! Loads token values from JSON files, filters them against the protocol's token
//! list, follows present transitive `var(--x)` dependencies for tokens in each
//! theme, and generates per-theme CSS declaration strings ready for state
//! injection. Theme token internals are trusted: unresolved or cyclic
//! dependencies remain browser CSS semantics rather than build failures.
//!
//! # Token file format
//!
//! Single multi-theme JSON:
//!
//! ```json
//! {
//!   "themes": {
//!     "light": { "surface-page": "#ffffff", "text-primary": "#111827" },
//!     "dark":  { "surface-page": "#171717", "text-primary": "#fafafa" }
//!   }
//! }
//! ```
//!
//! Or flat single-theme JSON (treated as a single `"default"` theme):
//!
//! ```json
//! { "surface-page": "#ffffff", "text-primary": "#111827" }
//! ```

mod error;

pub use error::{Result, TokenError};

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Loaded token themes — maps theme name → (token name → CSS value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenFile {
    /// Theme name → token map. Token names are without the `--` prefix.
    pub themes: HashMap<String, HashMap<String, String>>,
}

/// A CSS `var()` fallback chain that needs theme or local CSS coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssFallbackChain {
    /// Candidate custom property names in browser fallback order, without `--`.
    pub tokens: Vec<String>,
    /// Whether the CSS chain ends in a non-token literal fallback.
    ///
    /// When true, the chain is valid even if no candidate token exists in a
    /// theme because the browser can use the literal fallback.
    pub has_literal_fallback: bool,
}

/// Result of resolving tokens: per-theme CSS strings.
#[derive(Debug, Clone)]
pub struct ResolvedTokens {
    /// Theme name → CSS declarations string (e.g. `"--surface-page: #fff;\n--text-primary: #111;"`)
    pub css: HashMap<String, String>,
}

// ── Loading ──────────────────────────────────────────────────────────

/// Load a multi-theme token file.
///
/// Expects either:
/// - `{ "themes": { "light": { ... }, "dark": { ... } } }` (multi-theme)
/// - `{ "token-name": "value", ... }` (flat — treated as a single `"default"` theme)
///
/// # Errors
///
/// Returns [`TokenError::Io`] if the file cannot be read, [`TokenError::InvalidJson`]
/// if parsing fails, or [`TokenError::Schema`] if the structure is invalid.
pub fn load_token_file(path: &Path) -> Result<TokenFile> {
    let content = std::fs::read_to_string(path).map_err(|e| TokenError::io(path, e))?;
    parse_token_content(&content, path)
}

/// Parse token JSON content into a [`TokenFile`].
///
/// # Errors
///
/// Returns [`TokenError::InvalidJson`] or [`TokenError::Schema`] on bad input.
pub fn parse_token_content(content: &str, source_path: &Path) -> Result<TokenFile> {
    let value: Value =
        serde_json::from_str(content).map_err(|e| TokenError::json(source_path, e))?;

    let obj = value
        .as_object()
        .ok_or_else(|| TokenError::Schema("Token file must be a JSON object".into()))?;

    // Multi-theme format: { "themes": { ... } }
    if let Some(themes_val) = obj.get("themes") {
        let themes_obj = themes_val
            .as_object()
            .ok_or_else(|| TokenError::Schema("\"themes\" must be an object".into()))?;

        if themes_obj.is_empty() {
            return Err(TokenError::Schema(
                "\"themes\" must contain at least one theme".into(),
            ));
        }

        let mut themes = HashMap::with_capacity(themes_obj.len());
        for (theme_name, theme_val) in themes_obj {
            let tokens = parse_flat_token_map(theme_val, theme_name)?;
            themes.insert(theme_name.clone(), tokens);
        }
        return Ok(TokenFile { themes });
    }

    // Flat format: { "token-name": "value", ... } → single "default" theme
    // Skip $schema key if present
    let tokens = parse_flat_token_map(&value, "default")?;
    if tokens.is_empty() {
        return Err(TokenError::Schema(
            "Token file contains no token definitions".into(),
        ));
    }
    let mut themes = HashMap::with_capacity(1);
    themes.insert("default".into(), tokens);
    Ok(TokenFile { themes })
}

/// Parse a JSON value as a flat `{ "name": "value" }` token map.
fn parse_flat_token_map(value: &Value, theme_name: &str) -> Result<HashMap<String, String>> {
    let obj = value
        .as_object()
        .ok_or_else(|| TokenError::Schema(format!("Theme '{theme_name}' must be a JSON object")))?;

    let mut tokens = HashMap::with_capacity(obj.len());
    for (key, val) in obj {
        // Skip metadata keys like $schema
        if key.starts_with('$') {
            continue;
        }
        // Skip nested "themes" key (already handled at top level)
        if key == "themes" {
            continue;
        }
        match val.as_str() {
            Some(s) => {
                tokens.insert(key.clone(), s.to_string());
            }
            None => {
                return Err(TokenError::Schema(format!(
                    "Token '{key}' in theme '{theme_name}' must be a string value, got {}",
                    value_type_name(val)
                )));
            }
        }
    }
    Ok(tokens)
}

/// Human-readable JSON type name.
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── Theme resolution ─────────────────────────────────────────────────

/// Resolve a `--theme` value to a file path on disk.
///
/// Resolution order:
/// 1. If `theme` is a path to an existing file, use it directly.
/// 2. Otherwise, treat it as an npm package name and walk up from
///    `search_root` looking for `node_modules/<pkg>/tokens.json`:
///    - `@scope/pkg` → `node_modules/@scope/pkg/tokens.json`
///    - `@scope/pkg/custom.json` → `node_modules/@scope/pkg/custom.json`
///    - `pkg` → `node_modules/pkg/tokens.json`
///
/// # Errors
///
/// Returns [`TokenError::Schema`] if the theme cannot be resolved.
pub fn resolve_theme_path(theme: &str, search_root: &Path) -> Result<PathBuf> {
    // Direct file path?
    let as_path = PathBuf::from(theme);
    if as_path.exists() {
        return as_path
            .canonicalize()
            .map_err(|e| TokenError::Schema(format!("Failed to canonicalize {theme}: {e}")));
    }

    // Package name resolution: split into (package, subpath)
    let (pkg, subpath) = if theme.starts_with('@') {
        let parts: Vec<&str> = theme.splitn(3, '/').collect();
        if parts.len() >= 3 {
            let pkg = format!("{}/{}", parts[0], parts[1]);
            (pkg, parts[2].to_string())
        } else if parts.len() == 2 {
            (theme.to_string(), "tokens.json".to_string())
        } else {
            return Err(TokenError::Schema(format!(
                "Invalid scoped package name: {theme}"
            )));
        }
    } else if let Some((pkg, sub)) = theme.split_once('/') {
        (pkg.to_string(), sub.to_string())
    } else {
        (theme.to_string(), "tokens.json".to_string())
    };

    // Walk up directories looking for node_modules (standard Node resolution)
    let mut dir = search_root;
    loop {
        let candidate = dir.join("node_modules").join(&pkg).join(&subpath);
        if candidate.exists() {
            return candidate.canonicalize().map_err(|e| {
                TokenError::Schema(format!(
                    "Failed to canonicalize {}: {e}",
                    candidate.display()
                ))
            });
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    Err(TokenError::Schema(format!(
        "Theme '{theme}' not found. Looked for:\n  \
         1. File: {theme}\n  \
         2. Package: node_modules/{pkg}/{subpath} (walked up from {})\n\n  \
         Make sure the package is installed (pnpm install) or the path is correct.",
        search_root.display()
    )))
}

// ── Resolution ───────────────────────────────────────────────────────

/// Resolve tokens for all themes against the protocol's token candidate list.
///
/// For each theme:
/// 1. Keeps only protocol tokens that are present in that theme
/// 2. Follows present transitive `var(--x)` dependencies
/// 3. Generates CSS declaration strings
///
/// # Errors
///
/// Missing protocol token candidates and missing transitive dependencies are
/// skipped. Call [`validate_required_tokens`] first when flat token presence must
/// be strict; theme token values themselves are trusted and left to browser CSS
/// semantics.
pub fn resolve_tokens(
    protocol_tokens: &[String],
    token_file: &TokenFile,
) -> Result<ResolvedTokens> {
    let mut css = HashMap::with_capacity(token_file.themes.len());

    for (theme_name, theme_tokens) in &token_file.themes {
        let css_string = resolve_theme(protocol_tokens, theme_tokens);
        css.insert(theme_name.clone(), css_string);
    }

    Ok(ResolvedTokens { css })
}

/// Validate that every required token exists in every theme.
///
/// The caller selects which tokens are mandatory (e.g. only the candidates of
/// `var()` chains without a literal CSS fallback). Theme token values themselves
/// are trusted and are not dependency-validated.
///
/// # Errors
///
/// Returns [`TokenError::MissingToken`] if a required token is not present in
/// every theme.
pub fn validate_required_tokens(required_tokens: &[String], token_file: &TokenFile) -> Result<()> {
    for (theme_name, theme_tokens) in &token_file.themes {
        validate_theme_required_tokens(required_tokens, theme_tokens, theme_name)?;
    }
    Ok(())
}

/// Validate the tokens required by parser `var()` fallback chains against a theme.
///
/// A token is *required* when it appears in at least one chain with **no**
/// literal CSS fallback. A chain that terminates in a literal value
/// (`var(--x, 16px)`) does not make its candidates required — the literal
/// already supplies a value, so `--x` may be absent from a theme. Theme token
/// values themselves are trusted and are not dependency-validated.
///
/// # Errors
///
/// Returns [`TokenError::MissingToken`] if a required token is not present in
/// every theme.
pub fn validate_chain_tokens(chains: &[CssFallbackChain], token_file: &TokenFile) -> Result<()> {
    validate_required_tokens(&required_chain_tokens(chains), token_file)
}

/// CSS tokens used **only** with a literal `var()` fallback and defined in no
/// theme — likely typos.
///
/// A token referenced solely as `var(--name, <literal>)` is not a validation
/// failure (the literal supplies a value), but when no theme defines it the
/// reference is often a misspelling (e.g. `var(--colr-brand, #000)` for
/// `--color-brand`). Callers may surface these as non-fatal advisories.
/// Returned sorted and deduplicated, without the `--` prefix.
#[must_use]
pub fn unthemed_literal_fallback_tokens(
    chains: &[CssFallbackChain],
    token_file: &TokenFile,
) -> Vec<String> {
    let required: HashSet<&str> = chains
        .iter()
        .filter(|chain| !chain.has_literal_fallback)
        .flat_map(|chain| chain.tokens.iter().map(String::as_str))
        .collect();

    let mut unthemed: Vec<String> = chains
        .iter()
        .flat_map(|chain| chain.tokens.iter())
        .filter(|token| !required.contains(token.as_str()))
        .filter(|token| {
            token_file
                .themes
                .values()
                .all(|theme| !theme.contains_key(token.as_str()))
        })
        .cloned()
        .collect();
    unthemed.sort_unstable();
    unthemed.dedup();
    unthemed
}

/// Collect the tokens made mandatory by `var()` chains without a literal
/// fallback, sorted and deduplicated so validation order is deterministic.
fn required_chain_tokens(chains: &[CssFallbackChain]) -> Vec<String> {
    let mut required: Vec<String> = chains
        .iter()
        .filter(|chain| !chain.has_literal_fallback)
        .flat_map(|chain| chain.tokens.iter().cloned())
        .collect();
    required.sort_unstable();
    required.dedup();
    required
}

/// Resolve a single theme: filter, expand deps, generate CSS.
fn resolve_theme(required_tokens: &[String], theme_tokens: &HashMap<String, String>) -> String {
    let present_tokens: Vec<String> = required_tokens
        .iter()
        .filter(|token| theme_tokens.contains_key(*token))
        .cloned()
        .collect();
    let closure = theme_token_closure(&present_tokens, theme_tokens);
    generate_css_declarations(&closure, theme_tokens)
}

fn validate_theme_required_tokens(
    required_tokens: &[String],
    theme_tokens: &HashMap<String, String>,
    theme_name: &str,
) -> Result<()> {
    for name in required_tokens {
        if !theme_tokens.contains_key(name) {
            return Err(TokenError::MissingToken {
                theme: theme_name.into(),
                token: name.clone(),
            });
        }
    }
    Ok(())
}

fn theme_token_closure(
    required_tokens: &[String],
    theme_tokens: &HashMap<String, String>,
) -> Vec<String> {
    let required: HashSet<&str> = required_tokens.iter().map(String::as_str).collect();
    compute_token_closure(&required, theme_tokens)
}

/// Compute the transitive closure of required tokens by following present
/// `var(--x)` references in token values.
fn compute_token_closure(
    required: &HashSet<&str>,
    theme_tokens: &HashMap<String, String>,
) -> Vec<String> {
    let mut closure: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = required
        .iter()
        .filter(|name| theme_tokens.contains_key(**name))
        .map(|s| (*s).into())
        .collect();

    while let Some(name) = queue.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }

        if let Some(value) = theme_tokens.get(&name) {
            let deps = extract_var_references(value);
            for dep in deps {
                if theme_tokens.contains_key(&dep) && !closure.contains(&dep) {
                    queue.push(dep);
                }
            }
        }
    }

    let mut sorted: Vec<String> = closure.into_iter().collect();
    sorted.sort();
    sorted
}

// ── var(--x) extraction (no regex — deterministic scanner) ───────────

/// Extract `var(--name)` references from a CSS value string.
///
/// Uses a simple scanner instead of regex per project guidelines.
/// Handles nested `var()` calls and `var(--name, fallback)` syntax.
#[must_use]
pub fn extract_var_references(value: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i + 5 < len {
        // Look for "var(" prefix
        if bytes[i] == b'v' && bytes[i + 1] == b'a' && bytes[i + 2] == b'r' && bytes[i + 3] == b'('
        {
            i += 4;
            // Skip whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            // Expect "--"
            if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                i += 2;
                let start = i;
                // Read name: alphanumeric + hyphens
                while i < len
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
                {
                    i += 1;
                }
                if i > start {
                    let name = &value[start..i];
                    refs.push(name.into());
                }
            }
        } else {
            i += 1;
        }
    }

    refs
}

// ── CSS generation ───────────────────────────────────────────────────

/// Generate a CSS declarations string from the resolved token closure.
///
/// Tokens are sorted alphabetically for deterministic output. Each token
/// becomes a `--name: value;` declaration.
fn generate_css_declarations(closure: &[String], theme_tokens: &HashMap<String, String>) -> String {
    // Pre-calculate capacity: each line is roughly "--name: value;\n"
    let estimated_capacity: usize = closure
        .iter()
        .map(|name| {
            // "--" + name + ": " + value + ";\n"
            4 + name.len() + theme_tokens.get(name).map_or(0, |v| v.len()) + 2
        })
        .sum();

    let mut css = String::with_capacity(estimated_capacity);
    for (idx, name) in closure.iter().enumerate() {
        if let Some(value) = theme_tokens.get(name) {
            if idx > 0 {
                css.push('\n');
            }
            css.push_str("--");
            css.push_str(name);
            css.push_str(": ");
            css.push_str(value);
            css.push(';');
        }
    }
    css
}

/// Inject resolved token CSS into a JSON state value under `state.tokens`.
///
/// Sets `state["tokens"]["<theme>"] = "<css string>"` for each theme.
/// Creates the `"tokens"` object if it doesn't exist.
///
/// Silently returns without modification if `state` is not a JSON object.
pub fn inject_into_state(state: &mut Value, resolved: &ResolvedTokens) {
    let Some(state_map) = state.as_object_mut() else {
        return;
    };

    let tokens_obj = state_map
        .entry("tokens")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Some(map) = tokens_obj.as_object_mut() {
        for (theme_name, css_string) in &resolved.css {
            map.insert(theme_name.clone(), Value::String(css_string.clone()));
        }
    }
}

/// Inject pre-resolved per-theme CSS strings into a JSON state value.
///
/// This is the render-time equivalent of [`inject_into_state`] that works
/// with the pre-resolved `HashMap<String, String>` stored in server context
/// rather than re-resolving from a [`TokenFile`] each time.
///
/// Silently returns without modification if `state` is not a JSON object.
pub fn inject_token_css(state: &mut Value, token_css: &HashMap<String, String>) {
    let Some(state_map) = state.as_object_mut() else {
        return;
    };

    let tokens_obj = state_map
        .entry("tokens")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Some(map) = tokens_obj.as_object_mut() {
        for (theme, css) in token_css {
            map.insert(theme.clone(), Value::String(css.clone()));
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── var(--x) extraction tests ────────────────────────────────────

    #[test]
    fn extract_var_refs_simple() {
        let refs = extract_var_references("var(--color-primary)");
        assert_eq!(refs, vec!["color-primary"]);
    }

    #[test]
    fn extract_var_refs_multiple() {
        let refs =
            extract_var_references("color-mix(in oklch, var(--accent) 15%, var(--background))");
        assert_eq!(refs, vec!["accent", "background"]);
    }

    #[test]
    fn extract_var_refs_with_fallback() {
        let refs = extract_var_references("var(--color-brand, #0078d4)");
        assert_eq!(refs, vec!["color-brand"]);
    }

    #[test]
    fn extract_var_refs_nested() {
        let refs = extract_var_references("var(--a, var(--b))");
        assert_eq!(refs, vec!["a", "b"]);
    }

    #[test]
    fn extract_var_refs_with_whitespace() {
        let refs = extract_var_references("var( --spaced-name )");
        assert_eq!(refs, vec!["spaced-name"]);
    }

    #[test]
    fn extract_var_refs_no_refs() {
        let refs = extract_var_references("#0078d4");
        assert!(refs.is_empty());
    }

    #[test]
    fn extract_var_refs_underscore_name() {
        let refs = extract_var_references("var(--my_token)");
        assert_eq!(refs, vec!["my_token"]);
    }

    #[test]
    fn extract_var_refs_calc_expression() {
        let refs = extract_var_references("calc(var(--spacing-m) * 2)");
        assert_eq!(refs, vec!["spacing-m"]);
    }

    #[test]
    fn extract_var_refs_empty_string() {
        let refs = extract_var_references("");
        assert!(refs.is_empty());
    }

    #[test]
    fn extract_var_refs_short_string() {
        let refs = extract_var_references("abc");
        assert!(refs.is_empty());
    }

    // ── Token file loading tests ─────────────────────────────────────

    /// Write a JSON value to a temp file and return it.
    fn write_json_file(value: &Value) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        serde_json::to_writer(&mut file, value).unwrap();
        file
    }

    #[test]
    fn load_multi_theme_file() {
        let json = serde_json::json!({
            "themes": {
                "light": { "surface-page": "#ffffff", "text-primary": "#111827" },
                "dark": { "surface-page": "#171717", "text-primary": "#fafafa" }
            }
        });
        let file = write_json_file(&json);

        let tokens = load_token_file(file.path()).unwrap();
        assert_eq!(tokens.themes.len(), 2);
        assert_eq!(tokens.themes["light"]["surface-page"], "#ffffff");
        assert_eq!(tokens.themes["dark"]["text-primary"], "#fafafa");
    }

    #[test]
    fn load_flat_single_theme_file() {
        let json = serde_json::json!({
            "surface-page": "#ffffff",
            "text-primary": "#111827"
        });
        let file = write_json_file(&json);

        let tokens = load_token_file(file.path()).unwrap();
        assert_eq!(tokens.themes.len(), 1);
        assert!(tokens.themes.contains_key("default"));
        assert_eq!(tokens.themes["default"]["surface-page"], "#ffffff");
    }

    #[test]
    fn load_file_with_schema_key() {
        let json = serde_json::json!({
            "$schema": "https://webui.dev/schemas/tokens.json",
            "themes": {
                "light": { "color-brand": "#0078d4" }
            }
        });
        let file = write_json_file(&json);

        let tokens = load_token_file(file.path()).unwrap();
        assert_eq!(tokens.themes["light"]["color-brand"], "#0078d4");
    }

    #[test]
    fn load_empty_themes_returns_error() {
        let json = serde_json::json!({ "themes": {} });
        let file = write_json_file(&json);

        let result = load_token_file(file.path());
        assert!(matches!(result, Err(TokenError::Schema(_))));
    }

    #[test]
    fn load_non_string_value_returns_error() {
        let json = serde_json::json!({ "themes": { "light": { "bad": 42 } } });
        let file = write_json_file(&json);

        let result = load_token_file(file.path());
        assert!(
            matches!(result, Err(TokenError::Schema(ref msg)) if msg.contains("must be a string")),
            "expected schema error, got: {result:?}"
        );
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let result = load_token_file(Path::new("/nonexistent/tokens.json"));
        assert!(matches!(result, Err(TokenError::Io { .. })));
    }

    #[test]
    fn load_invalid_json_returns_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"{ not json }").unwrap();

        let result = load_token_file(file.path());
        assert!(matches!(result, Err(TokenError::InvalidJson { .. })));
    }

    // ── Resolution tests ─────────────────────────────────────────────

    #[test]
    fn resolve_filters_to_protocol_tokens() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("surface-page".into(), "#ffffff".into()),
                    ("text-primary".into(), "#111827".into()),
                    ("unused-token".into(), "#999999".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into(), "text-primary".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        let css = &resolved.css["light"];
        assert!(css.contains("--surface-page: #ffffff;"));
        assert!(css.contains("--text-primary: #111827;"));
        assert!(!css.contains("unused-token"));
    }

    #[test]
    fn resolve_expands_transitive_dependencies() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("surface-page".into(), "var(--neutral-100)".into()),
                    ("neutral-100".into(), "#f9fafb".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        let css = &resolved.css["light"];
        assert!(
            css.contains("--neutral-100: #f9fafb;"),
            "transitive dep should be included: {css}"
        );
        assert!(css.contains("--surface-page: var(--neutral-100);"));
    }

    #[test]
    fn resolve_deep_transitive_chain() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("a".into(), "var(--b)".into()),
                    ("b".into(), "var(--c)".into()),
                    ("c".into(), "#000".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["a".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        let css = &resolved.css["light"];

        assert!(css.contains("--a: var(--b);"), "a should be in CSS: {css}");
        assert!(css.contains("--b: var(--c);"), "b should be in CSS: {css}");
        assert!(css.contains("--c: #000;"), "c should be in CSS: {css}");
    }

    #[test]
    fn resolve_trusts_direct_cycle() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("a".into(), "var(--b)".into()),
                    ("b".into(), "var(--a)".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["a".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        let css = &resolved.css["light"];

        assert!(
            css.contains("--a: var(--b);"),
            "a should be included: {css}"
        );
        assert!(
            css.contains("--b: var(--a);"),
            "b should be included: {css}"
        );
    }

    #[test]
    fn resolve_trusts_self_referencing_cycle() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("a".into(), "var(--a)".into())]),
            )]),
        };

        let protocol_tokens = vec!["a".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        assert_eq!(resolved.css["light"], "--a: var(--a);");
    }

    #[test]
    fn resolve_skips_missing_protocol_token() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("surface-page".into(), "#fff".into())]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into(), "nonexistent".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        assert_eq!(resolved.css["light"], "--surface-page: #fff;");
    }

    #[test]
    fn resolve_trusts_missing_dependency() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("surface-page".into(), "var(--not-defined)".into())]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        assert_eq!(resolved.css["light"], "--surface-page: var(--not-defined);");
    }

    #[test]
    fn validate_required_tokens_accepts_complete_theme() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("surface-page".into(), "var(--neutral-100)".into()),
                    ("neutral-100".into(), "#fff".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        validate_required_tokens(&protocol_tokens, &token_file).unwrap();
    }

    #[test]
    fn validate_required_tokens_errors_on_missing_token() {
        let token_file = TokenFile {
            themes: HashMap::from([("light".into(), HashMap::new())]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        let result = validate_required_tokens(&protocol_tokens, &token_file);
        assert!(
            matches!(result, Err(TokenError::MissingToken { ref token, .. }) if token == "surface-page"),
            "expected missing token error, got: {result:?}"
        );
    }

    fn chain(tokens: &[&str], has_literal_fallback: bool) -> CssFallbackChain {
        CssFallbackChain {
            tokens: tokens.iter().map(|t| (*t).to_string()).collect(),
            has_literal_fallback,
        }
    }

    #[test]
    fn validate_chain_tokens_requires_all_non_literal_candidates() {
        // `var(--token-a, var(--token-b))` with no literal fallback requires both.
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("token-a".into(), "red".into())]),
            )]),
        };
        let chains = vec![chain(&["token-a", "token-b"], false)];

        let result = validate_chain_tokens(&chains, &token_file);
        assert!(
            matches!(result, Err(TokenError::MissingToken { ref token, .. }) if token == "token-b"),
            "expected token-b missing, got: {result:?}"
        );
    }

    #[test]
    fn validate_chain_tokens_exempts_literal_fallback_chain() {
        // `var(--brand, #000)` must pass even when no theme defines `--brand`.
        let token_file = TokenFile {
            themes: HashMap::from([("light".into(), HashMap::new())]),
        };
        let chains = vec![chain(&["brand"], true)];

        validate_chain_tokens(&chains, &token_file).unwrap();
    }

    #[test]
    fn validate_chain_tokens_requires_token_with_any_bare_usage() {
        // The same token used once with and once without a literal fallback is
        // required because of the bare usage.
        let token_file = TokenFile {
            themes: HashMap::from([("light".into(), HashMap::new())]),
        };
        let chains = vec![chain(&["brand"], true), chain(&["brand"], false)];

        let result = validate_chain_tokens(&chains, &token_file);
        assert!(
            matches!(result, Err(TokenError::MissingToken { ref token, .. }) if token == "brand"),
            "expected brand required, got: {result:?}"
        );
    }

    #[test]
    fn unthemed_literal_fallback_tokens_flags_only_literal_only_absent_tokens() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("present".into(), "2px".into()),
                    ("required".into(), "8px".into()),
                ]),
            )]),
        };
        let chains = vec![
            chain(&["colr-brand"], true), // literal-only, absent → flagged
            chain(&["present"], true),    // literal-only but themed → not flagged
            chain(&["required"], false),  // no literal fallback → not an advisory
        ];

        assert_eq!(
            unthemed_literal_fallback_tokens(&chains, &token_file),
            vec!["colr-brand".to_string()]
        );
    }

    #[test]
    fn unthemed_literal_fallback_tokens_does_not_flag_token_with_bare_usage() {
        let token_file = TokenFile {
            themes: HashMap::from([("light".into(), HashMap::new())]),
        };
        // Used both with and without a literal fallback → required, not advisory.
        let chains = vec![chain(&["brand"], true), chain(&["brand"], false)];

        assert!(unthemed_literal_fallback_tokens(&chains, &token_file).is_empty());
    }

    #[test]
    fn resolve_multiple_themes() {
        let token_file = TokenFile {
            themes: HashMap::from([
                (
                    "light".into(),
                    HashMap::from([("surface-page".into(), "#ffffff".into())]),
                ),
                (
                    "dark".into(),
                    HashMap::from([("surface-page".into(), "#171717".into())]),
                ),
            ]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        assert_eq!(resolved.css["light"], "--surface-page: #ffffff;");
        assert_eq!(resolved.css["dark"], "--surface-page: #171717;");
    }

    #[test]
    fn resolve_empty_protocol_tokens_produces_empty_css() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("surface-page".into(), "#fff".into())]),
            )]),
        };

        let resolved = resolve_tokens(&[], &token_file).unwrap();
        assert_eq!(resolved.css["light"], "");
    }

    #[test]
    fn resolve_complex_css_values() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("shadow-card".into(), "0 8px 32px rgba(0,0,0,0.3)".into()),
                    (
                        "accent-subtle".into(),
                        "color-mix(in oklch, var(--accent) 15%, transparent)".into(),
                    ),
                    ("accent".into(), "#0078d4".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["shadow-card".into(), "accent-subtle".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        let css = &resolved.css["light"];

        assert!(
            css.contains("--accent: #0078d4;"),
            "transitive dep 'accent' should be included: {css}"
        );
        assert!(css.contains("--shadow-card: 0 8px 32px rgba(0,0,0,0.3);"));
    }

    #[test]
    fn resolve_css_output_is_sorted() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("z-token".into(), "z".into()),
                    ("a-token".into(), "a".into()),
                    ("m-token".into(), "m".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["z-token".into(), "a-token".into(), "m-token".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        let css = &resolved.css["light"];

        let lines: Vec<&str> = css.lines().collect();
        assert_eq!(lines[0], "--a-token: a;");
        assert_eq!(lines[1], "--m-token: m;");
        assert_eq!(lines[2], "--z-token: z;");
    }

    // ── State injection tests ────────────────────────────────────────

    #[test]
    fn inject_into_empty_state() {
        let mut state = serde_json::json!({});
        let resolved = ResolvedTokens {
            css: HashMap::from([
                ("light".into(), "--a: #fff;".into()),
                ("dark".into(), "--a: #000;".into()),
            ]),
        };

        inject_into_state(&mut state, &resolved);

        assert_eq!(state["tokens"]["light"], "--a: #fff;");
        assert_eq!(state["tokens"]["dark"], "--a: #000;");
    }

    #[test]
    fn inject_preserves_existing_state() {
        let mut state = serde_json::json!({
            "title": "My App",
            "count": 42
        });
        let resolved = ResolvedTokens {
            css: HashMap::from([("light".into(), "--a: #fff;".into())]),
        };

        inject_into_state(&mut state, &resolved);

        assert_eq!(state["title"], "My App");
        assert_eq!(state["count"], 42);
        assert_eq!(state["tokens"]["light"], "--a: #fff;");
    }

    #[test]
    fn inject_merges_with_existing_tokens() {
        let mut state = serde_json::json!({
            "tokens": { "existing": "preserved" }
        });
        let resolved = ResolvedTokens {
            css: HashMap::from([("light".into(), "--a: #fff;".into())]),
        };

        inject_into_state(&mut state, &resolved);

        assert_eq!(state["tokens"]["existing"], "preserved");
        assert_eq!(state["tokens"]["light"], "--a: #fff;");
    }

    // ── Edge case: diamond dependency ────────────────────────────────

    #[test]
    fn resolve_diamond_dependency() {
        // a → b, a → c, b → d, c → d (diamond, not a cycle)
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([
                    ("a".into(), "var(--b) var(--c)".into()),
                    ("b".into(), "var(--d)".into()),
                    ("c".into(), "var(--d)".into()),
                    ("d".into(), "#000".into()),
                ]),
            )]),
        };

        let protocol_tokens = vec!["a".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();
        let css = &resolved.css["light"];

        assert!(css.contains("--d: #000;"), "d should be included: {css}");
        assert!(css.contains("--b:"), "b should be included: {css}");
        assert!(css.contains("--c:"), "c should be included: {css}");
    }
}
