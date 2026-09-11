// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! npm package resolution for external component discovery.
//!
//! Resolves npm packages from `node_modules/` using Node.js-style upward
//! traversal. Plugins own component naming and layout; the manifest helpers
//! support metadata-based discovery such as FAST.

use anyhow::{bail, Context, Result};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::cache::{CacheKey, DiscoveryCache};
use super::{DiscoveredComponent, DiscoveryPlugin};

/// Validated npm package context presented to a discovery plugin.
#[derive(Debug, Clone, Copy)]
pub struct PackageContext<'a> {
    /// npm package name.
    pub name: &'a str,
    /// Canonical package root.
    pub root: &'a Path,
    /// Parsed `package.json`, present only when the plugin opts into
    /// [`DiscoveryPlugin::requires_package_metadata`].
    pub manifest: Option<&'a serde_json::Value>,
}

pub(crate) struct ComponentDeclaration {
    pub(crate) tag_name: String,
    pub(crate) name: Option<String>,
    pub(crate) module_specifier: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ResolvedPackageModule {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
    pub(crate) relative_path: PathBuf,
    pub(crate) package_json: PathBuf,
    pub(crate) manifest: serde_json::Value,
    pub(crate) ordered_manifest: OrderedJson,
    pub(crate) resolution_dependencies: Vec<PathBuf>,
}

/// Maximum file size for package.json and custom elements manifests (10 MB).
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024;

/// Package fields that conventionally point to a browser/module entry.
const SCRIPT_ENTRY_FIELDS: &[&str] = &["main", "module", "browser"];

/// Resource-only exports do not imply registration scripts for metadata-based plugins.
const SCRIPTLESS_ASSET_EXPORTS: &[&str] = &[
    "./template-webui.html",
    "./styles.css",
    "./component-asset.js",
];

const ACTIVE_EXPORT_CONDITIONS: &[&str] = &["browser", "import", "default"];

enum ExportTarget {
    Target(String),
    Blocked,
    NoMatch,
    Invalid,
}

#[derive(Debug)]
pub(crate) enum OrderedJson {
    String(String),
    Object(Vec<(String, OrderedJson)>),
    Null,
    Other,
}

struct OrderedJsonSeed;

impl<'de> DeserializeSeed<'de> for OrderedJsonSeed {
    type Value = OrderedJson;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(OrderedJsonVisitor)
    }
}

struct OrderedJsonVisitor;

impl<'de> Visitor<'de> for OrderedJsonVisitor {
    type Value = OrderedJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::String(value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Null)
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Null)
    }

    fn visit_bool<E>(self, _value: bool) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Other)
    }

    fn visit_i64<E>(self, _value: i64) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Other)
    }

    fn visit_u64<E>(self, _value: u64) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Other)
    }

    fn visit_f64<E>(self, _value: f64) -> std::result::Result<Self::Value, E> {
        Ok(OrderedJson::Other)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(OrderedJsonSeed)?.is_some() {}
        Ok(OrderedJson::Other)
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            entries.push((key, map.next_value_seed(OrderedJsonSeed)?));
        }
        Ok(OrderedJson::Object(entries))
    }
}

impl OrderedJson {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find_map(|(candidate, value)| (candidate == key).then_some(value)),
            _ => None,
        }
    }
}

fn parse_ordered_json(content: &str, path: &Path) -> Result<OrderedJson> {
    let mut deserializer = serde_json::Deserializer::from_str(content);
    let value = OrderedJsonSeed
        .deserialize(&mut deserializer)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    deserializer
        .end()
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(value)
}

/// Check if a package name is a bare scope (e.g., `@reactive-ui` without a sub-package).
fn is_bare_scope(name: &str) -> bool {
    name.starts_with('@') && !name.contains('/')
}

fn validate_package_source(name: &str) -> Result<()> {
    let valid = match name.strip_prefix('@') {
        Some(scoped) => match scoped.split_once('/') {
            Some((scope, package)) => is_package_segment(scope) && is_package_segment(package),
            None => is_package_segment(scoped),
        },
        None => is_package_segment(name),
    };
    if !valid {
        return Err(invalid_package_source(name));
    }
    Ok(())
}

fn is_package_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with(['.', '_'])
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

