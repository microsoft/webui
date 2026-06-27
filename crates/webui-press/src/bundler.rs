// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! JavaScript bundling, local component script discovery, and script injection.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use crate::error::{Error, Result};
use crate::types::BundlerConfig;

static BUNDLE_REBUILD_NONCE: AtomicU64 = AtomicU64::new(0);

/// Resolve a configured component source for the per-page builds.
///
/// Local paths are made absolute against `cwd` (the project root) because
/// `webui-discovery` resolves relative paths against the synthesized per-page
/// app directory, not the project. npm package names and scopes (e.g.
/// `@mai-ui`) are left bare so discovery resolves them from `node_modules`.
pub(crate) fn resolve_config_component_source(source: &str, cwd: &Path) -> String {
    if webui_discovery::is_local_source(source) {
        cwd.join(source).to_string_lossy().to_string()
    } else {
        source.to_string()
    }
}

const BUNDLE_SCRIPT_EXTENSIONS: &[&str] = &["js", "mjs", "jsx", "ts", "tsx"];
const SCRIPT_GRAPH_EXTENSIONS: &[&str] = &["js", "mjs", "jsx", "ts", "tsx"];

/// A page-level esbuild entry generated from local component scripts and
/// explicit `<script bundle>` sources.
#[derive(Debug, Clone)]
pub(crate) struct PageBundleEntry {
    /// Unique numeric ID used in the generated output filename.
    pub(crate) id: usize,
    /// The page path this bundle belongs to (retained for diagnostics).
    pub(crate) page_path: String,
    /// Local component scripts used by this page.
    pub(crate) component_scripts: Vec<PathBuf>,
    /// Explicit scripts from `<script bundle>` tags or custom page `scriptFile`.
    pub(crate) explicit_scripts: Vec<ScriptSource>,
}

/// Root esbuild entry for template chrome scripts shared by every page.
#[derive(Debug, Clone)]
pub(crate) struct RootBundleEntry {
    /// Template-level script such as `template/index.ts`.
    pub(crate) script_path: Option<PathBuf>,
    /// Local component scripts used by the template chrome.
    pub(crate) component_scripts: Vec<PathBuf>,
}

/// Local component script discovered from a component HTML file and optional
/// sibling TypeScript file.
#[derive(Debug, Clone)]
pub(crate) struct ComponentScript {
    html_content: String,
    script_path: PathBuf,
}

pub(crate) struct BundleThread {
    handle: Option<std::thread::JoinHandle<Result<BundleResult>>>,
}

impl BundleThread {
    pub(crate) fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() -> Result<BundleResult> + Send + 'static,
    {
        Self {
            handle: Some(std::thread::spawn(f)),
        }
    }

    pub(crate) fn join(mut self) -> Result<BundleResult> {
        let Some(handle) = self.handle.take() else {
            return Err(Error::Build("Bundle thread already joined".to_string()));
        };
        handle
            .join()
            .map_err(|_| Error::Build("Bundle thread panicked".to_string()))?
    }
}

impl Drop for BundleThread {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

/// Source of a bundleable script.
#[derive(Debug, Clone)]
pub(crate) enum ScriptSource {
    /// Inline script content to bundle.
    Inline(String),
    /// A `src` attribute path (resolved relative to the config dir).
    File(String),
}

fn is_component_html_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "html")
        && path
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains('-'))
}

/// Discover local component scripts from component sources.
///
/// A script is considered the client entry for a component when a
/// `tag-name.html` file has a sibling `tag-name.ts` file. npm component
/// sources are skipped here because package custom-element imports are
/// explicit page script responsibility.
pub(crate) fn discover_component_scripts(
    component_sources: &[String],
) -> Result<BTreeMap<String, ComponentScript>> {
    let mut scripts = BTreeMap::new();
    for source in component_sources {
        let root = Path::new(source);
        if !root.is_dir() {
            continue;
        }
        collect_component_scripts(root, &mut scripts)?;
    }
    Ok(scripts)
}

