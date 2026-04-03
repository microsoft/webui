// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Design token loading, filtering, and CSS generation for WebUI.
//!
//! Loads token values from JSON files, filters them against the protocol's
//! required token list, resolves transitive `var(--x)` dependencies, detects
//! cycles, and generates per-theme CSS declaration strings ready for state
//! injection.
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

pub use error::{Result, TokenError, TokenWarning};

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Loaded token themes — maps theme name → (token name → CSS value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenFile {
    /// Theme name → token map. Token names are without the `--` prefix.
    pub themes: HashMap<String, HashMap<String, String>>,
}

/// Result of resolving tokens: per-theme CSS strings and any warnings.
#[derive(Debug, Clone)]
pub struct ResolvedTokens {
    /// Theme name → CSS declarations string (e.g. `"--surface-page: #fff;\n--text-primary: #111;"`)
    pub css: HashMap<String, String>,
    /// Non-fatal warnings encountered during resolution.
    pub warnings: Vec<TokenWarning>,
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

// ── Resolution ───────────────────────────────────────────────────────

/// Resolve tokens for all themes against the protocol's required token list.
///
/// For each theme:
/// 1. Filters to keep only tokens in `protocol_tokens`
/// 2. Expands transitive `var(--x)` dependencies
/// 3. Detects cyclic references
/// 4. Generates CSS declaration strings
///
/// # Errors
///
/// Returns [`TokenError::CyclicDependency`] if a dependency cycle is found.
pub fn resolve_tokens(
    protocol_tokens: &[String],
    token_file: &TokenFile,
) -> Result<ResolvedTokens> {
    let required: HashSet<&str> = protocol_tokens.iter().map(String::as_str).collect();
    let mut css = HashMap::with_capacity(token_file.themes.len());
    let mut warnings = Vec::new();

    for (theme_name, theme_tokens) in &token_file.themes {
        let (css_string, theme_warnings) = resolve_theme(&required, theme_tokens, theme_name)?;
        css.insert(theme_name.clone(), css_string);
        warnings.extend(theme_warnings);
    }

    Ok(ResolvedTokens { css, warnings })
}

/// Resolve a single theme: filter, expand deps, generate CSS.
fn resolve_theme(
    required: &HashSet<&str>,
    theme_tokens: &HashMap<String, String>,
    theme_name: &str,
) -> Result<(String, Vec<TokenWarning>)> {
    let mut warnings = Vec::new();

    // Step 1: Compute the closure of required tokens (including transitive deps)
    let closure = compute_token_closure(required, theme_tokens, theme_name, &mut warnings)?;

    // Step 2: Warn about required tokens missing from the theme
    for name in required {
        if !theme_tokens.contains_key(*name) {
            warnings.push(TokenWarning::MissingToken {
                theme: theme_name.into(),
                token: (*name).into(),
            });
        }
    }

    // Step 3: Generate sorted CSS declarations
    let css = generate_css_declarations(&closure, theme_tokens);

    Ok((css, warnings))
}

/// Compute the transitive closure of required tokens by following `var(--x)`
/// references in token values. Uses iterative expansion with cycle detection.
fn compute_token_closure(
    required: &HashSet<&str>,
    theme_tokens: &HashMap<String, String>,
    theme_name: &str,
    warnings: &mut Vec<TokenWarning>,
) -> Result<Vec<String>> {
    let mut closure: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = required
        .iter()
        .filter(|name| theme_tokens.contains_key(**name))
        .map(|s| (*s).into())
        .collect();

    // Track dependency graph edges for cycle detection
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();

    while let Some(name) = queue.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }

        if let Some(value) = theme_tokens.get(&name) {
            let deps = extract_var_references(value);
            for dep in deps {
                if !theme_tokens.contains_key(&dep) {
                    warnings.push(TokenWarning::MissingDependency {
                        theme: theme_name.into(),
                        token: name.clone(),
                        dependency: dep,
                    });
                    continue;
                }
                edges.entry(name.clone()).or_default().push(dep.clone());
                if !closure.contains(&dep) {
                    queue.push(dep);
                }
            }
        }
    }

    // Detect cycles using DFS on the dependency graph
    detect_cycles(&edges, &closure)?;

    let mut sorted: Vec<String> = closure.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

/// Detect cycles in the dependency graph using iterative DFS with explicit
/// coloring (White → Gray → Black).
fn detect_cycles(edges: &HashMap<String, Vec<String>>, all_nodes: &HashSet<String>) -> Result<()> {
    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;

    let mut color: HashMap<&str, u8> = all_nodes.iter().map(|n| (n.as_str(), WHITE)).collect();
    // parent map for cycle path reconstruction
    let mut parent: HashMap<&str, &str> = HashMap::new();

    for start in all_nodes {
        if color[start.as_str()] != WHITE {
            continue;
        }

        let mut stack = vec![start.as_str()];

        while let Some(node) = stack.last().copied() {
            match color[node] {
                WHITE => {
                    color.insert(node, GRAY);
                    if let Some(deps) = edges.get(node) {
                        for dep in deps {
                            match color.get(dep.as_str()).copied().unwrap_or(WHITE) {
                                GRAY => {
                                    // Cycle found — reconstruct
                                    let mut chain = vec![dep.as_str(), node];
                                    let mut cur = node;
                                    while let Some(&p) = parent.get(cur) {
                                        if p == dep.as_str() {
                                            break;
                                        }
                                        chain.push(p);
                                        cur = p;
                                    }
                                    chain.reverse();
                                    return Err(TokenError::cycle(&chain));
                                }
                                WHITE => {
                                    parent.insert(dep.as_str(), node);
                                    stack.push(dep.as_str());
                                }
                                _ => {} // BLACK — already processed
                            }
                        }
                    }
                }
                GRAY => {
                    color.insert(node, BLACK);
                    stack.pop();
                }
                _ => {
                    stack.pop();
                }
            }
        }
    }

    Ok(())
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
pub fn inject_into_state(state: &mut Value, resolved: &ResolvedTokens) {
    let tokens_obj = state
        .as_object_mut()
        .expect("state must be a JSON object")
        .entry("tokens")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Some(map) = tokens_obj.as_object_mut() {
        for (theme_name, css_string) in &resolved.css {
            map.insert(theme_name.clone(), Value::String(css_string.clone()));
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
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
    fn resolve_detects_direct_cycle() {
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
        let result = resolve_tokens(&protocol_tokens, &token_file);
        assert!(
            matches!(result, Err(TokenError::CyclicDependency(ref msg)) if msg.contains("--a") && msg.contains("--b")),
            "expected cycle error, got: {result:?}"
        );
    }

    #[test]
    fn resolve_detects_self_referencing_cycle() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("a".into(), "var(--a)".into())]),
            )]),
        };

        let protocol_tokens = vec!["a".into()];
        let result = resolve_tokens(&protocol_tokens, &token_file);
        assert!(
            matches!(result, Err(TokenError::CyclicDependency(_))),
            "expected cycle error, got: {result:?}"
        );
    }

    #[test]
    fn resolve_warns_on_missing_protocol_token() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("surface-page".into(), "#fff".into())]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into(), "nonexistent".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        assert!(
            resolved.warnings.iter().any(|w| matches!(
                w,
                TokenWarning::MissingToken { token, .. } if token == "nonexistent"
            )),
            "should warn about missing token: {:?}",
            resolved.warnings
        );
    }

    #[test]
    fn resolve_warns_on_missing_dependency() {
        let token_file = TokenFile {
            themes: HashMap::from([(
                "light".into(),
                HashMap::from([("surface-page".into(), "var(--not-defined)".into())]),
            )]),
        };

        let protocol_tokens = vec!["surface-page".into()];
        let resolved = resolve_tokens(&protocol_tokens, &token_file).unwrap();

        assert!(
            resolved.warnings.iter().any(|w| matches!(
                w,
                TokenWarning::MissingDependency { dependency, .. } if dependency == "not-defined"
            )),
            "should warn about missing dep: {:?}",
            resolved.warnings
        );
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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

    // ── Warning Display ──────────────────────────────────────────────

    #[test]
    fn warning_display_missing_token() {
        let w = TokenWarning::MissingToken {
            theme: "light".into(),
            token: "color-brand".into(),
        };
        let msg = w.to_string();
        assert!(msg.contains("--color-brand"));
        assert!(msg.contains("light"));
    }

    #[test]
    fn warning_display_missing_dependency() {
        let w = TokenWarning::MissingDependency {
            theme: "dark".into(),
            token: "surface".into(),
            dependency: "neutral-100".into(),
        };
        let msg = w.to_string();
        assert!(msg.contains("--surface"));
        assert!(msg.contains("--neutral-100"));
    }
}