#[cold]
#[inline(never)]
fn invalid_package_source(name: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Invalid npm component source '{name}'. Use a package name, @scope, or @scope/package \
         with an optional trailing /*. For local directories, use an explicit path such as \
         ./components instead of a package subpath."
    )
}

/// Validate that a relative path from package.json does not escape the package directory.
///
/// Rejects absolute paths, root-relative paths, and any `..` components
/// to prevent path traversal attacks.
fn validate_relative_path(rel_path: &str, field_name: &str) -> Result<()> {
    let path = Path::new(rel_path);
    if path.is_absolute() || rel_path.starts_with('/') || rel_path.starts_with('\\') {
        bail!("Absolute path not allowed in {}: {}", field_name, rel_path);
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!(
            "Parent directory traversal (..) not allowed in {}: {}",
            field_name,
            rel_path
        );
    }
    Ok(())
}

/// Read a file with a size limit to prevent denial-of-service via oversized manifests.
fn read_to_string_limited(path: &Path, max_size: u64) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("Failed to stat {}", path.display()))?;
    if metadata.len() > max_size {
        bail!(
            "File too large ({} bytes, max {}): {}",
            metadata.len(),
            max_size,
            path.display()
        );
    }
    fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))
}

/// Resolve an npm package or scope to discovered components.
pub fn resolve(
    name: &str,
    search_dir: &Path,
    plugin: &dyn DiscoveryPlugin,
    cache: &mut DiscoveryCache,
) -> Result<Vec<DiscoveredComponent>> {
    let name = name.strip_suffix("/*").unwrap_or(name);
    validate_package_source(name)?;
    // Walk up from the build's app directory first, then fall back to the
    // process working directory. The fallback covers callers whose app
    // directory lives outside the project (e.g. a system-temp scratch dir),
    // where the project's `node_modules` is only reachable from the cwd the
    // command was invoked in.
    let fallback = std::env::current_dir().unwrap_or_else(|_| search_dir.to_path_buf());
    let node_modules = find_package_node_modules(name, search_dir, &fallback)?;
    if is_bare_scope(name) {
        resolve_scoped(name, &node_modules, plugin, cache)
    } else {
        resolve_single(name, &node_modules, plugin, cache, false)
    }
}

fn find_package_node_modules(name: &str, primary: &Path, fallback: &Path) -> Result<PathBuf> {
    for start in [primary, fallback] {
        if let Some(resolution) = find_package_node_modules_from(name, start)? {
            return Ok(resolution.node_modules);
        }
    }
    bail!(
        "Package or scope '{name}' not found in node_modules (searched upward from {} and {}). \
         Install the required packages in the project before building.",
        primary.display(),
        fallback.display()
    );
}

struct PackageNodeModulesResolution {
    node_modules: PathBuf,
    probes: Vec<PathBuf>,
}

fn find_package_node_modules_from(
    name: &str,
    start: &Path,
) -> Result<Option<PackageNodeModulesResolution>> {
    let mut probes = Vec::new();
    for directory in start.ancestors() {
        let node_modules = directory.join("node_modules");
        let candidate = node_modules.join(name);
        probes.push(candidate.clone());
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                return Ok(Some(PackageNodeModulesResolution {
                    node_modules,
                    probes,
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect package {}", candidate.display()));
            }
        }
    }
    Ok(None)
}

/// Enumerate all sub-packages under a scoped directory (e.g., `@reactive-ui/*`).
fn resolve_scoped(
    scope: &str,
    node_modules: &Path,
    plugin: &dyn DiscoveryPlugin,
    cache: &mut DiscoveryCache,
) -> Result<Vec<DiscoveredComponent>> {
    let scope_dir = node_modules.join(scope);
    if !scope_dir.is_dir() {
        bail!(
            "Scoped package directory not found: {}",
            scope_dir.display()
        );
    }

    let mut entries = fs::read_dir(&scope_dir)
        .with_context(|| format!("Failed to read scope directory: {}", scope_dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_cached_key(fs::DirEntry::file_name);
    let mut all = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub_name = format!("{}/{}", scope, entry.file_name().to_string_lossy());
        all.extend(
            resolve_single(&sub_name, node_modules, plugin, cache, true)
                .with_context(|| format!("Failed to discover scope member '{sub_name}'"))?,
        );
    }

    Ok(all)
}

/// Resolve a single npm package to discovered components.
fn resolve_single(
    name: &str,
    node_modules: &Path,
    plugin: &dyn DiscoveryPlugin,
    cache: &mut DiscoveryCache,
    scope_member: bool,
) -> Result<Vec<DiscoveredComponent>> {
    let pkg_dir = node_modules.join(name);

    // Resolve symlinks (pnpm, npm workspaces, yarn)
    let pkg_dir = fs::canonicalize(&pkg_dir).with_context(|| {
        format!(
            "Package not found or broken symlink: {} (looked in {})",
            name,
            node_modules.display()
        )
    })?;

    if !pkg_dir.is_dir() {
        bail!("Package path is not a directory: {}", pkg_dir.display());
    }

    let pkg_json_path = pkg_dir.join("package.json");
    if !pkg_json_path.exists() {
        bail!("No package.json found at {}", pkg_json_path.display());
    }

    let pkg_json = if plugin.requires_package_metadata() {
        let content = read_to_string_limited(&pkg_json_path, MAX_MANIFEST_SIZE)?;
        Some(
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {}", pkg_json_path.display()))?,
        )
    } else {
        None
    };
    let metadata_path = pkg_json.as_ref().map(|_| pkg_json_path.as_path());
    let package = PackageContext {
        name,
        root: &pkg_dir,
        manifest: pkg_json.as_ref(),
    };
    if scope_member && !plugin.supports_package(package)? {
        return Ok(Vec::new());
    }
    let cache_files = plugin.package_cache_files(package)?;
    let fingerprint = DiscoveryCache::fingerprint(metadata_path, &cache_files)?;
    let cache_key = CacheKey {
        namespace: plugin.cache_namespace(),
        source: name,
        package_json: &pkg_json_path,
        fingerprint,
    };
    if let Some(cached) = cache.get(&cache_key)? {
        return Ok(cached);
    }
    let components = plugin.discover_package(package)?;

    // Do not persist a mixed snapshot if package files changed during discovery.
    if DiscoveryCache::fingerprint(metadata_path, &cache_files)? == fingerprint {
        cache.put(&cache_key, &components)?;
    }

    Ok(components)
}

pub(crate) fn package_component_declarations(
    package: PackageContext<'_>,
) -> Result<Vec<ComponentDeclaration>> {
    let path = custom_elements_manifest_path(package)?;
    parse_custom_elements_manifest(&path)
}

pub(crate) fn package_metadata(package: PackageContext<'_>) -> Result<&serde_json::Value> {
    package.manifest.with_context(|| {
        format!(
            "Discovery of '{}' needs package metadata. Opt in with requires_package_metadata().",
            package.name
        )
    })
}

pub(crate) fn package_export_path(
    package: PackageContext<'_>,
    key: &str,
) -> Result<Option<PathBuf>> {
    let package_json = package.root.join("package.json");
    let content = read_to_string_limited(&package_json, MAX_MANIFEST_SIZE)?;
    let ordered_manifest = parse_ordered_json(&content, &package_json)?;
    package_export_path_from_metadata(package.name, package.root, &ordered_manifest, key)
}

pub(crate) fn package_export_path_from_metadata(
    package_name: &str,
    package_root: &Path,
    manifest: &OrderedJson,
    key: &str,
) -> Result<Option<PathBuf>> {
    let Some(exports) = manifest.get("exports") else {
        return Ok(None);
    };
    let target = match resolve_export_target(exports, key) {
        ExportTarget::Target(target) => target,
        ExportTarget::Blocked | ExportTarget::NoMatch => return Ok(None),
        ExportTarget::Invalid => {
            bail!(
                "Package '{package_name}' export '{key}' must be a relative file path string \
                 or a supported conditional object"
            );
        }
    };
    validate_package_export_target(&target, key)?;
    let relative = Path::new(&target);
    validate_package_path_boundary(package_root, relative, key).with_context(|| {
        format!("Package '{package_name}' export '{key}' leaves the package boundary")
    })?;
    Ok(Some(package_root.join(relative)))
}

pub(crate) fn resolve_bare_module_specifier(
    specifier: &str,
    from: &Path,
) -> Result<Option<ResolvedPackageModule>> {
    let Some((package_name, subpath)) = bare_package_specifier(specifier)? else {
        return Ok(None);
    };
    let Some(resolution) = find_package_node_modules_from(package_name, from)? else {
        if package_name.starts_with('@') {
            bail!(
                "Package '{package_name}' referenced by CEM module specifier '{specifier}' \
                 was not found in node_modules"
            );
        }
        return Ok(None);
    };
    let node_modules = resolution.node_modules;
    let root = fs::canonicalize(node_modules.join(package_name)).with_context(|| {
        format!(
            "Package not found or broken symlink: {} (looked in {})",
            package_name,
            node_modules.display()
        )
    })?;
    if !root.is_dir() {
        bail!("Package path is not a directory: {}", root.display());
    }
    let package_json = root.join("package.json");
    let content = read_to_string_limited(&package_json, MAX_MANIFEST_SIZE)?;
    let manifest: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", package_json.display()))?;
    let ordered_manifest = parse_ordered_json(&content, &package_json)?;
    let relative_path =
        resolve_package_module_path(package_name, &root, subpath, &manifest, &ordered_manifest)
            .with_context(|| format!("Failed to resolve CEM module specifier '{specifier}'"))?;
    Ok(Some(ResolvedPackageModule {
        name: package_name.to_string(),
        root,
        relative_path,
        package_json,
        manifest,
        ordered_manifest,
        resolution_dependencies: resolution.probes,
    }))
}

pub(crate) fn bare_module_package_name(specifier: &str) -> Result<Option<&str>> {
    bare_package_specifier(specifier).map(|parsed| parsed.map(|(package_name, _)| package_name))
}

pub(crate) fn resolve_self_module_specifier(
    package_name: &str,
    package_root: &Path,
    specifier: &str,
    manifest: &serde_json::Value,
) -> Result<PathBuf> {
    let Some((specifier_package, subpath)) = bare_package_specifier(specifier)? else {
        bail!("CEM module specifier is not a bare package reference: {specifier}");
    };
    if specifier_package != package_name {
        bail!(
            "CEM module specifier '{specifier}' does not reference containing package '{package_name}'"
        );
    }
    let package_json = package_root.join("package.json");
    let content = read_to_string_limited(&package_json, MAX_MANIFEST_SIZE)?;
    let ordered_manifest = parse_ordered_json(&content, &package_json)?;
    resolve_package_module_path(
        package_name,
        package_root,
        subpath,
        manifest,
        &ordered_manifest,
    )
}

fn bare_package_specifier(specifier: &str) -> Result<Option<(&str, Option<&str>)>> {
    if specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier.starts_with('\\')
    {
        return Ok(None);
    }
    validate_relative_path(specifier, "customElements modules[].path")?;

    if specifier.starts_with('@') {
        let Some(scope_end) = specifier.find('/') else {
            bail!("Invalid scoped CEM module specifier: {specifier}");
        };
        let remainder = &specifier[scope_end + 1..];
        let (package, subpath) = remainder
            .split_once('/')
            .map_or((remainder, None), |(package, subpath)| {
                (package, Some(subpath))
            });
        let package_name = &specifier[..scope_end + 1 + package.len()];
        validate_package_source(package_name)?;
        validate_package_subpath(subpath, specifier)?;
        return Ok(Some((package_name, subpath)));
    }

    let (package_name, subpath) = specifier
        .split_once('/')
        .map_or((specifier, None), |(package, subpath)| {
            (package, Some(subpath))
        });
    if !is_package_segment(package_name) {
        return Ok(None);
    }
    validate_package_subpath(subpath, specifier)?;
    Ok(Some((package_name, subpath)))
}

fn validate_package_subpath(subpath: Option<&str>, specifier: &str) -> Result<()> {
    if subpath.is_some_and(|path| {
        path.is_empty()
            || path.starts_with('/')
            || path.ends_with('/')
            || path.contains('\\')
            || contains_encoded_separator(path)
            || path
                .split('/')
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".." | "node_modules"))
    }) {
        bail!("Invalid CEM module specifier: {specifier}");
    }
    Ok(())
}

fn resolve_package_module_path(
    package_name: &str,
    package_root: &Path,
    subpath: Option<&str>,
    manifest: &serde_json::Value,
    ordered_manifest: &OrderedJson,
) -> Result<PathBuf> {
    let export_key = subpath.map_or_else(|| ".".to_string(), |path| format!("./{path}"));
    let (target, is_export) = if let Some(exports) = ordered_manifest.get("exports") {
        let target = match resolve_export_target(exports, &export_key) {
            ExportTarget::Target(target) => target,
            ExportTarget::Blocked => {
                bail!("Package '{package_name}' blocks export '{export_key}'")
            }
            ExportTarget::NoMatch => {
                bail!("Package '{package_name}' does not export '{export_key}'")
            }
            ExportTarget::Invalid => {
                bail!("Package '{package_name}' has an invalid export '{export_key}'")
            }
        };
        (target, true)
    } else if let Some(subpath) = subpath {
        (subpath.to_string(), false)
    } else {
        (
            ["module", "main", "browser"]
                .into_iter()
                .find_map(|field| manifest.get(field).and_then(serde_json::Value::as_str))
                .unwrap_or("index.js")
                .to_string(),
            false,
        )
    };
    if is_export {
        validate_package_export_target(&target, &export_key)?;
    } else {
        validate_relative_path(&target, "package module path")?;
    }
    let relative = PathBuf::from(target);
    validate_package_path_boundary(package_root, &relative, "package module path")?;
    Ok(relative)
}

fn resolve_export_target(exports: &OrderedJson, key: &str) -> ExportTarget {
    match exports {
        OrderedJson::Object(entries)
            if entries
                .iter()
                .any(|(candidate, _)| candidate.starts_with('.')) =>
        {
            if let Some(value) = exports.get(key) {
                resolve_export_value(value)
            } else {
                resolve_pattern_export_target(entries, key)
            }
        }
        _ if key == "." => resolve_export_value(exports),
        _ => ExportTarget::NoMatch,
    }
}

fn resolve_export_value(value: &OrderedJson) -> ExportTarget {
    match value {
        OrderedJson::String(target) => ExportTarget::Target(target.clone()),
        OrderedJson::Null => ExportTarget::Blocked,
        OrderedJson::Object(entries) => {
            for (condition, target) in entries {
                if ACTIVE_EXPORT_CONDITIONS.contains(&condition.as_str()) {
                    match resolve_export_value(target) {
                        ExportTarget::NoMatch => {}
                        resolved => return resolved,
                    }
                }
            }
            ExportTarget::NoMatch
        }
        _ => ExportTarget::Invalid,
    }
}

fn resolve_pattern_export_target(exports: &[(String, OrderedJson)], key: &str) -> ExportTarget {
    let mut selected = None;
    for (pattern, value) in exports {
        let Some((prefix, suffix)) = pattern.split_once('*') else {
            continue;
        };
        if suffix.contains('*')
            || key.len() < prefix.len() + suffix.len()
            || !key.starts_with(prefix)
            || !key.ends_with(suffix)
        {
            continue;
        }
        let matched = &key[prefix.len()..key.len() - suffix.len()];
        let score = (prefix.len(), suffix.len());
        if selected
            .as_ref()
            .is_none_or(|(best_score, _, _)| score > *best_score)
        {
            selected = Some((score, value, matched));
        }
    }
    let Some((_, value, matched)) = selected else {
        return ExportTarget::NoMatch;
    };
    match resolve_export_value(value) {
        ExportTarget::Target(target) => ExportTarget::Target(target.replace('*', matched)),
        resolved => resolved,
    }
}

fn validate_package_export_target(target: &str, key: &str) -> Result<()> {
    let Some(relative) = target.strip_prefix("./") else {
        bail!("Package export '{key}' target must start with './': {target}");
    };
    if relative.is_empty()
        || target.contains('\\')
        || contains_encoded_separator(target)
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".." | "node_modules"))
    {
        bail!("Invalid package export '{key}' target: {target}");
    }
    validate_relative_path(target, key)
}

fn contains_encoded_separator(value: &str) -> bool {
    value.as_bytes().windows(3).any(|bytes| {
        bytes[0] == b'%'
            && ((bytes[1] == b'2' && bytes[2].eq_ignore_ascii_case(&b'f'))
                || (bytes[1] == b'5' && bytes[2].eq_ignore_ascii_case(&b'c')))
    })
}

pub(crate) fn validate_package_asset_path(
    root: &Path,
    path: &Path,
    field_name: &str,
) -> Result<()> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "{field_name} is outside package root {}: {}",
            root.display(),
            path.display()
        )
    })?;
    validate_package_path_boundary(root, relative, field_name)
}

fn validate_package_path_boundary(root: &Path, relative: &Path, field_name: &str) -> Result<()> {
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("Failed to resolve package root {}", root.display()))?;
    let target = root.join(relative);
    let mut existing = target.as_path();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => {
                let canonical = fs::canonicalize(existing).with_context(|| {
                    format!("Failed to resolve {field_name}: {}", existing.display())
                })?;
                if !canonical.starts_with(&canonical_root) {
                    bail!(
                        "{field_name} resolves outside package root {}: {}",
                        root.display(),
                        target.display()
                    );
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect {field_name}: {}", existing.display())
                });
            }
        }
        existing = existing.parent().with_context(|| {
            format!(
                "Could not validate {field_name} within package root {}",
                root.display()
            )
        })?;
    }
}

pub(crate) fn custom_elements_manifest_path(package: PackageContext<'_>) -> Result<PathBuf> {
    let package_json = package.root.join("package.json");
    let relative = package_metadata(package)?
        .get("customElements")
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("No 'customElements' field in {}", package_json.display()))?;
    validate_relative_path(relative, "customElements")?;
    let path = package.root.join(relative);
    validate_package_asset_path(package.root, &path, "customElements").with_context(|| {
        format!(
            "Custom elements manifest leaves package '{}': {}",
            package.name,
            path.display()
        )
    })?;
    Ok(path)
}

pub(crate) fn read_required_file(path: &Path, kind: &str) -> Result<String> {
    read_to_string_limited(path, MAX_MANIFEST_SIZE)
        .with_context(|| format!("Failed to read {kind}: {}", path.display()))
}

pub(crate) fn read_optional_file(path: Option<&Path>, kind: &str) -> Result<Option<String>> {
    match path {
        Some(path) if path.is_file() => read_required_file(path, kind).map(Some),
        _ => Ok(None),
    }
}

pub(crate) fn package_has_authored_script(pkg_json: &serde_json::Value) -> bool {
    for field in SCRIPT_ENTRY_FIELDS {
        match pkg_json.get(*field) {
            Some(serde_json::Value::String(path)) if is_script_path(path) => return true,
            Some(serde_json::Value::Object(map)) if map.values().any(export_value_has_script) => {
                return true;
            }
            _ => {}
        }
    }

    let Some(exports) = pkg_json.get("exports") else {
        return false;
    };
    match exports {
        serde_json::Value::String(path) => is_script_path(path),
        serde_json::Value::Array(values) => values.iter().any(export_value_has_script),
        serde_json::Value::Object(map) => {
            if let Some(root) = map.get(".") {
                return export_value_has_script(root);
            }
            map.iter()
                .any(|(key, value)| !is_resource_export(key) && export_value_has_script(value))
        }
        _ => false,
    }
}

fn export_value_has_script(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(path) => is_script_path(path),
        serde_json::Value::Array(values) => values.iter().any(export_value_has_script),
        serde_json::Value::Object(map) => map.values().any(export_value_has_script),
        _ => false,
    }
}

fn is_resource_export(key: &str) -> bool {
    SCRIPTLESS_ASSET_EXPORTS.contains(&key)
}

fn is_script_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(prefix, _)| prefix);
    path.ends_with(".js")
        || path.ends_with(".mjs")
        || path.ends_with(".cjs")
        || path.ends_with(".ts")
}

// Parse `modules[].{path,declarations[].{name,tagName}}` from a Custom Elements
// Manifest.
fn parse_custom_elements_manifest(path: &Path) -> Result<Vec<ComponentDeclaration>> {
    let content = read_to_string_limited(path, MAX_MANIFEST_SIZE)
        .with_context(|| format!("Custom elements manifest: {}", path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse custom elements manifest: {}",
            path.display()
        )
    })?;

    let mut seen = std::collections::HashSet::new();
    let mut declarations = Vec::new();

    if let Some(modules) = manifest.get("modules").and_then(|v| v.as_array()) {
        for module in modules {
            let module_path = module.get("path").and_then(|value| value.as_str());
            if let Some(path) = module_path {
                validate_relative_path(path, "customElements modules[].path")?;
            }
            if let Some(module_declarations) = module.get("declarations").and_then(|v| v.as_array())
            {
                for declaration in module_declarations {
                    let Some(tag_name) =
                        declaration.get("tagName").and_then(|value| value.as_str())
                    else {
                        continue;
                    };
                    if seen.insert(tag_name) {
                        declarations.push(ComponentDeclaration {
                            tag_name: tag_name.to_string(),
                            name: declaration
                                .get("name")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            module_specifier: module_path.map(str::to_string),
                        });
                    }
                }
            }
        }
    }

    Ok(declarations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FastDiscoveryPlugin, WebUIDiscoveryPlugin};
    use std::fs;
    use tempfile::TempDir;

    fn ordered_json(source: &str) -> OrderedJson {
        parse_ordered_json(source, Path::new("package.json")).unwrap()
    }

    fn resolve_test_package_module_path(
        root: &Path,
        subpath: Option<&str>,
        source: &str,
    ) -> Result<PathBuf> {
        let manifest = serde_json::from_str(source).unwrap();
        let ordered_manifest = ordered_json(source);
        resolve_package_module_path(
            "external-module",
            root,
            subpath,
            &manifest,
            &ordered_manifest,
        )
    }

    fn create_npm_package(dir: &Path, name: &str, tag_name: &str, html: &str, css: Option<&str>) {
        let pkg_dir = dir.join(name);
        fs::create_dir_all(&pkg_dir).unwrap();

        // Create template
        fs::write(pkg_dir.join(format!("{tag_name}.html")), html).unwrap();

        // Create styles (optional)
        if let Some(css_content) = css {
            fs::write(pkg_dir.join(format!("{tag_name}.css")), css_content).unwrap();
        }

        let pkg_json = serde_json::json!({
            "name": name,
            "version": "1.0.0"
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string_pretty(&pkg_json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_find_node_modules_in_cwd() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir_all(nm.join("fixture-pkg")).unwrap();

        let result = find_package_node_modules("fixture-pkg", tmp.path(), tmp.path());
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            nm.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_node_modules_walks_up() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir_all(nm.join("fixture-pkg")).unwrap();
        let sub = tmp.path().join("packages").join("my-app");
        fs::create_dir_all(&sub).unwrap();

        let result = find_package_node_modules("fixture-pkg", &sub, &sub);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            nm.canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_node_modules_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = find_package_node_modules("fixture-pkg", tmp.path(), tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_find_node_modules_fallback_used_when_primary_empty() {
        let primary = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let nm = project.path().join("node_modules");
        fs::create_dir_all(nm.join("fixture-pkg")).unwrap();

        let found =
            find_package_node_modules("fixture-pkg", primary.path(), project.path()).unwrap();
        assert_eq!(found, nm);
    }

    #[test]
    fn test_find_node_modules_fallback_prefers_primary() {
        let primary = TempDir::new().unwrap();
        let nm_primary = primary.path().join("node_modules");
        fs::create_dir_all(nm_primary.join("fixture-pkg")).unwrap();
        let project = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join("node_modules/fixture-pkg")).unwrap();

        let found =
            find_package_node_modules("fixture-pkg", primary.path(), project.path()).unwrap();
        assert_eq!(found, nm_primary);
    }

    #[test]
    fn test_find_node_modules_fallback_errors_when_neither_has_it() {
        let primary = TempDir::new().unwrap();
        let fallback = TempDir::new().unwrap();
        assert!(find_package_node_modules("fixture-pkg", primary.path(), fallback.path()).is_err());
    }

    #[test]
    fn test_bare_module_resolution_tracks_all_ancestor_probes() {
        let tmp = TempDir::new().unwrap();
        let app = tmp.path().join("packages/app");
        fs::create_dir_all(&app).unwrap();
        let plain = tmp.path().join("node_modules/plain-module");
        fs::create_dir_all(&plain).unwrap();
        fs::write(
            plain.join("package.json"),
            r#"{"exports":{"./component.js":"./component.js"}}"#,
        )
        .unwrap();
        let scoped = tmp.path().join("node_modules/@fixture/scoped-module");
        fs::create_dir_all(&scoped).unwrap();
        fs::write(
            scoped.join("package.json"),
            r#"{"exports":{"./component.js":"./component.js"}}"#,
        )
        .unwrap();

        let plain_resolution = resolve_bare_module_specifier("plain-module/component.js", &app)
            .unwrap()
            .unwrap();
        assert_eq!(
            plain_resolution.resolution_dependencies,
            [
                app.join("node_modules/plain-module"),
                tmp.path().join("packages/node_modules/plain-module"),
                tmp.path().join("node_modules/plain-module"),
            ]
        );

        let scoped_resolution =
            resolve_bare_module_specifier("@fixture/scoped-module/component.js", &app)
                .unwrap()
                .unwrap();
        assert_eq!(
            scoped_resolution.resolution_dependencies,
            [
                app.join("node_modules/@fixture/scoped-module"),
                tmp.path()
                    .join("packages/node_modules/@fixture/scoped-module"),
                tmp.path().join("node_modules/@fixture/scoped-module"),
            ]
        );
    }

    #[test]
    fn test_is_bare_scope() {
        assert!(is_bare_scope("@reactive-ui"));
        assert!(is_bare_scope("@scope"));
        assert!(!is_bare_scope("@scope/button"));
        assert!(!is_bare_scope("my-widget"));
    }

    #[test]
    fn test_resolve_single_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        create_npm_package(
            &nm,
            "my-widget",
            "my-widget",
            "<div><slot></slot></div>",
            Some(".widget { color: blue; }"),
        );

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "my-widget",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(result.is_ok());

        let components = result.unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].tag_name, "my-widget");
        assert_eq!(components[0].html_content, "<div><slot></slot></div>");
        assert_eq!(
            components[0].css_content.as_deref(),
            Some(".widget { color: blue; }")
        );
        assert!(!components[0].is_client_owned);
    }

    #[test]
    fn test_fast_package_script_ownership_uses_manifest_entrypoints() {
        let static_pkg = serde_json::json!({
            "exports": {
                "./component-asset.js": "./component-asset.js",
                "./styles.css": "./styles.css"
            }
        });
        let interactive_pkg = serde_json::json!({
            "exports": {
                ".": { "import": "./dist/index.js" }
            }
        });
        let module_pkg = serde_json::json!({
            "module": "./dist/index.mjs"
        });

        assert!(!package_has_authored_script(&static_pkg));
        assert!(package_has_authored_script(&interactive_pkg));
        assert!(package_has_authored_script(&module_pkg));
    }

    #[test]
    fn test_resolve_scoped_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        let scope_dir = nm.join("@mylib");
        fs::create_dir_all(&scope_dir).unwrap();

        create_npm_package(
            &scope_dir,
            "button",
            "mylib-button",
            "<button><slot></slot></button>",
            Some(".btn { padding: 8px; }"),
        );
        create_npm_package(
            &scope_dir,
            "text",
            "mylib-text",
            "<span><slot></slot></span>",
            None,
        );

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "@mylib",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(result.is_ok());

        let components = result.unwrap();
        assert_eq!(components.len(), 2);

        let names: Vec<&str> = components.iter().map(|c| c.tag_name.as_str()).collect();
        assert!(names.contains(&"mylib-button"));
        assert!(names.contains(&"mylib-text"));
    }

    #[test]
    fn test_resolve_missing_package() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "nonexistent",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_custom_elements_manifest() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [
                {
                    "kind": "javascript-module",
                    "path": "src/button.js",
                    "declarations": [
                        {
                            "kind": "class",
                            "name": "MyButton",
                            "tagName": "my-button"
                        }
                    ]
                },
                {
                    "kind": "javascript-module",
                    "path": "src/text.js",
                    "declarations": [
                        {
                            "kind": "class",
                            "name": "MyText",
                            "tagName": "my-text"
                        },
                        {
                            "kind": "variable",
                            "name": "VERSION"
                        }
                    ]
                }
            ]
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        let result = parse_custom_elements_manifest(&cem_path);
        assert!(result.is_ok());

        let declarations = result.unwrap();
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0].tag_name, "my-button");
        assert_eq!(declarations[1].tag_name, "my-text");
    }

    #[test]
    fn test_parse_custom_elements_manifest_empty() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": []
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        let result = parse_custom_elements_manifest(&cem_path);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_custom_elements_manifest_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [
                {
                    "kind": "javascript-module",
                    "path": "src/button.js",
                    "declarations": [{ "kind": "class", "name": "MyButton", "tagName": "my-button" }]
                },
                {
                    "kind": "javascript-module",
                    "path": "src/other-button.js",
                    "declarations": [{ "kind": "class", "name": "OtherButton", "tagName": "my-button" }]
                }
            ]
        });

        let cem_path = tmp.path().join("custom-elements.json");
        fs::write(&cem_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        let declarations = parse_custom_elements_manifest(&cem_path).unwrap();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].tag_name, "my-button");
    }

    #[test]
    fn test_cached_result_is_reused() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        create_npm_package(&nm, "cached-pkg", "cached-comp", "<div>cached</div>", None);

        let mut cache = DiscoveryCache::open().unwrap();

        // First resolve: populates cache
        let first = resolve(
            "cached-pkg",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        )
        .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tag_name, "cached-comp");

        // Second resolve: should hit cache
        let second = resolve(
            "cached-pkg",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        )
        .unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].tag_name, "cached-comp");
    }

    #[test]
    fn test_validate_relative_path_rejects_absolute() {
        let result = validate_relative_path("/etc/passwd", "exports");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Absolute path"));
    }

    #[test]
    fn test_validate_relative_path_rejects_parent_traversal() {
        let result = validate_relative_path("../../etc/passwd", "exports");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));
    }

    #[test]
    fn test_validate_relative_path_rejects_hidden_traversal() {
        let result = validate_relative_path("foo/../../../etc/passwd", "exports");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_accepts_valid() {
        assert!(validate_relative_path("./dist/template.html", "exports").is_ok());
        assert!(validate_relative_path("custom-elements.json", "customElements").is_ok());
        assert!(validate_relative_path("dist/nested/file.css", "exports").is_ok());
    }

    #[test]
    fn test_bare_module_export_cannot_escape_package() {
        let tmp = TempDir::new().unwrap();
        let package = tmp.path().join("node_modules/external-module");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"exports":{"./component.js":"./dist/../outside.js"}}"#,
        )
        .unwrap();

        let error =
            resolve_bare_module_specifier("external-module/component.js", tmp.path()).unwrap_err();
        assert!(format!("{error:#}").contains(".."));
    }

    #[test]
    fn test_bare_module_resolves_pattern_export() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_test_package_module_path(
            tmp.path(),
            Some("components/button"),
            r#"{
            "exports": {
                "./components/*": {
                    "default": "./dist/*.js"
                }
            }
        }"#,
        )
        .unwrap();
        assert_eq!(path, PathBuf::from("./dist/button.js"));
    }

    #[test]
    fn test_bare_module_uses_first_active_export_condition() {
        let tmp = TempDir::new().unwrap();
        let path = resolve_test_package_module_path(
            tmp.path(),
            Some("component.js"),
            r#"{
            "exports": {
                "./component.js": {
                    "default": "./dist/default.js",
                    "import": "./dist/import.js"
                }
            }
        }"#,
        )
        .unwrap();

        assert_eq!(path, PathBuf::from("./dist/default.js"));
    }

    #[test]
    fn test_active_null_condition_blocks_later_default() {
        let tmp = TempDir::new().unwrap();
        let error = resolve_test_package_module_path(
            tmp.path(),
            Some("component.js"),
            r#"{
            "exports": {
                "./component.js": {
                    "import": null,
                    "default": "./dist/default.js"
                }
            }
        }"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("blocks export"));
    }

    #[test]
    fn test_exact_null_export_blocks_pattern_fallback() {
        let tmp = TempDir::new().unwrap();
        assert!(resolve_test_package_module_path(
            tmp.path(),
            Some("component.js"),
            r#"{
            "exports": {
                "./component.js": null,
                "./*": "./dist/*.js"
            }
        }"#,
        )
        .is_err());
    }

    #[test]
    fn test_package_exports_reject_invalid_target_segments() {
        let tmp = TempDir::new().unwrap();
        for target in [
            "dist/component.js",
            "./node_modules/component.js",
            "./dist/./component.js",
            "./dist%2fcomponent.js",
            "./dist%5Ccomponent.js",
        ] {
            let source = serde_json::json!({
                "exports": {"./component.js": target}
            })
            .to_string();
            assert!(
                resolve_test_package_module_path(tmp.path(), Some("component.js"), &source,)
                    .is_err()
            );
        }
    }

    #[test]
    fn test_package_export_symlink_cannot_escape_root() {
        let tmp = TempDir::new().unwrap();
        let package = tmp.path().join("package");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("component.js"), "export {};").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, package.join("link")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, package.join("link")).unwrap();
        let error = resolve_test_package_module_path(
            &package,
            Some("component.js"),
            r#"{"exports":{"./component.js":"./link/component.js"}}"#,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("outside package root"));
    }

    #[test]
    fn test_asset_exports_support_patterns_and_exact_blocks() {
        let tmp = TempDir::new().unwrap();
        let manifest = ordered_json(
            r#"{
            "exports": {
                "./*.html": "./assets/*.html",
                "./*.css": "./assets/*.css"
            }
        }"#,
        );

        let template = package_export_path_from_metadata(
            "fixture-package",
            tmp.path(),
            &manifest,
            "./template-webui.html",
        )
        .unwrap();
        let styles = package_export_path_from_metadata(
            "fixture-package",
            tmp.path(),
            &manifest,
            "./styles.css",
        )
        .unwrap();

        assert_eq!(
            template,
            Some(tmp.path().join("./assets/template-webui.html"))
        );
        assert_eq!(styles, Some(tmp.path().join("./assets/styles.css")));

        let blocked = ordered_json(
            r#"{
            "exports": {
                "./template-webui.html": null,
                "./*.html": "./assets/*.html"
            }
        }"#,
        );
        assert!(package_export_path_from_metadata(
            "fixture-package",
            tmp.path(),
            &blocked,
            "./template-webui.html",
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn test_custom_elements_manifest_symlink_cannot_escape_root() {
        let tmp = TempDir::new().unwrap();
        let package = tmp.path().join("package");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("custom-elements.json"), r#"{"modules":[]}"#).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.join("custom-elements.json"),
            package.join("custom-elements.json"),
        )
        .unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(
            outside.join("custom-elements.json"),
            package.join("custom-elements.json"),
        )
        .unwrap();
        let manifest = serde_json::json!({
            "customElements": "./custom-elements.json"
        });
        let context = PackageContext {
            name: "fixture-package",
            root: &package,
            manifest: Some(&manifest),
        };

        let error = custom_elements_manifest_path(context).unwrap_err();

        assert!(format!("{error:#}").contains("leaves package"));
    }

    #[test]
    fn test_resolve_symlinked_package() {
        // Simulates pnpm-style layout where node_modules/@scope/pkg is a symlink
        // to node_modules/.pnpm/@scope+pkg@version/node_modules/@scope/pkg.
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();

        // Create the real package inside .pnpm store
        let pnpm_store = nm
            .join(".pnpm")
            .join("my-widget@1.0.0")
            .join("node_modules");
        fs::create_dir_all(&pnpm_store).unwrap();
        create_npm_package(
            &pnpm_store,
            "my-widget",
            "my-widget",
            "<div>symlinked</div>",
            Some(".w { color: red; }"),
        );

        // Create a symlink: node_modules/my-widget -> .pnpm store location
        let link_path = nm.join("my-widget");
        let target_path = pnpm_store.join("my-widget");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_path, &link_path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target_path, &link_path).unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "my-widget",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(
            result.is_ok(),
            "symlinked package should resolve: {result:?}"
        );

        let components = result.unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].tag_name, "my-widget");
        assert_eq!(components[0].html_content, "<div>symlinked</div>");
    }

    #[test]
    fn test_resolve_scoped_symlinked_package() {
        // Simulates pnpm layout for scoped packages:
        // node_modules/@scope/pkg -> .pnpm/@scope+pkg@ver/node_modules/@scope/pkg
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        let scope_dir = nm.join("@mai-ui");
        fs::create_dir_all(&scope_dir).unwrap();

        // Real package inside .pnpm store
        let pnpm_store = nm
            .join(".pnpm")
            .join("@mai-ui+button@1.8.3")
            .join("node_modules")
            .join("@mai-ui");
        fs::create_dir_all(&pnpm_store).unwrap();
        create_npm_package(
            &pnpm_store,
            "button",
            "mai-button",
            "<button><slot></slot></button>",
            None,
        );

        // Symlink: node_modules/@mai-ui/button -> .pnpm store
        let link_path = scope_dir.join("button");
        let target_path = pnpm_store.join("button");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_path, &link_path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target_path, &link_path).unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "@mai-ui",
            tmp.path(),
            &WebUIDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(
            result.is_ok(),
            "scoped symlinked package should resolve: {result:?}"
        );

        let components = result.unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].tag_name, "mai-button");
    }

    #[test]
    fn test_fast_resolve_rejects_path_traversal_in_manifest() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        let pkg_dir = nm.join("evil-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        // FAST still validates metadata paths before reading them.
        let pkg_json = serde_json::json!({
            "name": "evil-pkg",
            "version": "1.0.0",
            "customElements": "../../../etc/passwd"
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string(&pkg_json).unwrap(),
        )
        .unwrap();

        let mut cache = DiscoveryCache::open().unwrap();
        let result = resolve(
            "evil-pkg",
            tmp.path(),
            &FastDiscoveryPlugin::new(),
            &mut cache,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".."));
    }
}
