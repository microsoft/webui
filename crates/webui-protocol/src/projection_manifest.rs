// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared state-projection manifest schema and structural validation.

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Component, Path};
use thiserror::Error;

/// Stable manifest diagnostic codes shared by all hosts.
pub mod codes {
    pub const UNSUPPORTED_SCHEMA: &str = "PROJ-M002";
    pub const BUILD_ID_MISMATCH: &str = "PROJ-M005";
    pub const INVALID_JSON: &str = "PROJ-M008";
    pub const INVALID_FIELD: &str = "PROJ-M009";
    pub const TOO_MANY_COMPONENTS: &str = "PROJ-S002";
    pub const PATH_TRAVERSAL: &str = "PROJ-S003";
    pub const INVALID_HASH_FORMAT: &str = "PROJ-S004";
}

/// Current projection manifest schema identifier.
pub const SCHEMA_ID: &str = "webui.state-projection/v1";
/// Required producer identity.
pub const PRODUCER_NAME: &str = "@microsoft/webui/projection.js";
/// Maximum component entries in one fragment.
pub const MAX_COMPONENT_COUNT: usize = 65_535;
/// Maximum parent segments in `root`.
pub const MAX_ROOT_PARENTS: usize = 32;

/// A structurally invalid projection manifest.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ProjectionManifestError {
    code: &'static str,
    message: String,
}

impl ProjectionManifestError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

/// Versioned projection manifest fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionManifest {
    /// Versioned schema identifier.
    pub schema: String,
    /// Producer package identity.
    pub producer: ProjectionProducer,
    /// Adapter and bundler identity.
    pub adapter: ProjectionAdapter,
    /// Build root relative to the manifest directory.
    pub root: String,
    /// Hash of normalized graph edges and output membership.
    pub analysis_hash: String,
    /// Deterministic hash of the complete manifest proof.
    pub build_id: String,
    /// Emitted output path/virtual ID to content hash.
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub outputs: BTreeMap<String, String>,
    /// Input path/virtual ID to content hash.
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub inputs: BTreeMap<String, String>,
    /// Exact component surfaces keyed by custom-element tag.
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub components: BTreeMap<String, ProjectionComponent>,
}

/// Manifest producer identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionProducer {
    /// Producer package name.
    pub name: String,
    /// Producer package version.
    pub version: String,
}

/// Bundler adapter identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAdapter {
    /// Adapter name.
    pub name: String,
    /// Bundler name and version.
    pub bundler: String,
}

/// One shipped component's exact state surfaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionComponent {
    /// Defining input module key.
    pub module: String,
    /// Output keys containing the component.
    pub outputs: Vec<String>,
    /// Exact initial `@observable` keys.
    pub hydration_keys: Vec<String>,
    /// Exact navigation `@observable + @attr` keys.
    pub navigation_keys: Vec<String>,
}

impl ProjectionManifest {
    /// Parse and structurally validate one JSON manifest.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ProjectionManifestError> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(parse_error)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate schema, hashes, references, ordering, and deterministic ID.
    pub fn validate(&self) -> Result<(), ProjectionManifestError> {
        if self.schema != SCHEMA_ID {
            return Err(error(
                codes::UNSUPPORTED_SCHEMA,
                format!("unsupported projection schema '{}'", self.schema),
            ));
        }
        if self.producer.name != PRODUCER_NAME || self.producer.version.is_empty() {
            return Err(invalid_field(
                "producer must identify @microsoft/webui/projection.js with a non-empty version",
            ));
        }
        if self.adapter.name.is_empty() || self.adapter.bundler.is_empty() {
            return Err(invalid_field(
                "adapter.name and adapter.bundler must be non-empty",
            ));
        }
        if !is_canonical_root(&self.root) {
            return Err(error(
                codes::PATH_TRAVERSAL,
                "root must be '.' or at most 32 parent-only segments",
            ));
        }
        if !is_sha256(&self.analysis_hash) || !is_sha256(&self.build_id) {
            return Err(error(
                codes::INVALID_HASH_FORMAT,
                "analysisHash and buildId must be lowercase SHA-256 values",
            ));
        }
        if self.components.len() > MAX_COMPONENT_COUNT {
            return Err(error(
                codes::TOO_MANY_COMPONENTS,
                format!(
                    "manifest declares {} components, exceeding {}",
                    self.components.len(),
                    MAX_COMPONENT_COUNT
                ),
            ));
        }
        validate_hash_map(&self.inputs)?;
        validate_hash_map(&self.outputs)?;
        validate_components(self)?;
        if self.compute_build_id() != self.build_id {
            return Err(error(
                codes::BUILD_ID_MISMATCH,
                "buildId does not match the canonical manifest contents",
            ));
        }
        Ok(())
    }