/// Collect component script pairs from a directory tree (iterative).
fn collect_component_scripts(
    dir: &Path,
    scripts: &mut BTreeMap<String, ComponentScript>,
) -> Result<()> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d)
            .map_err(|e| Error::Io(format!("Cannot read component dir {}: {e}", d.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_component_html_file(&path) {
                let Some(tag) = path.file_stem().and_then(|name| name.to_str()) else {
                    continue;
                };
                let script_path = path.with_extension("ts");
                if script_path.exists() {
                    let script_path = script_path.canonicalize().unwrap_or(script_path);
                    let html_content = fs::read_to_string(&path).map_err(|e| {
                        Error::Io(format!(
                            "Cannot read component template {}: {e}",
                            path.display()
                        ))
                    })?;
                    scripts.entry(tag.to_string()).or_insert(ComponentScript {
                        html_content,
                        script_path,
                    });
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn collect_component_scripts_for_html(
    html_fragments: &[&str],
    component_scripts: &BTreeMap<String, ComponentScript>,
) -> Result<Vec<PathBuf>> {
    if component_scripts.is_empty() {
        return Ok(Vec::new());
    }

    let mut pending = Vec::new();
    for html in html_fragments {
        push_custom_element_tags(html, component_scripts, &mut pending);
    }

    let mut seen_tags = HashSet::with_capacity(pending.len());
    let mut script_paths = Vec::new();
    let mut cursor = 0;
    while cursor < pending.len() {
        let tag = pending[cursor].clone();
        cursor += 1;
        if !seen_tags.insert(tag.clone()) {
            continue;
        }
        let Some(component) = component_scripts.get(&tag) else {
            continue;
        };
        script_paths.push(component.script_path.clone());
        push_custom_element_tags(&component.html_content, component_scripts, &mut pending);
    }

    Ok(script_paths)
}

pub(crate) fn page_bundle_signature(
    component_scripts: &[PathBuf],
    explicit_scripts: &[ScriptSource],
    config_dir: &Path,
) -> String {
    let mut signature =
        String::with_capacity(component_scripts.len() * 96 + explicit_scripts.len() * 96);
    let mut component_paths: Vec<String> = component_scripts
        .iter()
        .map(|path| path_for_js(path))
        .collect();
    component_paths.sort();
    component_paths.dedup();
    for path in component_paths {
        signature.push_str("component:");
        signature.push_str(&path);
        signature.push('\n');
    }
    for source in explicit_scripts {
        match source {
            ScriptSource::Inline(code) => {
                signature.push_str("inline:");
                signature.push_str(&format!("{:x}", fxhash_bytes(code.as_bytes())));
                signature.push('\n');
            }
            ScriptSource::File(path) => {
                let resolved = config_dir.join(path);
                let resolved = resolved.canonicalize().unwrap_or(resolved);
                signature.push_str("file:");
                signature.push_str(&path_for_js(&resolved));
                signature.push('\n');
            }
        }
    }
    signature
}

fn push_custom_element_tags(
    html: &str,
    component_scripts: &BTreeMap<String, ComponentScript>,
    tags: &mut Vec<String>,
) {
    let bytes = html.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(rel) = html[cursor..].find('<') else {
            break;
        };
        let tag_start = cursor + rel;
        let name_start = tag_start + 1;
        let Some(&first) = bytes.get(name_start) else {
            break;
        };

        if first == b'!' {
            if bytes.get(name_start + 1) == Some(&b'-') && bytes.get(name_start + 2) == Some(&b'-')
            {
                if let Some(end_rel) = html[name_start + 3..].find("-->") {
                    cursor = name_start + 3 + end_rel + 3;
                    continue;
                }
                break;
            }
            cursor = name_start + 1;
            continue;
        }

        if matches!(first, b'/' | b'?') {
            cursor = name_start + 1;
            continue;
        }

        let mut name_end = name_start;
        let mut has_hyphen = false;
        while name_end < bytes.len() && is_tag_name_byte(bytes[name_end]) {
            if bytes[name_end] == b'-' {
                has_hyphen = true;
            }
            name_end += 1;
        }

        if name_end == name_start {
            cursor = name_start + 1;
            continue;
        }

        let tag = &html[name_start..name_end];
        if tag == "script" || tag == "style" {
            let close = if tag == "script" {
                "</script>"
            } else {
                "</style>"
            };
            if let Some(end_rel) = html[name_end..].find(close) {
                cursor = name_end + end_rel + close.len();
                continue;
            }
            break;
        }

        if has_hyphen && component_scripts.contains_key(tag) {
            tags.push(tag.to_string());
        }

        cursor = name_end;
    }
}

fn is_tag_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Bundle result returned by [`bundle_assets`].
pub(crate) struct BundleResult {
    /// Root script shared by every page, when the template has browser code.
    pub(crate) root_script: Option<String>,
    /// Number of local component scripts imported by page entries.
    pub(crate) component_count: usize,
    /// Number of page-specific import groups bundled.
    pub(crate) page_entry_count: usize,
    /// Map from page-bundle ID to relative output paths.
    ///
    /// Import-only esbuild entry wrappers are flattened to the chunks they
    /// import so pages do not pay a request for a `page-N.js` file that only
    /// forwards to shared chunks.
    pub(crate) script_map: HashMap<usize, Vec<String>>,
}

/// Configuration for the [`bundle_assets`] function.
pub(crate) struct BundleOptions<'a> {
    pub(crate) site_dir: &'a Path,
    pub(crate) node_modules: Option<&'a Path>,
    pub(crate) root_bundle: Option<&'a RootBundleEntry>,
    pub(crate) page_bundles: &'a [PageBundleEntry],
    pub(crate) bundler_config: Option<&'a BundlerConfig>,
    pub(crate) dev_mode: bool,
    pub(crate) config_dir: &'a Path,
    pub(crate) content_dir: &'a Path,
}

fn path_for_js(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn push_import_once(entry: &mut String, imports: &mut HashSet<String>, specifier: &str) {
    if imports.insert(specifier.to_string()) {
        entry.push_str("import \"");
        entry.push_str(specifier);
        entry.push_str("\";\n");
    }
}

fn push_external_args(
    args: &mut Vec<String>,
    external: &[String],
    aliases: &BTreeMap<String, String>,
) {
    for ext in external {
        if aliases.contains_key(ext.as_str()) {
            continue;
        }
        args.push(format!("--external:{ext}"));
    }
}

fn file_version(path: &Path) -> Result<String> {
    let bytes =
        fs::read(path).map_err(|e| Error::Io(format!("Cannot read {}: {e}", path.display())))?;
    Ok(format!("{:x}", fxhash_bytes(&bytes)))
}

fn versioned_asset_path(rel_path: &str, full_path: &Path) -> Result<String> {
    let version = file_version(full_path)?;
    Ok(format!("{rel_path}?v={version}"))
}

fn has_allowed_extension(path: &Path, allowed: &[&str]) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    allowed
        .iter()
        .any(|allowed_ext| ext.eq_ignore_ascii_case(allowed_ext))
}

fn resolve_existing_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|e| {
        Error::Build(format!(
            "Cannot resolve script path {}: {e}",
            path.display()
        ))
    })
}

fn resolve_allowed_root(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(path.canonicalize().map_err(|e| {
        Error::Build(format!(
            "Cannot canonicalize allowed script root {}: {e}",
            path.display()
        ))
    })?))
}

