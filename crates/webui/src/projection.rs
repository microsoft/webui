// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time consumption of bundler-neutral state projection manifests.

use crate::error::WebUIError;
use crate::ProjectionManifestSource;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use webui_parser::Diagnostic;
use webui_protocol::projection_manifest::{self, ProjectionComponent, ProjectionManifest};

/// Stable machine-readable projection diagnostic codes.
pub mod codes {
    /// Manifest or a declared physical file is missing/unreadable.
    pub const MANIFEST_UNREADABLE: &str = "PROJ-M001";
    /// Declared input content changed.
    pub const STALE_INPUT: &str = "PROJ-M003";
    /// Declared output content changed.
    pub const STALE_OUTPUT: &str = "PROJ-M004";
    /// Duplicate component ownership across fragments.
    pub const DUPLICATE_COMPONENT_OWNERSHIP: &str = "PROJ-M006";
    /// Same canonical artifact observed with conflicting hashes.
    pub const CONFLICTING_HASH: &str = "PROJ-M007";
    /// Missing, duplicate, unknown, or invalid manifest field/reference.
    pub const INVALID_FIELD: &str = "PROJ-M009";
    /// A compiled scripted component has no manifest entry.
    pub const MISSING_COVERAGE: &str = "PROJ-B001";
    /// Projection metadata was supplied to an incompatible plugin.
    pub const INCOMPATIBLE_PLUGIN: &str = "PROJ-B002";
    /// Manifest exceeds 16 MiB.
    pub const MANIFEST_TOO_LARGE: &str = "PROJ-S001";
    /// Root/path escapes or violates canonical syntax.
    pub const PATH_TRAVERSAL: &str = "PROJ-S003";
    /// Invalid SHA-256/virtual pairing.
    pub const INVALID_HASH_FORMAT: &str = "PROJ-S004";
}

const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST_BYTES_USIZE: usize = 16 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 16 * 1024;

/// One component's exact, validated client projection surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComponentEntry {
    pub hydration_keys: Vec<String>,
    pub navigation_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ArtifactIdentity {
    Physical(PathBuf),
    Virtual(String),
}

struct ValidatedFiles {
    manifest_values: BTreeMap<String, String>,
    identities: BTreeMap<ArtifactIdentity, String>,
}

struct LoadedFragment {
    components: BTreeMap<String, ComponentEntry>,
    artifacts: BTreeMap<ArtifactIdentity, String>,
}

/// Load, validate, and merge all disk manifest fragments.
pub(crate) fn load_and_merge(
    sources: &[ProjectionManifestSource],
) -> Result<Option<BTreeMap<String, ComponentEntry>>, WebUIError> {
    if sources.is_empty() {
        return Ok(None);
    }

    let mut components = BTreeMap::new();
    let mut artifacts = BTreeMap::new();
    for source in sources {
        let fragment = match source {
            ProjectionManifestSource::Path(path) => load_fragment_path(path)?,
            ProjectionManifestSource::Inline {
                manifest_path,
                json,
            } => load_fragment_bytes(manifest_path, json.as_bytes())?,
            ProjectionManifestSource::Prepared(prepared) => LoadedFragment {
                components: prepared.components.as_ref().clone(),
                artifacts: BTreeMap::new(),
            },
            ProjectionManifestSource::Pending(pending) => {
                let prepared = pending.wait()?;
                LoadedFragment {
                    components: prepared.components.as_ref().clone(),
                    artifacts: BTreeMap::new(),
                }
            }
        };
        for (tag, entry) in fragment.components {
            if components.insert(tag.clone(), entry).is_some() {
                return Err(projection_error(
                    codes::DUPLICATE_COMPONENT_OWNERSHIP,
                    format!("component <{tag}> is declared by more than one projection manifest"),
                    "Each custom-element tag must be owned by exactly one manifest fragment.",
                ));
            }
        }
        merge_artifacts(&mut artifacts, fragment.artifacts)?;
    }
    Ok(Some(components))
}