    /// Recompute the deterministic cross-language build ID.
    #[must_use]
    pub fn compute_build_id(&self) -> String {
        let mut canonical = String::with_capacity(512);
        append_record(&mut canonical, "schema", &[SCHEMA_ID]);
        append_record(
            &mut canonical,
            "producer",
            &[&self.producer.name, &self.producer.version],
        );
        append_record(
            &mut canonical,
            "adapter",
            &[&self.adapter.name, &self.adapter.bundler],
        );
        append_record(&mut canonical, "root", &[&self.root]);
        append_record(&mut canonical, "analysis", &[&self.analysis_hash]);

        let input_count = self.inputs.len().to_string();
        append_record(&mut canonical, "inputs", &[&input_count]);
        for (path, hash) in &self.inputs {
            append_record(&mut canonical, "input", &[path, hash]);
        }

        let output_count = self.outputs.len().to_string();
        append_record(&mut canonical, "outputs", &[&output_count]);
        for (path, hash) in &self.outputs {
            append_record(&mut canonical, "output", &[path, hash]);
        }

        let component_count = self.components.len().to_string();
        append_record(&mut canonical, "components", &[&component_count]);
        for (tag, component) in &self.components {
            let output_count = component.outputs.len().to_string();
            let hydration_count = component.hydration_keys.len().to_string();
            let navigation_count = component.navigation_keys.len().to_string();
            let mut fields = Vec::with_capacity(
                5 + component.outputs.len()
                    + component.hydration_keys.len()
                    + component.navigation_keys.len(),
            );
            fields.push(tag.as_str());
            fields.push(component.module.as_str());
            fields.push(output_count.as_str());
            fields.extend(component.outputs.iter().map(String::as_str));
            fields.push(hydration_count.as_str());
            fields.extend(component.hydration_keys.iter().map(String::as_str));
            fields.push(navigation_count.as_str());
            fields.extend(component.navigation_keys.iter().map(String::as_str));
            append_record(&mut canonical, "component", &fields);
        }
        hash_bytes(canonical.as_bytes())
    }
}

fn validate_hash_map(values: &BTreeMap<String, String>) -> Result<(), ProjectionManifestError> {
    for (path, hash) in values {
        if is_virtual_key(path) {
            if !is_canonical_virtual_key(path) || hash != "virtual" {
                return Err(error(
                    codes::INVALID_HASH_FORMAT,
                    format!("invalid virtual/hash pairing for '{path}'"),
                ));
            }
        } else {
            if !is_canonical_file_key(path) {
                return Err(error(
                    codes::PATH_TRAVERSAL,
                    format!("unsafe or non-canonical manifest path '{path}'"),
                ));
            }
            if !is_sha256(hash) {
                return Err(error(
                    codes::INVALID_HASH_FORMAT,
                    format!("invalid physical-file hash for '{path}'"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_components(manifest: &ProjectionManifest) -> Result<(), ProjectionManifestError> {
    for (tag, component) in &manifest.components {
        if !is_component_tag(tag) {
            return Err(invalid_field(&format!(
                "component tag '{tag}' is not lowercase custom-element syntax"
            )));
        }
        if !manifest.inputs.contains_key(&component.module) {
            return Err(invalid_field(&format!(
                "component <{tag}> module '{}' is absent from inputs",
                component.module
            )));
        }
        if component.outputs.is_empty() || !is_sorted_unique(&component.outputs) {
            return Err(invalid_field(&format!(
                "component <{tag}> outputs must be non-empty, sorted, and unique"
            )));
        }
        if component
            .outputs
            .iter()
            .any(|output| !manifest.outputs.contains_key(output))
        {
            return Err(invalid_field(&format!(
                "component <{tag}> references an undeclared output"
            )));
        }
        if !valid_keys(&component.hydration_keys)
            || !valid_keys(&component.navigation_keys)
            || component
                .hydration_keys
                .iter()
                .any(|key| component.navigation_keys.binary_search(key).is_err())
        {
            return Err(invalid_field(&format!(
                "component <{tag}> key surfaces are invalid or navigation is not a hydration superset"
            )));
        }
    }
    Ok(())
}

fn valid_keys(values: &[String]) -> bool {
    is_sorted_unique(values)
        && values
            .iter()
            .all(|value| !value.is_empty() && !has_control(value))
}

fn deserialize_unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_map(UniqueMapVisitor(PhantomData))
}

struct UniqueMapVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for UniqueMapVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = BTreeMap<String, T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object with unique string keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = access.next_entry::<String, T>()? {
            if values.insert(key.clone(), value).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate object key '{key}'"
                )));
            }
        }
        Ok(values)
    }
}