fn allowed_script_roots(config_dir: &Path, content_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::with_capacity(2);
    if let Some(root) = resolve_allowed_root(config_dir)? {
        roots.push(root);
    }
    if let Some(root) = resolve_allowed_root(content_dir)? {
        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn resolve_bundle_script_file(
    config_dir: &Path,
    content_dir: &Path,
    path: &str,
) -> Result<PathBuf> {
    let requested = Path::new(path);
    if requested.is_absolute() {
        return Err(Error::Build(format!(
            "Bundled script path {path} must be relative to config.json"
        )));
    }
    if !has_allowed_extension(requested, BUNDLE_SCRIPT_EXTENSIONS) {
        return Err(Error::Build(format!(
            "Bundled script path {path} must use one of these extensions: {}",
            BUNDLE_SCRIPT_EXTENSIONS.join(", ")
        )));
    }

    let resolved = resolve_existing_path(&config_dir.join(requested))?;
    let roots = allowed_script_roots(config_dir, content_dir)?;
    if roots.iter().any(|root| path_is_under_root(&resolved, root)) {
        Ok(resolved)
    } else {
        Err(Error::Build(format!(
            "Bundled script path {} resolves outside the docs project",
            resolved.display()
        )))
    }
}

fn package_import_prefix_len(specifier: &str) -> usize {
    if specifier.starts_with('@') {
        let mut slash_count = 0;
        for (idx, byte) in specifier.bytes().enumerate() {
            if byte == b'/' {
                slash_count += 1;
                if slash_count == 2 {
                    return idx;
                }
            }
        }
        return specifier.len();
    }
    specifier.find('/').unwrap_or(specifier.len())
}

fn is_external_or_alias_import(specifier: &str, cfg: Option<&BundlerConfig>) -> bool {
    let Some(cfg) = cfg else {
        return false;
    };
    if cfg.alias.contains_key(specifier)
        || cfg.external.iter().any(|external| external == specifier)
    {
        return true;
    }
    let prefix_len = package_import_prefix_len(specifier);
    let prefix = &specifier[..prefix_len];
    cfg.alias.contains_key(prefix) || cfg.external.iter().any(|external| external == prefix)
}

fn validate_script_graph_imports(
    script_path: &Path,
    allowed_roots: &[PathBuf],
    bundler_config: Option<&BundlerConfig>,
) -> Result<()> {
    let mut pending = vec![script_path.to_path_buf()];
    let mut seen = HashSet::with_capacity(8);
    while let Some(path) = pending.pop() {
        let resolved = resolve_existing_path(&path)?;
        if !seen.insert(resolved.clone())
            || !has_allowed_extension(&resolved, SCRIPT_GRAPH_EXTENSIONS)
        {
            continue;
        }
        let contents = fs::read_to_string(&resolved)
            .map_err(|e| Error::Io(format!("Cannot read {}: {e}", resolved.display())))?;
        validate_script_imports(
            &contents,
            &resolved,
            allowed_roots,
            bundler_config,
            &mut pending,
        )?;
    }
    Ok(())
}

fn validate_script_imports(
    contents: &str,
    owner: &Path,
    allowed_roots: &[PathBuf],
    bundler_config: Option<&BundlerConfig>,
    pending: &mut Vec<PathBuf>,
) -> Result<()> {
    for specifier in import_specifiers(contents) {
        if specifier.starts_with('.') {
            let Some(parent) = owner.parent() else {
                continue;
            };
            let resolved = resolve_existing_path(&parent.join(specifier))?;
            if !allowed_roots
                .iter()
                .any(|root| path_is_under_root(&resolved, root))
            {
                return Err(Error::Build(format!(
                    "Bundled script import {specifier} in {} resolves outside the docs project",
                    owner.display()
                )));
            }
            pending.push(resolved);
        } else if Path::new(specifier).is_absolute()
            && !is_external_or_alias_import(specifier, bundler_config)
        {
            return Err(Error::Build(format!(
                "Bundled script import {specifier} in {} must not be an absolute filesystem path",
                owner.display()
            )));
        }
    }
    Ok(())
}

fn validate_inline_script_imports(
    code: &str,
    config_dir: &Path,
    allowed_roots: &[PathBuf],
    bundler_config: Option<&BundlerConfig>,
) -> Result<()> {
    let mut pending = Vec::new();
    validate_script_imports(
        code,
        &config_dir.join("<inline bundle script>"),
        allowed_roots,
        bundler_config,
        &mut pending,
    )?;
    let mut seen = HashSet::with_capacity(pending.len());
    while let Some(path) = pending.pop() {
        let resolved = resolve_existing_path(&path)?;
        if !seen.insert(resolved.clone())
            || !has_allowed_extension(&resolved, SCRIPT_GRAPH_EXTENSIONS)
        {
            continue;
        }
        let contents = fs::read_to_string(&resolved)
            .map_err(|e| Error::Io(format!("Cannot read {}: {e}", resolved.display())))?;
        validate_script_imports(
            &contents,
            &resolved,
            allowed_roots,
            bundler_config,
            &mut pending,
        )?;
    }
    Ok(())
}

fn versioned_script_paths(site_dir: &Path, rel_paths: &[String]) -> Result<Vec<String>> {
    let mut versioned = Vec::with_capacity(rel_paths.len());
    for rel_path in rel_paths {
        let full_path = site_dir.join(rel_path);
        if !full_path.exists() {
            return Err(Error::Build(format!(
                "Bundled chunk output missing: {}",
                full_path.display()
            )));
        }
        versioned.push(versioned_asset_path(rel_path, &full_path)?);
    }
    Ok(versioned)
}

fn skip_js_whitespace(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn skip_js_trivia(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    loop {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b';')
        {
            cursor += 1;
        }

        if cursor + 1 >= bytes.len() || bytes[cursor] != b'/' {
            return cursor;
        }

        if bytes[cursor + 1] == b'/' {
            cursor += 2;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }

        if bytes[cursor + 1] == b'*' {
            if let Some(end_rel) = input[cursor + 2..].find("*/") {
                cursor += end_rel + 4;
                continue;
            }
            return input.len();
        }

        return cursor;
    }
}

fn quoted_js_string_at(input: &str, cursor: usize) -> Option<(&str, usize)> {
    let bytes = input.as_bytes();
    let quote = *bytes.get(cursor)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }

    let start = cursor + 1;
    let mut end = start;
    while end < bytes.len() {
        if bytes[end] == b'\\' {
            end += 2;
            continue;
        }
        if bytes[end] == quote {
            return Some((&input[start..end], end + 1));
        }
        end += 1;
    }
    None
}

fn next_leading_import_statement<'a>(input: &'a str, cursor: &mut usize) -> Option<&'a str> {
    let start = skip_js_trivia(input, *cursor);
    if !is_import_keyword_at(input, start) {
        *cursor = start;
        return None;
    }

    let after_import = start + "import".len();
    if let Some(byte) = input.as_bytes().get(after_import) {
        if byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$' {
            *cursor = start;
            return None;
        }
    }

    let end_rel = input[after_import..].find(';')?;
    let end = after_import + end_rel;
    *cursor = end + 1;
    Some(&input[start..=end])
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn is_import_keyword_at(input: &str, cursor: usize) -> bool {
    if !input[cursor..].starts_with("import") {
        return false;
    }
    let bytes = input.as_bytes();
    if cursor > 0 && is_identifier_byte(bytes[cursor - 1]) {
        return false;
    }
    let after = cursor + "import".len();
    !bytes
        .get(after)
        .is_some_and(|byte| is_identifier_byte(*byte))
}

fn skip_quoted_js(input: &str, mut cursor: usize, quote: u8) -> usize {
    let bytes = input.as_bytes();
    cursor += 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor += 2;
        } else if bytes[cursor] == quote {
            return cursor + 1;
        } else {
            cursor += 1;
        }
    }
    cursor
}

fn skip_line_comment(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    cursor += 2;
    while cursor < bytes.len() && bytes[cursor] != b'\n' {
        cursor += 1;
    }
    cursor
}

fn skip_block_comment(input: &str, cursor: usize) -> usize {
    if let Some(end_rel) = input[cursor + 2..].find("*/") {
        cursor + end_rel + 4
    } else {
        input.len()
    }
}

fn skip_js_non_code(input: &str, cursor: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    match bytes[cursor] {
        b'\'' | b'"' | b'`' => Some(skip_quoted_js(input, cursor, bytes[cursor])),
        b'/' if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'/' => {
            Some(skip_line_comment(input, cursor))
        }
        b'/' if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'*' => {
            Some(skip_block_comment(input, cursor))
        }
        _ => None,
    }
}

fn js_statement_end(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    while cursor < bytes.len() {
        if let Some(next) = skip_js_non_code(input, cursor) {
            cursor = next;
        } else if bytes[cursor] == b';' {
            return cursor + 1;
        } else {
            cursor += 1;
        }
    }
    cursor
}