fn load_fragment_path(path: &Path) -> Result<LoadedFragment, WebUIError> {
    let metadata = std::fs::metadata(path).map_err(|_| manifest_unreadable(path))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(projection_error(
            codes::MANIFEST_TOO_LARGE,
            format!(
                "projection manifest '{}' is {} bytes, exceeding the 16 MiB limit",
                path.display(),
                metadata.len()
            ),
            "Projection manifests contain only hashes and exact key metadata; verify the bundler adapter output.",
        ));
    }

    let bytes = std::fs::read(path).map_err(|_| manifest_unreadable(path))?;
    load_fragment_bytes(path, &bytes)
}

fn load_fragment_bytes(path: &Path, bytes: &[u8]) -> Result<LoadedFragment, WebUIError> {
    if bytes.len() > MAX_MANIFEST_BYTES_USIZE {
        return Err(projection_error(
            codes::MANIFEST_TOO_LARGE,
            format!(
                "projection manifest '{}' is {} bytes, exceeding the 16 MiB limit",
                path.display(),
                bytes.len()
            ),
            "Projection manifests contain only hashes and exact key metadata; verify the bundler adapter output.",
        ));
    }
    let raw = ProjectionManifest::from_slice(bytes)
        .map_err(|error| shared_manifest_error(path, &error))?;

    let manifest_path = canonical_manifest_location(path)?;
    let root = resolve_root(&manifest_path, &raw.root)?;
    let inputs = validate_files(path, &root, raw.inputs, ArtifactKind::Input)?;
    let outputs = validate_files(path, &root, raw.outputs, ArtifactKind::Output)?;
    let components = validate_components(
        path,
        raw.components,
        &inputs.manifest_values,
        &outputs.manifest_values,
    )?;

    let mut artifacts = inputs.identities;
    merge_artifacts(&mut artifacts, outputs.identities)?;
    let runtime_components = components
        .into_iter()
        .map(|(tag, entry)| {
            (
                tag,
                ComponentEntry {
                    hydration_keys: entry.hydration_keys,
                    navigation_keys: entry.navigation_keys,
                },
            )
        })
        .collect();
    Ok(LoadedFragment {
        components: runtime_components,
        artifacts,
    })
}

fn canonical_manifest_location(path: &Path) -> Result<PathBuf, WebUIError> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Ok(canonical);
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent).map_err(|_| manifest_unreadable(path))?;
    let file_name = path.file_name().ok_or_else(|| manifest_unreadable(path))?;
    Ok(parent.join(file_name))
}

fn resolve_root(manifest_path: &Path, raw: &str) -> Result<PathBuf, WebUIError> {
    if raw.contains('\\') || raw.is_empty() {
        return Err(path_error(manifest_path, raw));
    }
    let parent_count = if raw == "." {
        0
    } else {
        let segments: Vec<&str> = raw.split('/').collect();
        if segments.len() > projection_manifest::MAX_ROOT_PARENTS
            || segments.iter().any(|segment| *segment != "..")
        {
            return Err(path_error(manifest_path, raw));
        }
        segments.len()
    };

    let mut root = manifest_path
        .parent()
        .ok_or_else(|| path_error(manifest_path, raw))?
        .to_path_buf();
    for _ in 0..parent_count {
        if !root.pop() {
            return Err(path_error(manifest_path, raw));
        }
    }
    let root = std::fs::canonicalize(&root).map_err(|_| manifest_unreadable(&root))?;
    if !manifest_path.starts_with(&root) {
        return Err(path_error(manifest_path, raw));
    }
    Ok(root)
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    Input,
    Output,
}