fn parse_error(source: serde_json::Error) -> ProjectionManifestError {
    let code = if source.classify() == serde_json::error::Category::Syntax
        || source.classify() == serde_json::error::Category::Eof
    {
        codes::INVALID_JSON
    } else {
        codes::INVALID_FIELD
    };
    error(
        code,
        format!("projection manifest could not be parsed: {source}"),
    )
}

fn invalid_field(message: &str) -> ProjectionManifestError {
    error(codes::INVALID_FIELD, message)
}

fn error(code: &'static str, message: impl Into<String>) -> ProjectionManifestError {
    ProjectionManifestError {
        code,
        message: message.into(),
    }
}

fn append_record(output: &mut String, label: &str, fields: &[&str]) {
    output.push_str(label);
    for field in fields {
        output.push_str(&field.len().to_string());
        output.push(':');
        output.push_str(field);
    }
    output.push('\n');
}

fn hash_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

/// Whether a value is exactly `sha256:` plus 64 lowercase hex characters.
#[must_use]
pub fn is_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(is_lower_hex)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Whether a manifest key identifies a virtual module/output.
#[must_use]
pub fn is_virtual_key(value: &str) -> bool {
    value.starts_with("virtual:")
}

fn is_canonical_virtual_key(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("virtual:") else {
        return false;
    };
    !hex.is_empty() && hex.len().is_multiple_of(2) && hex.bytes().all(is_lower_hex)
}

/// Whether a physical manifest key is safe and canonical.
#[must_use]
pub fn is_canonical_file_key(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with("./")
        || value.contains('\\')
        || has_control(value)
    {
        return false;
    }
    let mut saw_segment = false;
    for component in Path::new(value).components() {
        match component {
            Component::Normal(segment) if !segment.is_empty() => saw_segment = true,
            _ => return false,
        }
    }
    saw_segment && value.split('/').all(|segment| !segment.is_empty())
}

fn is_canonical_root(value: &str) -> bool {
    if value == "." {
        return true;
    }
    let segments: Vec<&str> = value.split('/').collect();
    !segments.is_empty()
        && segments.len() <= MAX_ROOT_PARENTS
        && segments.iter().all(|segment| *segment == "..")
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn is_component_tag(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 3 || !bytes.contains(&b'-') || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(*byte, b'-' | b'.' | b'_')
    })
}

fn is_sorted_unique(values: &[String]) -> bool {
    values
        .windows(2)
        .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_language_build_id_matches() {
        let mut manifest = ProjectionManifest {
            schema: SCHEMA_ID.to_string(),
            producer: ProjectionProducer {
                name: PRODUCER_NAME.to_string(),
                version: "0.0.18".to_string(),
            },
            adapter: ProjectionAdapter {
                name: "esbuild".to_string(),
                bundler: "esbuild@0.28.1".to_string(),
            },
            root: "..".to_string(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            outputs: BTreeMap::from([(
                "dist/a.js".to_string(),
                format!("sha256:{}", "3".repeat(64)),
            )]),
            inputs: BTreeMap::from([(
                "src/a.ts".to_string(),
                format!("sha256:{}", "2".repeat(64)),
            )]),
            components: BTreeMap::from([(
                "a-card".to_string(),
                ProjectionComponent {
                    module: "src/a.ts".to_string(),
                    outputs: vec!["dist/a.js".to_string()],
                    hydration_keys: vec!["displayValue".to_string()],
                    navigation_keys: vec!["displayValue".to_string(), "é".to_string()],
                },
            )]),
        };
        manifest.build_id = manifest.compute_build_id();
        assert_eq!(
            manifest.build_id,
            "sha256:8319202a060626c39cce76df50197c92dee27aab29d601161183c188204d7c18"
        );
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn duplicate_json_key_is_rejected() {
        let error = ProjectionManifest::from_slice(
            br#"{"schema":"webui.state-projection/v1","schema":"duplicate"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code(), codes::INVALID_FIELD);
    }
}