fn import_specifiers(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut specifiers = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if let Some(next) = skip_js_non_code(input, cursor) {
            cursor = next;
            continue;
        }

        if !is_import_keyword_at(input, cursor) {
            cursor += 1;
            continue;
        }

        let mut after_import = skip_js_whitespace(input, cursor + "import".len());
        if let Some((specifier, end)) = quoted_js_string_at(input, after_import) {
            specifiers.push(specifier);
            cursor = end;
            continue;
        }

        if bytes.get(after_import) == Some(&b'(') {
            after_import = skip_js_whitespace(input, after_import + 1);
            if let Some((specifier, end)) = quoted_js_string_at(input, after_import) {
                specifiers.push(specifier);
                cursor = end;
                continue;
            }
        }

        let statement_end = js_statement_end(input, after_import);
        if let Some(specifier) = static_import_specifier(&input[cursor..statement_end]) {
            specifiers.push(specifier);
        }
        cursor = statement_end.max(cursor + "import".len());
    }
    specifiers
}

fn side_effect_import_specifier(statement: &str) -> Option<&str> {
    let cursor = skip_js_whitespace(statement, "import".len());
    quoted_js_string_at(statement, cursor).map(|(specifier, _)| specifier)
}

fn static_import_specifier(statement: &str) -> Option<&str> {
    if let Some(specifier) = side_effect_import_specifier(statement) {
        return Some(specifier);
    }

    let from_pos = statement.rfind("from")?;
    let cursor = skip_js_whitespace(statement, from_pos + "from".len());
    quoted_js_string_at(statement, cursor).map(|(specifier, _)| specifier)
}

fn resolve_js_import_path(importer_rel: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }

    let parent = importer_rel
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let mut parts = Vec::new();
    for part in parent.split('/') {
        if !part.is_empty() {
            parts.push(part);
        }
    }
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn collect_leading_relative_imports(
    input: &str,
    importer_rel: &str,
    imports: &mut HashSet<String>,
) {
    let mut cursor = 0;
    while let Some(statement) = next_leading_import_statement(input, &mut cursor) {
        if let Some(specifier) = static_import_specifier(statement) {
            if let Some(rel_path) = resolve_js_import_path(importer_rel, specifier) {
                imports.insert(rel_path);
            }
        }
    }
}

fn import_only_relative_imports(input: &str, importer_rel: &str) -> Option<Vec<String>> {
    let mut cursor = 0;
    let mut imports = Vec::new();
    loop {
        cursor = skip_js_trivia(input, cursor);
        if cursor >= input.len() {
            return if imports.is_empty() {
                None
            } else {
                Some(imports)
            };
        }

        let statement = next_leading_import_statement(input, &mut cursor)?;
        let specifier = side_effect_import_specifier(statement)?;
        let rel_path = resolve_js_import_path(importer_rel, specifier)?;
        imports.push(rel_path);
    }
}

fn prune_redundant_imports(
    site_dir: &Path,
    root_imports: &HashSet<String>,
    imports: &[String],
) -> Result<Vec<String>> {
    let mut transitive_imports = HashSet::new();
    for rel_path in imports {
        let full_path = site_dir.join(rel_path);
        if !full_path.exists() {
            return Err(Error::Build(format!(
                "Bundled chunk output missing: {}",
                full_path.display()
            )));
        }
        let contents = fs::read_to_string(&full_path)
            .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
        collect_leading_relative_imports(&contents, rel_path, &mut transitive_imports);
    }

    let mut seen = HashSet::with_capacity(imports.len());
    let mut retained = Vec::with_capacity(imports.len());
    for rel_path in imports {
        if root_imports.contains(rel_path) || transitive_imports.contains(rel_path) {
            continue;
        }
        if seen.insert(rel_path.clone()) {
            retained.push(rel_path.clone());
        }
    }
    Ok(retained)
}

fn page_script_paths(
    site_dir: &Path,
    output_file: &str,
    full_path: &Path,
    root_imports: &HashSet<String>,
) -> Result<Vec<String>> {
    let contents = fs::read_to_string(full_path)
        .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
    if let Some(imports) = import_only_relative_imports(&contents, output_file) {
        let rel_paths = prune_redundant_imports(site_dir, root_imports, &imports)?;
        fs::remove_file(full_path).map_err(|e| {
            Error::Io(format!(
                "Cannot remove import-only script wrapper {}: {e}",
                full_path.display()
            ))
        })?;
        versioned_script_paths(site_dir, &rel_paths)
    } else {
        Ok(vec![versioned_asset_path(output_file, full_path)?])
    }
}