impl ArtifactKind {
    const fn stale_code(self) -> &'static str {
        match self {
            Self::Input => codes::STALE_INPUT,
            Self::Output => codes::STALE_OUTPUT,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

fn validate_files(
    manifest_path: &Path,
    root: &Path,
    declared: BTreeMap<String, String>,
    kind: ArtifactKind,
) -> Result<ValidatedFiles, WebUIError> {
    let mut values = BTreeMap::new();
    let mut identities = BTreeMap::new();

    for (key, expected_hash) in declared {
        if projection_manifest::is_virtual_key(&key) {
            if matches!(kind, ArtifactKind::Output) {
                return Err(projection_error(
                    codes::INVALID_HASH_FORMAT,
                    format!(
                        "disk projection manifest '{}' declares virtual output '{key}'",
                        manifest_path.display()
                    ),
                    "Disk manifests must hash exact emitted output bytes; virtual outputs are reserved for in-memory hosts.",
                ));
            }
            identities.insert(
                ArtifactIdentity::Virtual(key.clone()),
                expected_hash.clone(),
            );
            values.insert(key, expected_hash);
            continue;
        }

        if !projection_manifest::is_canonical_file_key(&key) {
            return Err(path_error(manifest_path, &key));
        }
        if !projection_manifest::is_sha256(&expected_hash) {
            return Err(invalid_hash(manifest_path, &key));
        }
        let joined = root.join(path_from_manifest_key(&key));
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|_| missing_artifact(manifest_path, kind, &key))?;
        if !canonical.starts_with(root) {
            return Err(path_error(manifest_path, &key));
        }
        let actual_hash =
            hash_file(&canonical).map_err(|_| missing_artifact(manifest_path, kind, &key))?;
        if actual_hash != expected_hash {
            return Err(projection_error(
                kind.stale_code(),
                format!(
                    "projection manifest '{}' is stale: {} '{}' changed",
                    manifest_path.display(),
                    kind.label(),
                    key
                ),
                "Rebuild the client bundle so manifest hashes match the files on disk.",
            ));
        }

        let identity = ArtifactIdentity::Physical(canonical);
        if identities.insert(identity, expected_hash.clone()).is_some() {
            return Err(invalid_field(
                manifest_path,
                &format!(
                    "multiple keys resolve to the same canonical {} file",
                    kind.label()
                ),
            ));
        }
        values.insert(key, expected_hash);
    }

    Ok(ValidatedFiles {
        manifest_values: values,
        identities,
    })
}

fn validate_components(
    manifest_path: &Path,
    declared: BTreeMap<String, ProjectionComponent>,
    inputs: &BTreeMap<String, String>,
    outputs: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ProjectionComponent>, WebUIError> {
    let mut validated = BTreeMap::new();
    for (tag, entry) in declared {
        if projection_manifest::is_virtual_key(&entry.module)
            || inputs
                .get(&entry.module)
                .is_none_or(|hash| hash == "virtual")
        {
            return Err(invalid_field(
                manifest_path,
                &format!(
                    "component <{tag}> module '{}' is not a physical inputs entry",
                    entry.module
                ),
            ));
        }
        for output in &entry.outputs {
            if outputs.get(output).is_none_or(|hash| hash == "virtual") {
                return Err(invalid_field(
                    manifest_path,
                    &format!("component <{tag}> references undeclared physical output '{output}'"),
                ));
            }
        }
        // Reinsert after disk-only checks; shared validation already proved
        // tag syntax, references, ordering, and hydration/navigation subset.
        validated.insert(tag, entry);
    }
    Ok(validated)
}

fn merge_artifacts(
    target: &mut BTreeMap<ArtifactIdentity, String>,
    source: BTreeMap<ArtifactIdentity, String>,
) -> Result<(), WebUIError> {
    for (identity, hash) in source {
        if let Some(existing) = target.get(&identity) {
            if existing != &hash {
                return Err(projection_error(
                    codes::CONFLICTING_HASH,
                    "projection manifest fragments observed one canonical artifact with conflicting hashes",
                    "Rebuild all fragments from one consistent source/output snapshot.",
                ));
            }
        } else {
            target.insert(identity, hash);
        }
    }
    Ok(())
}

/// Require exact metadata for every compiled scripted component.
pub(crate) fn validate_coverage(
    merged: &BTreeMap<String, ComponentEntry>,
    scripted_tags: &[&str],
) -> Result<(), WebUIError> {
    let mut missing: Vec<&str> = scripted_tags
        .iter()
        .copied()
        .filter(|tag| !merged.contains_key(*tag))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if missing.is_empty() {
        return Ok(());
    }

    let mut names = String::with_capacity(missing.len() * 16);
    for (index, tag) in missing.iter().enumerate() {
        if index > 0 {
            names.push_str(", ");
        }
        names.push('<');
        names.push_str(tag);
        names.push('>');
    }
    Err(projection_error(
        codes::MISSING_COVERAGE,
        format!(
            "scripted component(s) compiled into the protocol have no projection entry: {names}"
        ),
        "Bundle every compiled scripted component with a projection adapter, or provide its separate manifest fragment.",
    ))
}

/// Union client keys with Rust template roots.
pub(crate) fn union_keys(left: &[String], right: &[String]) -> Vec<String> {
    let mut keys = Vec::with_capacity(left.len() + right.len());
    keys.extend_from_slice(left);
    keys.extend_from_slice(right);
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format_digest(digest.finalize().as_slice()))
}

#[cfg(test)]
fn hash_bytes(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes).as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn path_from_manifest_key(value: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in value.split('/') {
        path.push(segment);
    }
    path
}

#[cold]
#[inline(never)]
fn projection_error(
    code: &'static str,
    title: impl Into<String>,
    help: impl Into<String>,
) -> WebUIError {
    let diagnostic = Diagnostic::error(title.into()).code(code).help(help.into());
    WebUIError::Projection(Box::new(diagnostic))
}

#[cold]
#[inline(never)]
fn manifest_unreadable(path: &Path) -> WebUIError {
    projection_error(
        codes::MANIFEST_UNREADABLE,
        format!(
            "projection manifest or declared file '{}' is missing or unreadable",
            path.display()
        ),
        "Run the client bundler successfully and pass the completed manifest path.",
    )
}

#[cold]
#[inline(never)]
fn shared_manifest_error(
    path: &Path,
    source: &projection_manifest::ProjectionManifestError,
) -> WebUIError {
    projection_error(
        source.code(),
        format!(
            "projection manifest '{}' is invalid: {source}",
            path.display()
        ),
        "Regenerate the manifest with a conformant projection adapter instead of editing it.",
    )
}

#[cold]
#[inline(never)]
fn invalid_field(path: &Path, detail: &str) -> WebUIError {
    projection_error(
        codes::INVALID_FIELD,
        format!(
            "projection manifest '{}' has an invalid field: {detail}",
            path.display()
        ),
        "Regenerate the manifest with a compatible @microsoft/webui projection adapter.",
    )
}

#[cold]
#[inline(never)]
fn invalid_hash(path: &Path, field: &str) -> WebUIError {
    projection_error(
        codes::INVALID_HASH_FORMAT,
        format!(
            "projection manifest '{}' has an invalid hash for '{field}'",
            path.display()
        ),
        "Use sha256:<64 lowercase hex> for physical files and virtual only with a virtual: key.",
    )
}

#[cold]
#[inline(never)]
fn path_error(path: &Path, field: &str) -> WebUIError {
    projection_error(
        codes::PATH_TRAVERSAL,
        format!(
            "projection manifest '{}' declares an unsafe or non-canonical path '{field}'",
            path.display()
        ),
        "Use a parent-only root and root-relative forward-slash file keys without . or .. segments.",
    )
}

#[cold]
#[inline(never)]
fn missing_artifact(path: &Path, kind: ArtifactKind, key: &str) -> WebUIError {
    projection_error(
        codes::MANIFEST_UNREADABLE,
        format!(
            "projection manifest '{}' references missing {} '{key}'",
            path.display(),
            kind.label()
        ),
        "Rebuild the client bundle and consume the manifest only after its atomic write completes.",
    )
}

/// Build the incompatible-plugin diagnostic.
#[cold]
#[inline(never)]
pub(crate) fn incompatible_plugin_error() -> WebUIError {
    projection_error(
        codes::INCOMPATIBLE_PLUGIN,
        "projection manifests require the WebUI plugin",
        "Select Plugin::WebUI or remove projection_manifests; default and FAST plugins preserve full state.",
    )
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::fs;
    use webui_protocol::projection_manifest::{ProjectionAdapter, ProjectionProducer};

    pub(crate) fn write_manifest(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    pub(crate) fn build_valid_manifest_json(
        dir: &Path,
        inputs: &[(&str, &str)],
        outputs: &[(&str, &str)],
        components: &[(&str, &str, &[&str], &[&str], &[&str])],
    ) -> String {
        let mut input_hashes = BTreeMap::new();
        for (name, content) in inputs {
            write_fixture(dir, name, content);
            input_hashes.insert((*name).to_string(), hash_bytes(content.as_bytes()));
        }
        let mut output_hashes = BTreeMap::new();
        for (name, content) in outputs {
            write_fixture(dir, name, content);
            output_hashes.insert((*name).to_string(), hash_bytes(content.as_bytes()));
        }
        let mut entries = BTreeMap::new();
        for (tag, module, component_outputs, hydration_keys, navigation_keys) in components {
            entries.insert(
                (*tag).to_string(),
                ProjectionComponent {
                    module: (*module).to_string(),
                    outputs: component_outputs
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    hydration_keys: hydration_keys
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    navigation_keys: navigation_keys
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                },
            );
        }
        let mut manifest = ProjectionManifest {
            schema: projection_manifest::SCHEMA_ID.to_string(),
            producer: ProjectionProducer {
                name: projection_manifest::PRODUCER_NAME.to_string(),
                version: "0.0.18".to_string(),
            },
            adapter: ProjectionAdapter {
                name: "test".to_string(),
                bundler: "test@1.0.0".to_string(),
            },
            root: ".".to_string(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            outputs: output_hashes,
            inputs: input_hashes,
            components: entries,
        };
        manifest.build_id = manifest.compute_build_id();
        serde_json::to_string(&manifest).unwrap()
    }

    fn write_fixture(dir: &Path, name: &str, content: &str) {
        let path = dir.join(path_from_manifest_key(name));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{build_valid_manifest_json, write_manifest};
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use webui_protocol::projection_manifest::{ProjectionAdapter, ProjectionProducer};

    #[test]
    fn loads_valid_exact_and_empty_surfaces() {
        let dir = TempDir::new().unwrap();
        let json = build_valid_manifest_json(
            dir.path(),
            &[("src/card.ts", "source"), ("src/static.ts", "static")],
            &[("dist/index.js", "output")],
            &[
                (
                    "demo-card",
                    "src/card.ts",
                    &["dist/index.js"],
                    &["count", "name"],
                    &["count", "name"],
                ),
                ("static-card", "src/static.ts", &["dist/index.js"], &[], &[]),
            ],
        );
        let manifest = write_manifest(dir.path(), "projection.json", &json);
        let merged = load_and_merge(&[manifest.into()]).unwrap().unwrap();
        assert_eq!(merged["demo-card"].hydration_keys, ["count", "name"]);
        assert_eq!(merged["demo-card"].navigation_keys, ["count", "name"]);
        assert!(merged["static-card"].hydration_keys.is_empty());
        assert!(merged["static-card"].navigation_keys.is_empty());
    }

    #[test]
    fn loads_inline_manifest_with_logical_path() {
        let dir = TempDir::new().unwrap();
        let json = build_valid_manifest_json(
            dir.path(),
            &[("src/card.ts", "source")],
            &[("dist/index.js", "output")],
            &[(
                "demo-card",
                "src/card.ts",
                &["dist/index.js"],
                &["name"],
                &["name"],
            )],
        );
        let source = ProjectionManifestSource::Inline {
            manifest_path: dir.path().join("projection.json"),
            json,
        };
        let merged = load_and_merge(&[source]).unwrap().unwrap();
        assert_eq!(merged["demo-card"].hydration_keys, ["name"]);
    }

    #[test]
    fn root_allows_dist_manifest_to_validate_src_and_dist_files() {
        let project = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::create_dir_all(project.path().join("dist")).unwrap();
        fs::write(project.path().join("src/card.ts"), "source").unwrap();
        fs::write(project.path().join("dist/index.js"), "output").unwrap();

        let input_hash = hash_file(&project.path().join("src/card.ts")).unwrap();
        let output_hash = hash_file(&project.path().join("dist/index.js")).unwrap();
        let inputs = BTreeMap::from([("src/card.ts".to_string(), input_hash)]);
        let outputs = BTreeMap::from([("dist/index.js".to_string(), output_hash)]);
        let components = BTreeMap::from([(
            "demo-card".to_string(),
            ProjectionComponent {
                module: "src/card.ts".to_string(),
                outputs: vec!["dist/index.js".to_string()],
                hydration_keys: vec!["name".to_string()],
                navigation_keys: vec!["name".to_string()],
            },
        )]);
        let mut projection = ProjectionManifest {
            schema: projection_manifest::SCHEMA_ID.to_string(),
            producer: ProjectionProducer {
                name: projection_manifest::PRODUCER_NAME.to_string(),
                version: "0.0.18".to_string(),
            },
            adapter: ProjectionAdapter {
                name: "test".to_string(),
                bundler: "test@1.0.0".to_string(),
            },
            root: "..".to_string(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            outputs,
            inputs,
            components,
        };
        projection.build_id = projection.compute_build_id();
        let json = serde_json::to_string(&projection).unwrap();
        let manifest = write_manifest(&project.path().join("dist"), "projection.json", &json);
        assert!(load_and_merge(&[manifest.into()]).is_ok());
    }

    #[test]
    fn rejects_stale_input_and_output() {
        let dir = TempDir::new().unwrap();
        let json = build_valid_manifest_json(
            dir.path(),
            &[("src/card.ts", "source")],
            &[("dist/index.js", "output")],
            &[(
                "demo-card",
                "src/card.ts",
                &["dist/index.js"],
                &["name"],
                &["name"],
            )],
        );
        let input_manifest = write_manifest(dir.path(), "input.json", &json);
        fs::write(dir.path().join("src/card.ts"), "changed").unwrap();
        assert_error_code(
            &load_and_merge(&[input_manifest.into()]).unwrap_err(),
            codes::STALE_INPUT,
        );

        let json = build_valid_manifest_json(
            dir.path(),
            &[("src/card.ts", "source")],
            &[("dist/index.js", "output")],
            &[(
                "demo-card",
                "src/card.ts",
                &["dist/index.js"],
                &["name"],
                &["name"],
            )],
        );
        let output_manifest = write_manifest(dir.path(), "output.json", &json);
        fs::write(dir.path().join("dist/index.js"), "changed").unwrap();
        assert_error_code(
            &load_and_merge(&[output_manifest.into()]).unwrap_err(),
            codes::STALE_OUTPUT,
        );
    }

    #[test]
    fn rejects_path_traversal_virtual_bypass_and_bad_references() {
        let dir = TempDir::new().unwrap();
        let base = format!(
            r#""schema":"{}","producer":{{"name":"{}","version":"0.0.18"}},"adapter":{{"name":"test","bundler":"test@1.0.0"}},"root":".","analysisHash":"sha256:{}","buildId":"sha256:{}""#,
            projection_manifest::SCHEMA_ID,
            projection_manifest::PRODUCER_NAME,
            "1".repeat(64),
            "2".repeat(64)
        );
        let traversal = write_manifest(
            dir.path(),
            "traversal.json",
            &format!(
                r#"{{{base},"outputs":{{}},"inputs":{{"../secret.ts":"sha256:{}"}},"components":{{}}}}"#,
                "3".repeat(64)
            ),
        );
        assert_error_code(
            &load_and_merge(&[traversal.into()]).unwrap_err(),
            codes::PATH_TRAVERSAL,
        );

        let virtual_output = write_manifest(
            dir.path(),
            "virtual-output.json",
            &format!(
                r#"{{{base},"outputs":{{"dist/index.js":"virtual"}},"inputs":{{}},"components":{{}}}}"#
            ),
        );
        assert_error_code(
            &load_and_merge(&[virtual_output.into()]).unwrap_err(),
            codes::INVALID_HASH_FORMAT,
        );

        let invalid_reference = write_manifest(
            dir.path(),
            "reference.json",
            &format!(
                r#"{{{base},"outputs":{{}},"inputs":{{}},"components":{{"demo-card":{{"module":"src/card.ts","outputs":["dist/index.js"],"hydrationKeys":[],"navigationKeys":[]}}}}}}"#
            ),
        );
        assert_error_code(
            &load_and_merge(&[invalid_reference.into()]).unwrap_err(),
            codes::INVALID_FIELD,
        );
    }

    #[test]
    fn rejects_duplicate_json_keys_and_duplicate_component_ownership() {
        let dir = TempDir::new().unwrap();
        let duplicate_key = write_manifest(
            dir.path(),
            "duplicate-key.json",
            r#"{"schema":"webui.state-projection/v1","schema":"webui.state-projection/v1"}"#,
        );
        assert_error_code(
            &load_and_merge(&[duplicate_key.into()]).unwrap_err(),
            codes::INVALID_FIELD,
        );

        let json = build_valid_manifest_json(
            dir.path(),
            &[("src/card.ts", "source")],
            &[("dist/index.js", "output")],
            &[(
                "demo-card",
                "src/card.ts",
                &["dist/index.js"],
                &["name"],
                &["name"],
            )],
        );
        let first = write_manifest(dir.path(), "first.json", &json);
        let second = write_manifest(dir.path(), "second.json", &json);
        assert_error_code(
            &load_and_merge(&[first.into(), second.into()]).unwrap_err(),
            codes::DUPLICATE_COMPONENT_OWNERSHIP,
        );
    }

    #[test]
    fn same_relative_names_under_different_roots_do_not_conflict() {
        let first_dir = TempDir::new().unwrap();
        let first_json = build_valid_manifest_json(
            first_dir.path(),
            &[("src/card.ts", "first")],
            &[("dist/index.js", "first-output")],
            &[("first-card", "src/card.ts", &["dist/index.js"], &[], &[])],
        );
        let first = write_manifest(first_dir.path(), "projection.json", &first_json);

        let second_dir = TempDir::new().unwrap();
        let second_json = build_valid_manifest_json(
            second_dir.path(),
            &[("src/card.ts", "second")],
            &[("dist/index.js", "second-output")],
            &[("second-card", "src/card.ts", &["dist/index.js"], &[], &[])],
        );
        let second = write_manifest(second_dir.path(), "projection.json", &second_json);

        let merged = load_and_merge(&[first.into(), second.into()])
            .unwrap()
            .unwrap();
        assert!(merged.contains_key("first-card"));
        assert!(merged.contains_key("second-card"));
    }

    #[test]
    fn validates_coverage_and_key_union() {
        let merged = BTreeMap::from([(
            "demo-card".to_string(),
            ComponentEntry {
                hydration_keys: vec!["client".to_string()],
                navigation_keys: vec!["client".to_string()],
            },
        )]);
        assert!(validate_coverage(&merged, &["demo-card"]).is_ok());
        assert_error_code(
            &validate_coverage(&merged, &["demo-card", "missing-card"]).unwrap_err(),
            codes::MISSING_COVERAGE,
        );
        assert_eq!(
            union_keys(
                &["client".to_string(), "shared".to_string()],
                &["server".to_string(), "shared".to_string()]
            ),
            ["client", "server", "shared"]
        );
    }

    #[test]
    fn rejects_oversize_manifest_before_parsing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("oversize.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_MANIFEST_BYTES + 1).unwrap();
        assert_error_code(
            &load_and_merge(&[path.into()]).unwrap_err(),
            codes::MANIFEST_TOO_LARGE,
        );
    }

    fn assert_error_code(error: &WebUIError, expected: &str) {
        let WebUIError::Projection(diagnostic) = error else {
            panic!("expected projection error, got {error:?}");
        };
        assert_eq!(diagnostic.error_code(), Some(expected));
    }
}