fn next_rebuild_nonce_hex() -> String {
    format!(
        "{:x}",
        BUNDLE_REBUILD_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub(crate) fn resolve_node_modules(config_dir: &Path) -> Result<PathBuf> {
    let start = config_dir.canonicalize().map_err(|e| {
        Error::Build(format!(
            "Cannot resolve config directory {} while locating node_modules: {e}",
            config_dir.display()
        ))
    })?;

    for dir in start.ancestors() {
        let node_modules = dir.join("node_modules");
        if node_modules.exists() {
            return Ok(node_modules);
        }
    }

    Err(Error::Build(format!(
        "Cannot find node_modules in {} or its ancestors. Run pnpm install from the docs project root before building bundled scripts.",
        start.display()
    )))
}

fn default_framework_alias(node_modules: &Path) -> Option<PathBuf> {
    let pkg = node_modules.join("@microsoft").join("webui-framework");
    let dist = pkg.join("dist").join("index.js");
    if dist.exists() {
        return Some(dist);
    }

    let src = pkg.join("src").join("index.ts");
    if src.exists() {
        return Some(src);
    }

    None
}

fn normalized_alias_target(config_dir: &Path, target: &str) -> String {
    let path = Path::new(target);
    if path.is_absolute() {
        return target.replace('\\', "/");
    }

    if target.starts_with('.') {
        return path_for_js(&config_dir.join(path));
    }

    target.replace('\\', "/")
}

fn build_aliases(opts: &BundleOptions<'_>) -> BTreeMap<String, String> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    if let Some(node_modules) = opts.node_modules {
        if let Some(path) = default_framework_alias(node_modules) {
            aliases.insert("@microsoft/webui-framework".to_string(), path_for_js(&path));
        }
    }

    if let Some(cfg) = opts.bundler_config {
        for (from, to) in &cfg.alias {
            aliases.insert(from.clone(), normalized_alias_target(opts.config_dir, to));
        }
    }

    aliases
}

fn push_alias_args(args: &mut Vec<String>, aliases: &BTreeMap<String, String>) {
    for (from, to) in aliases {
        args.push(format!("--alias:{from}={to}"));
    }
}

fn push_define_args(args: &mut Vec<String>, cfg: &BundlerConfig) {
    for (key, value) in &cfg.define {
        args.push(format!("--define:{key}={value}"));
    }
}

fn esbuild_args(
    opts: &BundleOptions<'_>,
    entry_files: &[(String, PathBuf)],
    bundle_tmp: &Path,
) -> Vec<String> {
    let aliases = build_aliases(opts);
    let target = opts
        .bundler_config
        .and_then(|cfg| cfg.target.as_deref())
        .unwrap_or("es2022");
    let mut args = Vec::with_capacity(14 + entry_files.len());
    args.push("--bundle".to_string());
    args.push("--platform=browser".to_string());
    args.push("--format=esm".to_string());
    args.push("--splitting".to_string());
    args.push(format!("--target={target}"));
    args.push(format!("--outdir={}", path_for_js(opts.site_dir)));
    args.push(format!("--outbase={}", path_for_js(bundle_tmp)));
    args.push("--entry-names=[dir]/[name]".to_string());
    args.push("--chunk-names=assets/[name]-[hash]".to_string());
    args.push("--loader:.html=text".to_string());
    args.push("--loader:.css=text".to_string());
    args.push("--log-level=warning".to_string());
    if !opts.dev_mode {
        args.push("--minify".to_string());
    }
    if let Some(cfg) = opts.bundler_config {
        push_external_args(&mut args, &cfg.external, &aliases);
        push_define_args(&mut args, cfg);
    }
    push_alias_args(&mut args, &aliases);
    for (_, path) in entry_files {
        args.push(path_for_js(path));
    }
    args
}

/// Bundle page-scoped scripts via esbuild.
///
/// Uses a single esbuild invocation with one virtual entry per page for
/// optimal code splitting. Each page entry imports local component scripts
/// discovered from that page's HTML plus any explicit `<script bundle>` or
/// `scriptFile` sources.
///
/// Returns a [`BundleResult`] with the component script count and a mapping
/// from page-bundle IDs to their output file paths.
pub(crate) fn bundle_assets(opts: &BundleOptions<'_>) -> Result<BundleResult> {
    if opts.root_bundle.is_none() && opts.page_bundles.is_empty() {
        return Ok(BundleResult {
            root_script: None,
            component_count: 0,
            page_entry_count: 0,
            script_map: HashMap::new(),
        });
    }
    let Some(node_modules) = opts.node_modules else {
        return Err(Error::Build(
            "Bundled scripts require node_modules. Run pnpm install from the docs project root."
                .to_string(),
        ));
    };
    let allowed_roots = allowed_script_roots(opts.config_dir, opts.content_dir)?;

    // Create a temp directory for the bundler entry files.
    let nonce = next_rebuild_nonce_hex();
    let bundle_tmp =
        std::env::temp_dir().join(format!("webui-press-bundle-{}-{nonce}", std::process::id(),));
    if bundle_tmp.exists() {
        fs::remove_dir_all(&bundle_tmp).ok();
    }
    fs::create_dir_all(&bundle_tmp)
        .map_err(|e| Error::Build(format!("Cannot create bundle temp dir: {e}")))?;
    let bundle_tmp = TempDirGuard::new(bundle_tmp);

    let assets_dir = opts.site_dir.join("assets");
    fs::create_dir_all(&assets_dir)
        .map_err(|e| Error::Io(format!("Cannot create assets dir: {e}")))?;

    let mut entry_files: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut component_imports = HashSet::new();

    if let Some(root) = opts.root_bundle {
        let entry_path = bundle_tmp.path().join("index.ts");
        let mut entry = String::with_capacity(32 + root.component_scripts.len() * 80);
        entry.push_str("// Root template script\n");
        let mut imports = HashSet::with_capacity(
            root.component_scripts.len() + usize::from(root.script_path.is_some()),
        );
        if let Some(path) = root.script_path.as_ref() {
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }
        for path in &root.component_scripts {
            component_imports.insert(path.clone());
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }
        fs::write(&entry_path, &entry)
            .map_err(|e| Error::Build(format!("Cannot write root script entry: {e}")))?;
        entry_files.push(("index".to_string(), entry_path));
    }

    // Write one virtual entry per page. Inline scripts are written as sibling
    // modules and imported, preserving module scope when a page has multiple
    // bundled script tags.
    for bundle in opts.page_bundles {
        let entry_name = format!("assets/page-{}", bundle.id);
        let entry_path = bundle_tmp.path().join(format!("{entry_name}.ts"));
        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Build(format!("Cannot create script entry dir: {e}")))?;
        }

        let mut entry = String::with_capacity(
            32 + bundle.page_path.len()
                + (bundle.component_scripts.len() + bundle.explicit_scripts.len()) * 80,
        );
        entry.push_str("// Page: ");
        entry.push_str(&bundle.page_path);
        entry.push('\n');
        let mut imports =
            HashSet::with_capacity(bundle.component_scripts.len() + bundle.explicit_scripts.len());

        for path in &bundle.component_scripts {
            component_imports.insert(path.clone());
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }

        for (idx, source) in bundle.explicit_scripts.iter().enumerate() {
            match source {
                ScriptSource::Inline(code) => {
                    validate_inline_script_imports(
                        code,
                        opts.config_dir,
                        &allowed_roots,
                        opts.bundler_config,
                    )?;
                    let inline_path = bundle_tmp
                        .path()
                        .join(format!("inline/page-{}-{idx}.ts", bundle.id));
                    if let Some(parent) = inline_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            Error::Build(format!("Cannot create inline script dir: {e}"))
                        })?;
                    }
                    fs::write(&inline_path, code)
                        .map_err(|e| Error::Build(format!("Cannot write inline script: {e}")))?;
                    let specifier = path_for_js(&inline_path);
                    push_import_once(&mut entry, &mut imports, &specifier);
                }
                ScriptSource::File(path) => {
                    let abs_path =
                        resolve_bundle_script_file(opts.config_dir, opts.content_dir, path)?;
                    validate_script_graph_imports(&abs_path, &allowed_roots, opts.bundler_config)?;
                    let specifier = path_for_js(&abs_path);
                    push_import_once(&mut entry, &mut imports, &specifier);
                }
            }
        }

        fs::write(&entry_path, &entry)
            .map_err(|e| Error::Build(format!("Cannot write script entry: {e}")))?;
        entry_files.push((entry_name, entry_path));
    }

    let args = esbuild_args(opts, &entry_files, bundle_tmp.path());
    let esbuild_bin = esbuild_command(node_modules);

    let output = std::process::Command::new(&esbuild_bin)
        .args(&args)
        .env("NODE_PATH", node_modules)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| Error::Build(format!("esbuild failed to start: {e}")))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(Error::Build(format!("esbuild error: {stderr}")));
    }

    let mut root_imports = HashSet::new();
    let root_script = if opts.root_bundle.is_some() {
        let output_file = "index.js";
        let full_path = opts.site_dir.join(output_file);
        if full_path.exists() {
            let contents = fs::read_to_string(&full_path)
                .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
            collect_leading_relative_imports(&contents, output_file, &mut root_imports);
            Some(versioned_asset_path(output_file, &full_path)?)
        } else {
            return Err(Error::Build(format!(
                "Bundled root script output missing: {}",
                full_path.display()
            )));
        }
    } else {
        None
    };

    // Build script_map: find output files for page-script entries.
    let mut script_map = HashMap::with_capacity(opts.page_bundles.len());
    for bundle in opts.page_bundles {
        let entry_name = format!("page-{}", bundle.id);
        // esbuild outputs entry chunks as `{entry_name}.js` in the output dir.
        let output_file = format!("assets/{entry_name}.js");
        let full_path = opts.site_dir.join(&output_file);
        if full_path.exists() {
            script_map.insert(
                bundle.id,
                page_script_paths(opts.site_dir, &output_file, &full_path, &root_imports)?,
            );
        } else {
            return Err(Error::Build(format!(
                "Bundled script output missing: {}",
                full_path.display()
            )));
        }
    }

    Ok(BundleResult {
        root_script,
        component_count: component_imports.len(),
        page_entry_count: opts.page_bundles.len(),
        script_map,
    })
}

/// Resolve the esbuild binary path from node_modules.
fn esbuild_command(node_modules: &Path) -> std::path::PathBuf {
    let binary = if cfg!(windows) {
        "esbuild.cmd"
    } else {
        "esbuild"
    };
    if let Some(project_dir) = node_modules.parent() {
        for dir in project_dir.ancestors() {
            let candidate = dir.join("node_modules").join(".bin").join(binary);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from(binary)
}

/// Extract `<script type="module" bundle>` and `<script type="module" bundle src="...">` tags
/// from page content HTML. Returns the modified content (with those tags removed)
/// and the extracted script sources.
///
/// The scanner is iterative and avoids regex (per project rules). It looks for
/// `<script` tags containing the `bundle` attribute.
pub(crate) fn extract_bundle_scripts(content: &str) -> (String, Vec<ScriptSource>) {
    let mut scripts = Vec::new();
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    let bytes = content.as_bytes();

    while cursor < bytes.len() {
        // Find next <script (case-sensitive — HTML from our pipeline is always lowercase).
        let Some(rel) = content[cursor..].find("<script") else {
            break;
        };
        let tag_start = cursor + rel;
        let after_tag = tag_start + 7; // len("<script")

        // Verify boundary: next char must be ' ', '\t', '\n', '\r', or '>'.
        match bytes.get(after_tag) {
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'>') => {}
            _ => {
                out.push_str(&content[cursor..after_tag]);
                cursor = after_tag;
                continue;
            }
        }

        // Find the end of the opening tag '>'
        let Some(gt_rel) = content[after_tag..].find('>') else {
            break;
        };
        let gt_pos = after_tag + gt_rel;
        let attrs_region = &content[after_tag..gt_pos];

        // Check for `bundle` attribute (space-separated, could be anywhere).
        if !has_bundle_attr(attrs_region) {
            out.push_str(&content[cursor..gt_pos + 1]);
            cursor = gt_pos + 1;
            continue;
        }

        // Find closing </script> tag.
        let Some(close_rel) = content[gt_pos + 1..].find("</script>") else {
            // Malformed — pass through the rest.
            break;
        };
        let close_start = gt_pos + 1 + close_rel;
        let close_end = close_start + "</script>".len();

        // Extract source info.
        let src_attr = extract_src_attr(attrs_region);
        let inline_body = &content[gt_pos + 1..close_start];

        let source = if let Some(src) = src_attr {
            ScriptSource::File(src.to_string())
        } else if !inline_body.trim().is_empty() {
            ScriptSource::Inline(inline_body.to_string())
        } else {
            // Empty script with no src — skip it.
            out.push_str(&content[cursor..close_end]);
            cursor = close_end;
            continue;
        };

        scripts.push(source);

        // Drop the original bundled script tag. The page's generated virtual
        // entry is injected once near the end of the rendered document.
        out.push_str(&content[cursor..tag_start]);
        cursor = close_end;
    }

    out.push_str(&content[cursor..]);
    (out, scripts)
}

/// Check if the attributes region contains a `bundle` attribute name.
fn has_bundle_attr(attrs: &str) -> bool {
    let bytes = attrs.as_bytes();
    let target = b"bundle";
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let name_start = i;
        while i < bytes.len()
            && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/' | b'>')
        {
            i += 1;
        }
        if name_start == i {
            i += 1;
            continue;
        }

        if i - name_start == target.len() && &bytes[name_start..i] == target {
            return true;
        }

        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }

        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
            }
        }
    }

    false
}

/// Extract the value of a `src="..."` or `src='...'` attribute from an attrs region.
fn extract_src_attr(attrs: &str) -> Option<&str> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some(rel) = attrs[i..].find("src") {
            let pos = i + rel;
            // Verify boundary before: space or start.
            let before_ok = pos == 0 || matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r');
            let after_src = pos + 3;
            if !before_ok || after_src >= bytes.len() {
                i = pos + 1;
                continue;
            }

            // Skip whitespace and '='.
            let mut eq = after_src;
            while eq < bytes.len() && matches!(bytes[eq], b' ' | b'\t') {
                eq += 1;
            }
            if eq >= bytes.len() || bytes[eq] != b'=' {
                i = pos + 1;
                continue;
            }
            eq += 1;
            while eq < bytes.len() && matches!(bytes[eq], b' ' | b'\t') {
                eq += 1;
            }

            // Read quoted value.
            if eq >= bytes.len() {
                i = pos + 1;
                continue;
            }
            let quote = bytes[eq];
            if quote != b'"' && quote != b'\'' {
                i = pos + 1;
                continue;
            }
            let val_start = eq + 1;
            if let Some(val_end_rel) = attrs[val_start..].find(quote as char) {
                return Some(&attrs[val_start..val_start + val_end_rel]);
            }
            i = pos + 1;
        } else {
            break;
        }
    }
    None
}

pub(crate) fn module_script_tag(base_path: &str, rel_path: &str) -> String {
    let mut tag = String::with_capacity(base_path.len() + rel_path.len() + 40);
    tag.push_str("\n<script type=\"module\" src=\"");
    if base_path.is_empty() || base_path == "/" {
        tag.push('/');
        tag.push_str(rel_path.trim_start_matches('/'));
    } else {
        tag.push_str(base_path.trim_end_matches('/'));
        tag.push('/');
        tag.push_str(rel_path.trim_start_matches('/'));
    }
    tag.push_str("\"></script>");
    tag
}

pub(crate) fn inject_module_script_tags(
    html: &mut String,
    base_path: &str,
    rel_paths: &[String],
) -> usize {
    let mut count = 0;
    for rel_path in rel_paths {
        let tag = module_script_tag(base_path, rel_path);
        inject_script_tag(html, &tag);
        count += 1;
    }
    count
}

pub(crate) fn inject_script_tag(html: &mut String, tag: &str) {
    if let Some(pos) = html.rfind("</body>") {
        html.insert_str(pos, tag);
    } else {
        html.push_str(tag);
    }
}

fn fxhash_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn test_hash(s: &str) -> u64 {
        fxhash_bytes(s.as_bytes())
    }

    #[test]
    fn esbuild_command_resolves_from_node_modules() {
        let tmp = std::env::temp_dir().join("webui-press-esbuild-test");
        let bin_dir = tmp.join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let bin_path = if cfg!(windows) {
            bin_dir.join("esbuild.cmd")
        } else {
            bin_dir.join("esbuild")
        };
        fs::write(&bin_path, "").unwrap();
        let resolved = esbuild_command(&tmp.join("node_modules"));
        assert_eq!(resolved, bin_path);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn push_external_args_filters_aliased_packages() {
        let external = vec![
            "@microsoft/webui-framework".to_string(),
            "cdn-only-package".to_string(),
        ];
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "@microsoft/webui-framework".to_string(),
            "/repo/packages/webui-framework/dist/index.js".to_string(),
        );
        let mut args = Vec::new();

        push_external_args(&mut args, &external, &aliases);

        assert!(!args
            .iter()
            .any(|arg| arg.contains("@microsoft/webui-framework")));
        assert!(args.contains(&"--external:cdn-only-package".to_string()));
    }

    #[test]
    fn config_component_source_preserves_npm_packages() {
        let cwd = Path::new("project");

        assert_eq!(resolve_config_component_source("@mai-ui", cwd), "@mai-ui");
        assert_eq!(
            resolve_config_component_source("@mai-ui/button", cwd),
            "@mai-ui/button"
        );
        assert_eq!(
            resolve_config_component_source("plain-widget", cwd),
            "plain-widget"
        );
    }

    #[test]
    fn config_component_source_resolves_local_paths() {
        let cwd = Path::new("project");

        assert_eq!(
            std::path::PathBuf::from(resolve_config_component_source("./components", cwd)),
            cwd.join("./components")
        );
    }

    #[test]
    fn discover_component_scripts_pairs_html_and_ts() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-component-script-test-{}-{:x}",
            std::process::id(),
            test_hash("component-script")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("my-widget"))?;
        fs::create_dir_all(root.join("html-only"))?;
        fs::write(root.join("my-widget/my-widget.ts"), "")?;
        fs::write(root.join("my-widget/my-widget.html"), "<p>widget</p>")?;
        fs::write(root.join("html-only/html-only.html"), "<p>no script</p>")?;

        let index = discover_component_scripts(&[root.to_string_lossy().into_owned()])?;
        let expected = root.join("my-widget/my-widget.ts").canonicalize()?;

        fs::remove_dir_all(&root)?;

        assert_eq!(index.len(), 1);
        let Some(script) = index.get("my-widget") else {
            panic!("my-widget script should be discovered");
        };
        assert_eq!(script.script_path, expected);
        Ok(())
    }

    #[test]
    fn collect_component_scripts_for_html_follows_nested_local_components() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-component-nesting-test-{}-{:x}",
            std::process::id(),
            test_hash("component-nesting")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("live-preview"))?;
        fs::create_dir_all(root.join("inner-card"))?;
        fs::write(
            root.join("live-preview/live-preview.html"),
            "<section><inner-card></inner-card></section>",
        )?;
        fs::write(root.join("live-preview/live-preview.ts"), "")?;
        fs::write(root.join("inner-card/inner-card.html"), "<slot></slot>")?;
        fs::write(root.join("inner-card/inner-card.ts"), "")?;

        let index = discover_component_scripts(&[root.to_string_lossy().into_owned()])?;
        let scripts =
            collect_component_scripts_for_html(&["<live-preview></live-preview>"], &index)?;
        let expected_live = root.join("live-preview/live-preview.ts").canonicalize()?;
        let expected_inner = root.join("inner-card/inner-card.ts").canonicalize()?;

        fs::remove_dir_all(&root)?;

        assert_eq!(scripts, vec![expected_live, expected_inner]);
        Ok(())
    }

    #[test]
    fn page_bundle_signature_matches_identical_import_sets() {
        let config_dir = Path::new("/repo/.webui-press");
        let components = vec![
            PathBuf::from("/repo/components/live-preview/live-preview.ts"),
            PathBuf::from("/repo/components/inner-card/inner-card.ts"),
        ];
        let reversed_components = vec![
            PathBuf::from("/repo/components/inner-card/inner-card.ts"),
            PathBuf::from("/repo/components/live-preview/live-preview.ts"),
        ];
        let scripts = vec![
            ScriptSource::File("./scripts/fluent.ts".to_string()),
            ScriptSource::Inline("import \"@mai-ui/button/define.js\";".to_string()),
        ];

        assert_eq!(
            page_bundle_signature(&components, &scripts, config_dir),
            page_bundle_signature(&reversed_components, &scripts, config_dir)
        );

        let changed = vec![ScriptSource::Inline(
            "import \"@mai-ui/card/define.js\";".to_string(),
        )];
        assert_ne!(
            page_bundle_signature(&components, &scripts, config_dir),
            page_bundle_signature(&components, &changed, config_dir)
        );
    }

    #[test]
    fn bundle_assets_skips_node_modules_when_no_scripts() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-no-script-bundle-test-{}-{:x}",
            std::process::id(),
            test_hash("no-script-bundle")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;

        let result = bundle_assets(&BundleOptions {
            site_dir: &root,
            node_modules: None,
            root_bundle: None,
            page_bundles: &[],
            bundler_config: None,
            dev_mode: false,
            config_dir: &root,
            content_dir: &root,
        })?;

        fs::remove_dir_all(&root)?;
        assert!(result.root_script.is_none());
        assert_eq!(result.page_entry_count, 0);
        Ok(())
    }

    #[test]
    fn temp_dir_guard_removes_directory_on_drop() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-temp-guard-test-{}-{:x}",
            std::process::id(),
            test_hash("temp-guard")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;

        {
            let _guard = TempDirGuard::new(root.clone());
            assert!(root.exists());
        }

        assert!(!root.exists());
        Ok(())
    }

    #[test]
    fn resolve_bundle_script_file_rejects_outside_project() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-script-sandbox-test-{}-{:x}",
            std::process::id(),
            test_hash("script-sandbox")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        let project = root.join("project");
        let config_dir = project.join(".webui-press");
        let outside = root.join("outside");
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&outside)?;
        fs::write(outside.join("secret.ts"), "console.log('secret');")?;

        let Err(err) = resolve_bundle_script_file(&config_dir, &project, "../../outside/secret.ts")
        else {
            panic!("outside script should be rejected");
        };

        fs::remove_dir_all(&root)?;
        assert!(err.to_string().contains("outside the docs project"));
        Ok(())
    }

    #[test]
    fn validate_inline_script_imports_rejects_absolute_import_after_code() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-inline-sandbox-test-{}-{:x}",
            std::process::id(),
            test_hash("inline-sandbox")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join(".webui-press"))?;
        let allowed_roots = allowed_script_roots(&root.join(".webui-press"), &root)?;

        let Err(err) = validate_inline_script_imports(
            "console.log('before'); import \"/tmp/secret.js\";",
            &root.join(".webui-press"),
            &allowed_roots,
            None,
        ) else {
            panic!("absolute import should be rejected");
        };

        fs::remove_dir_all(&root)?;
        assert!(err.to_string().contains("absolute filesystem path"));
        Ok(())
    }

    #[test]
    fn import_specifiers_find_imports_after_code_and_dynamic_imports() {
        let imports = import_specifiers(
            r#"console.log("start"); import "./after.js"; const x = import('@pkg/dynamic');"#,
        );
        assert_eq!(imports, vec!["./after.js", "@pkg/dynamic"]);
    }

    #[test]
    fn import_only_relative_imports_detects_wrapper_entries() {
        let imports = import_only_relative_imports(
            r#"import"./chunk-a.js";import "./chunk-b.js";"#,
            "assets/page-0.js",
        );
        assert_eq!(
            imports,
            Some(vec![
                "assets/chunk-a.js".to_string(),
                "assets/chunk-b.js".to_string()
            ])
        );

        assert_eq!(
            import_only_relative_imports(
                r#"import{a as b}from"./chunk-a.js";console.log(b);"#,
                "assets/page-0.js"
            ),
            None
        );
    }

    #[test]
    fn page_script_paths_flattens_import_only_wrapper() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-wrapper-flatten-test-{}-{:x}",
            std::process::id(),
            test_hash("wrapper-flatten")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        let assets = root.join("assets");
        fs::create_dir_all(&assets)?;
        fs::write(
            assets.join("page-0.js"),
            r#"import"./chunk-a.js";import"./chunk-b.js";import"./chunk-shared.js";"#,
        )?;
        fs::write(
            assets.join("chunk-a.js"),
            r#"import"./chunk-shared.js";console.log("a");"#,
        )?;
        fs::write(assets.join("chunk-b.js"), r#"console.log("b");"#)?;
        fs::write(assets.join("chunk-shared.js"), r#"console.log("shared");"#)?;

        let mut root_imports = HashSet::new();
        root_imports.insert("assets/chunk-shared.js".to_string());
        let paths = page_script_paths(
            &root,
            "assets/page-0.js",
            &assets.join("page-0.js"),
            &root_imports,
        )?;

        assert!(!assets.join("page-0.js").exists());
        assert_eq!(paths.len(), 2);
        assert!(paths[0].starts_with("assets/chunk-a.js?v="));
        assert!(paths[1].starts_with("assets/chunk-b.js?v="));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with("assets/chunk-shared.js")));

        fs::remove_dir_all(&root)?;
        Ok(())
    }

    // --- extract_bundle_scripts -------------------------------------------

    #[test]
    fn extract_bundle_scripts_inline() {
        let html = r#"<p>Hello</p><script type="module" bundle>import "@fluentui/web-components";</script><p>World</p>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(matches!(&scripts[0], ScriptSource::Inline(s) if s.contains("@fluentui")));
        assert!(!out.contains("<script"));
        assert!(out.contains("<p>Hello</p>"));
        assert!(out.contains("<p>World</p>"));
    }

    #[test]
    fn extract_bundle_scripts_src() {
        let html = r#"<script type="module" bundle src="./scripts/playground.ts"></script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(matches!(&scripts[0], ScriptSource::File(s) if s == "./scripts/playground.ts"));
        assert_eq!(out, "");
    }

    #[test]
    fn extract_bundle_scripts_ignores_non_bundle() {
        let html = r#"<script type="module">console.log("hi");</script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 0);
        assert_eq!(out, html);
    }

    #[test]
    fn extract_bundle_scripts_multiple() {
        let html = concat!(
            r#"<script type="module" bundle>import "a";</script>"#,
            r#"<p>middle</p>"#,
            r#"<script type="module" bundle src="./b.ts"></script>"#,
        );
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 2);
        assert!(matches!(&scripts[0], ScriptSource::Inline(s) if s.contains("import \"a\"")));
        assert!(matches!(&scripts[1], ScriptSource::File(s) if s == "./b.ts"));
        assert!(out.contains("<p>middle</p>"));
        assert!(!out.contains("<script"));
    }

    #[test]
    fn extract_bundle_scripts_empty_body_no_src_skipped() {
        let html = r#"<script type="module" bundle></script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 0);
        assert_eq!(out, html); // passes through unchanged
    }

    // --- has_bundle_attr --------------------------------------------------

    #[test]
    fn has_bundle_attr_standalone() {
        assert!(has_bundle_attr(r#" type="module" bundle"#));
        assert!(has_bundle_attr(r#" bundle type="module""#));
        assert!(has_bundle_attr(" bundle"));
        assert!(has_bundle_attr(r#" bundle="" type="module""#));
    }

    #[test]
    fn has_bundle_attr_not_substring() {
        assert!(!has_bundle_attr(r#" type="module" data-bundle="true""#));
        assert!(!has_bundle_attr(r#" type="module" data-mode="bundle""#));
        assert!(!has_bundle_attr(r#" unbundle"#));
    }

    // --- extract_src_attr -------------------------------------------------

    #[test]
    fn extract_src_attr_double_quotes() {
        assert_eq!(extract_src_attr(r#" src="./foo.ts""#), Some("./foo.ts"));
    }

    #[test]
    fn extract_src_attr_single_quotes() {
        assert_eq!(extract_src_attr(" src='bar.js'"), Some("bar.js"));
    }

    #[test]
    fn extract_src_attr_none_when_missing() {
        assert_eq!(extract_src_attr(r#" type="module" bundle"#), None);
    }

    // --- script tag injection --------------------------------------------

    #[test]
    fn module_script_tag_prefixes_base_path() {
        let tag = module_script_tag("/webui/", "assets/page-0.js?v=abc");
        assert_eq!(
            tag,
            "\n<script type=\"module\" src=\"/webui/assets/page-0.js?v=abc\"></script>"
        );
    }

    #[test]
    fn module_script_tag_handles_root_base_path() {
        assert_eq!(
            module_script_tag("/", "assets/page-0.js"),
            "\n<script type=\"module\" src=\"/assets/page-0.js\"></script>"
        );
    }

    #[test]
    fn inject_script_tag_inserts_before_body_close() {
        let mut html = "<html><body><p>content</p></body></html>".to_string();
        inject_script_tag(
            &mut html,
            "\n<script type=\"module\" src=\"/assets/page-0.js\"></script>",
        );
        assert!(html.contains(
            "<p>content</p>\n<script type=\"module\" src=\"/assets/page-0.js\"></script></body>"
        ));
    }
}
