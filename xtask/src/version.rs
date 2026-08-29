// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Atomic version bumping across all Cargo.toml and package.json files.
//!
//! The update pipeline has two phases:
//!
//! 1. **Discovery** – [`discover_targets`] walks the workspace and collects
//!    every file whose version must be bumped into a `Vec<VersionTarget>`.
//! 2. **Execution** – [`run`] iterates the targets and applies the appropriate
//!    updater via [`execute_update`], logging each result uniformly.

use crate::release_version::ReleaseVersion;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const PYTHON_CARGO_PACKAGE: &str = "microsoft-webui-python";

// ── Target model ────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum VersionFormat {
    Release,
    Python,
    PythonCargo,
}

struct VersionValues {
    release: String,
    python: String,
    python_cargo: String,
    npm_dist_tag: String,
}

impl VersionValues {
    fn new(parsed: ReleaseVersion) -> Self {
        Self {
            release: parsed.to_string(),
            python: parsed.python_version(),
            python_cargo: parsed.python_cargo_version(),
            npm_dist_tag: parsed.npm_dist_tag(),
        }
    }

    fn get(&self, format: VersionFormat) -> &str {
        match format {
            VersionFormat::Release => &self.release,
            VersionFormat::Python => &self.python,
            VersionFormat::PythonCargo => &self.python_cargo,
        }
    }
}

/// How to update the version inside a given file.
enum UpdateStrategy {
    /// Replace `version = "..."` inside a TOML `[section]`.
    TomlSection {
        section: &'static str,
        format: VersionFormat,
    },
    /// Replace `"version"` (and `@microsoft/webui-*` optional deps) in a
    /// `package.json`.
    PackageJson,
    /// Replace first-party package versions in `Cargo.lock`.
    CargoLock,
    /// Update one crate manifest's package and first-party dependency versions.
    CrateManifest {
        package_format: Option<VersionFormat>,
    },
    /// Replace `<Version>…</Version>` in a .NET `Directory.Build.props`.
    DotnetProps,
    /// Replace PEP 621 project version metadata in `pyproject.toml`.
    Pyproject(VersionFormat),
    /// Replace a string constant in a Rust source file.
    RustConst {
        name: &'static str,
        format: VersionFormat,
    },
}

/// A single file whose version must be updated.
struct VersionTarget {
    path: PathBuf,
    strategy: UpdateStrategy,
    /// When `true`, an `Ok(false)` result (no version field found) is treated
    /// as an error rather than a silent skip.
    required: bool,
}

/// Collect every file in the workspace that contains a version to bump.
fn discover_targets(root: &Path) -> Vec<VersionTarget> {
    let mut targets = Vec::new();

    // Workspace Cargo.toml (must contain [workspace.package].version)
    targets.push(VersionTarget {
        path: root.join("Cargo.toml"),
        strategy: UpdateStrategy::TomlSection {
            section: "[workspace.package]",
            format: VersionFormat::Release,
        },
        required: true,
    });

    // Root package.json
    targets.push(VersionTarget {
        path: root.join("package.json"),
        strategy: UpdateStrategy::PackageJson,
        required: false,
    });

    // Cargo.lock first-party package versions
    let cargo_lock = root.join("Cargo.lock");
    if cargo_lock.exists() {
        targets.push(VersionTarget {
            path: cargo_lock,
            strategy: UpdateStrategy::CargoLock,
            required: true,
        });
    }

    // dotnet/Directory.Build.props (only if present)
    let dotnet_props = root.join("dotnet").join("Directory.Build.props");
    if dotnet_props.exists() {
        targets.push(VersionTarget {
            path: dotnet_props,
            strategy: UpdateStrategy::DotnetProps,
            required: true,
        });
    }

    // Python uses a PEP 440 post release for SemVer hotfix prereleases.
    let pyproject = root
        .join("crates")
        .join("webui-python")
        .join("pyproject.toml");
    if pyproject.exists() {
        targets.push(VersionTarget {
            path: pyproject,
            strategy: UpdateStrategy::Pyproject(VersionFormat::Python),
            required: true,
        });
    }
    let python_version_source = root
        .join("crates")
        .join("webui-python")
        .join("src")
        .join("version.rs");
    if python_version_source.exists() {
        targets.push(VersionTarget {
            path: python_version_source,
            strategy: UpdateStrategy::RustConst {
                name: "PYTHON_PACKAGE_VERSION",
                format: VersionFormat::Python,
            },
            required: true,
        });
    }

    // crates/*/Cargo.toml - inter-crate dependencies and Python package metadata.
    let python_cargo_manifest = root.join("crates").join("webui-python").join("Cargo.toml");
    for toml in find_crate_cargo_tomls(root) {
        let package_format = (toml == python_cargo_manifest).then_some(VersionFormat::PythonCargo);
        targets.push(VersionTarget {
            path: toml,
            strategy: UpdateStrategy::CrateManifest { package_format },
            required: false,
        });
    }

    // packages/**/package.json
    for pkg in find_package_jsons(root) {
        targets.push(VersionTarget {
            path: pkg,
            strategy: UpdateStrategy::PackageJson,
            required: false,
        });
    }

    targets
}

/// Dispatch a version update to the right updater function.
fn execute_update(target: &VersionTarget, versions: &VersionValues) -> Result<bool, String> {
    match &target.strategy {
        UpdateStrategy::TomlSection { section, format } => {
            update_toml_section_version(&target.path, section, versions.get(*format))
        }
        UpdateStrategy::PackageJson => {
            update_package_json(&target.path, &versions.release, &versions.npm_dist_tag)
        }
        UpdateStrategy::CargoLock => {
            update_cargo_lock_versions(&target.path, &versions.release, &versions.python_cargo)
        }
        UpdateStrategy::CrateManifest { package_format } => update_crate_manifest(
            &target.path,
            package_format.map(|format| versions.get(format)),
            &versions.release,
        ),
        UpdateStrategy::DotnetProps => update_dotnet_version(&target.path, &versions.release),
        UpdateStrategy::Pyproject(format) => {
            update_pyproject_version(&target.path, versions.get(*format))
        }
        UpdateStrategy::RustConst { name, format } => {
            update_rust_string_const(&target.path, name, versions.get(*format))
        }
    }
}

fn apply_update(result: Result<bool, String>, target: &VersionTarget) -> Result<bool, String> {
    match result {
        Ok(true) => Ok(true),
        Ok(false) if target.required => Err(format!(
            "No version field found in {}",
            target.path.display()
        )),
        Ok(false) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Update `version = "..."` inside a specific TOML section of a file.
fn update_toml_section_version(path: &Path, section: &str, version: &str) -> Result<bool, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut result = String::with_capacity(content.len());
    let mut in_section = false;
    let mut updated = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            in_section = true;
        } else if trimmed.starts_with('[') {
            in_section = false;
        }
        if in_section && trimmed.starts_with("version") && trimmed.contains('=') && !updated {
            result.push_str("version = \"");
            result.push_str(version);
            result.push_str("\"\n");
            updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if updated {
        fs::write(path, &result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(updated)
}

/// Update first-party package versions in `Cargo.lock`.
fn update_cargo_lock_versions(
    path: &Path,
    release_version: &str,
    python_cargo_version: &str,
) -> Result<bool, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut result = String::with_capacity(content.len());
    let mut package_version = None;
    let mut updated = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            package_version = None;
        } else if let Some(name) = trimmed
            .strip_prefix("name = \"")
            .and_then(|value| value.strip_suffix('"'))
        {
            package_version = match name {
                PYTHON_CARGO_PACKAGE => Some(python_cargo_version),
                _ if name.starts_with("microsoft-webui") => Some(release_version),
                _ => None,
            };
        }

        if let Some(version) = package_version.filter(|_| trimmed.starts_with("version = \"")) {
            if let Some(new_line) = replace_inline_version(line, version) {
                result.push_str(&new_line);
                result.push('\n');
                updated = true;
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if updated {
        fs::write(path, result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(updated)
}

/// Update `version = "..."` in `[workspace.package]` of root Cargo.toml.
///
/// Used by tests only — production code goes through [`execute_update`].
#[cfg(test)]
fn update_cargo_workspace_version(root: &Path, version: &str) -> Result<(), String> {
    let cargo_path = root.join("Cargo.toml");
    if !update_toml_section_version(&cargo_path, "[workspace.package]", version)? {
        return Err("Could not find version in [workspace.package]".to_string());
    }
    Ok(())
}

/// Replace the `version = "..."` portion of a dependency line.
fn replace_inline_version(line: &str, new_version: &str) -> Option<String> {
    let version_key = "version = \"";
    let start = line.find(version_key)?;
    let value_start = start + version_key.len();
    let end = line[value_start..].find('"')?;

    let mut result = String::with_capacity(line.len());
    result.push_str(&line[..value_start]);
    result.push_str(new_version);
    result.push_str(&line[value_start + end..]);
    Some(result)
}

/// Find all `Cargo.toml` files under `crates/`.
fn find_crate_cargo_tomls(root: &Path) -> Vec<PathBuf> {
    let crates_dir = root.join("crates");
    let mut results = Vec::new();

    if !crates_dir.exists() {
        return results;
    }

    if let Ok(entries) = fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let toml = entry.path().join("Cargo.toml");
            if toml.is_file() {
                results.push(toml);
            }
        }
    }

    results.sort();
    results
}

/// Update `version = "..."` in inter-crate dependency lines of a crate's Cargo.toml.
fn update_crate_dep_versions(path: &Path, version: &str) -> Result<bool, String> {
    let release = ReleaseVersion::parse(version)
        .ok_or_else(|| format!("Invalid release version for {}: {version}", path.display()))?;
    let exact_version = (!release.is_stable()).then(|| format!("={version}"));
    let dependency_version = exact_version.as_deref().unwrap_or(version);
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut result = String::with_capacity(content.len());
    let mut changed = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("microsoft-webui")
            && trimmed.contains("path")
            && trimmed.contains("version")
        {
            if let Some(new_line) = replace_inline_version(line, dependency_version) {
                result.push_str(&new_line);
                result.push('\n');
                changed = true;
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if changed {
        fs::write(path, result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(changed)
}

fn update_crate_manifest(
    path: &Path,
    package_version: Option<&str>,
    dependency_version: &str,
) -> Result<bool, String> {
    let package_changed = match package_version {
        Some(version) => {
            if !update_toml_section_version(path, "[package]", version)? {
                return Err(format!(
                    "Could not find [package].version in {}",
                    path.display()
                ));
            }
            true
        }
        None => false,
    };
    let dependencies_changed = update_crate_dep_versions(path, dependency_version)?;
    Ok(package_changed || dependencies_changed)
}

/// Replace the value of the first occurrence of a JSON field in raw content.
///
/// Finds `"field": "old"` and produces `"field": "new"`, preserving all formatting.
fn replace_first_json_field(content: &str, field: &str, new_value: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let key_pos = content.find(&key)?;
    let after_key = key_pos + key.len();

    let colon_offset = content[after_key..].find(':')?;
    let after_colon = after_key + colon_offset + 1;

    let open_quote = content[after_colon..].find('"')?;
    let value_start = after_colon + open_quote + 1;

    let close_quote = content[value_start..].find('"')?;
    let value_end = value_start + close_quote;

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..value_start]);
    result.push_str(new_value);
    result.push_str(&content[value_end..]);
    Some(result)
}

fn replace_json_object_string_field(
    content: &str,
    object: &str,
    field: &str,
    new_value: &str,
) -> Option<String> {
    let object_key = format!("\"{object}\"");
    let object_start = content.find(&object_key)?;
    let updated_suffix = replace_first_json_field(&content[object_start..], field, new_value)?;
    let mut result =
        String::with_capacity(content.len() - content[object_start..].len() + updated_suffix.len());
    result.push_str(&content[..object_start]);
    result.push_str(&updated_suffix);
    Some(result)
}

/// Update version in a package.json file. Also updates optionalDependencies
/// that reference @microsoft/webui-* packages and the registry dist-tag for
/// public WebUI packages.
///
/// Uses serde_json to read the structure, then performs surgical string
/// replacement so only the version values change — all formatting is preserved.
fn update_package_json(path: &Path, version: &str, npm_dist_tag: &str) -> Result<bool, String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;

    let mut result = content;
    let mut changed = false;

    // Replace top-level "version" field value
    if obj.contains_key("version") {
        if let Some(updated) = replace_first_json_field(&result, "version", version) {
            result = updated;
            changed = true;
        }
    }

    // Replace @microsoft/webui-* version values in optionalDependencies.
    // Skip workspace: protocol values (pnpm resolves them at publish time).
    if let Some(deps) = obj.get("optionalDependencies").and_then(|v| v.as_object()) {
        for (key, val) in deps {
            if key.starts_with("@microsoft/webui") {
                let current = val.as_str().unwrap_or_default();
                if !current.starts_with("workspace:") {
                    if let Some(updated) = replace_first_json_field(&result, key, version) {
                        result = updated;
                        changed = true;
                    }
                }
            }
        }
    }

    let is_public_webui_package = obj
        .get("name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|name| name.starts_with("@microsoft/webui"))
        && obj.get("private").and_then(serde_json::Value::as_bool) != Some(true);
    if is_public_webui_package {
        let configured_tag = obj
            .get("publishConfig")
            .and_then(serde_json::Value::as_object)
            .and_then(|config| config.get("tag"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                format!(
                    "{} must define a string publishConfig.tag so hotfixes cannot acquire the \
                     latest npm dist-tag",
                    path.display()
                )
            })?;
        if configured_tag != npm_dist_tag {
            result =
                replace_json_object_string_field(&result, "publishConfig", "tag", npm_dist_tag)
                    .ok_or_else(|| {
                        format!("Could not update publishConfig.tag in {}", path.display())
                    })?;
            changed = true;
        }
    }

    if changed {
        fs::write(path, &result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(changed)
}

/// Update `<Version>...</Version>` in a .NET `Directory.Build.props` file.
fn update_dotnet_version(path: &Path, version: &str) -> Result<bool, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let Some(start) = content.find("<Version>") else {
        return Err(format!(
            "Could not find <Version> tag in {}",
            path.display()
        ));
    };
    let tag_value_start = start + "<Version>".len();
    let Some(end) = content[tag_value_start..].find("</Version>") else {
        return Err(format!(
            "Could not find closing </Version> tag in {}",
            path.display()
        ));
    };

    let old_version = &content[tag_value_start..tag_value_start + end];
    if old_version == version {
        return Ok(true);
    }

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..tag_value_start]);
    result.push_str(version);
    result.push_str(&content[tag_value_start + end..]);

    fs::write(path, result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(true)
}

/// Update the PEP 621 version in the Python package's `pyproject.toml`.
fn update_pyproject_version(path: &Path, version: &str) -> Result<bool, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut result = String::with_capacity(content.len());
    let mut in_project = false;
    let mut updated = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[project]" {
            in_project = true;
        } else if trimmed.starts_with('[') {
            in_project = false;
        }

        if in_project
            && !updated
            && (trimmed.starts_with("version = ") || trimmed == r#"dynamic = ["version"]"#)
        {
            result.push_str("version = \"");
            result.push_str(version);
            result.push_str("\"\n");
            updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if updated {
        fs::write(path, result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(updated)
}

/// Update a `pub(crate) const NAME: &str = "value";` declaration.
fn update_rust_string_const(path: &Path, name: &str, version: &str) -> Result<bool, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let prefix = format!("pub(crate) const {name}: &str = \"");
    let Some(start) = content.find(&prefix) else {
        return Ok(false);
    };
    let value_start = start + prefix.len();
    let Some(end) = content[value_start..].find('"') else {
        return Err(format!(
            "Could not find the closing quote for {name} in {}",
            path.display()
        ));
    };

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..value_start]);
    result.push_str(version);
    result.push_str(&content[value_start + end..]);
    fs::write(path, result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(true)
}

/// Find all package.json files under `packages/`.
fn find_package_jsons(root: &Path) -> Vec<PathBuf> {
    let packages_dir = root.join("packages");
    let mut results = Vec::new();

    if !packages_dir.exists() {
        return results;
    }

    let mut stack = vec![packages_dir];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                if name == "node_modules" || name == ".git" {
                    continue;
                }
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "package.json") {
                results.push(path);
            }
        }
    }

    results
}

/// Read the current workspace version from root `Cargo.toml`.
///
/// Parses `[workspace.package].version` and returns the semver string.
pub fn read_version() -> Result<String, String> {
    let root = crate::util::workspace_root()?;
    let cargo_path = root.join("Cargo.toml");
    read_toml_section_version(&cargo_path, "[workspace.package]")
}

fn read_toml_section_version(path: &Path, section: &str) -> Result<String, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            in_section = true;
        } else if trimmed.starts_with('[') {
            in_section = false;
        }
        if in_section {
            if let Some(version) = quoted_assignment(trimmed, "version") {
                return Ok(version.to_string());
            }
        }
    }

    Err(format!(
        "Could not find version in {section} of {}",
        path.display()
    ))
}

/// Return the PEP 440 package version after verifying Python metadata is synchronized.
pub(crate) fn python_package_version(root: &Path, release_version: &str) -> Result<String, String> {
    let release = ReleaseVersion::parse(release_version)
        .ok_or_else(|| format!("Unsupported release version `{release_version}`"))?;
    let expected = release.python_version();
    let expected_cargo = release.python_cargo_version();
    let cargo_path = root.join("crates").join("webui-python").join("Cargo.toml");
    let cargo_version = read_toml_section_version(&cargo_path, "[package]")?;
    if cargo_version != expected_cargo {
        return Err(format!(
            "{} has version {cargo_version}, expected {expected_cargo}.\n  help: run `cargo xtask \
             version {release_version}`",
            cargo_path.display()
        ));
    }

    let pyproject_path = root
        .join("crates")
        .join("webui-python")
        .join("pyproject.toml");
    let configured = read_pyproject_version(&pyproject_path)?;
    if configured != expected {
        return Err(format!(
            "{} has version {configured}, expected {expected}.\n  help: run `cargo xtask version \
             {release_version}`",
            pyproject_path.display()
        ));
    }

    let source_path = root
        .join("crates")
        .join("webui-python")
        .join("src")
        .join("version.rs");
    let exposed = read_rust_string_const(&source_path, "PYTHON_PACKAGE_VERSION")?;
    if exposed != expected {
        return Err(format!(
            "{} exposes version {exposed}, expected {expected}.\n  help: run `cargo xtask version \
             {release_version}`",
            source_path.display()
        ));
    }
    Ok(expected)
}

fn read_pyproject_version(path: &Path) -> Result<String, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut in_project = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[project]" {
            in_project = true;
        } else if trimmed.starts_with('[') {
            in_project = false;
        } else if in_project {
            if let Some(value) = quoted_assignment(trimmed, "version") {
                return Ok(value.to_string());
            }
        }
    }
    Err(format!(
        "Could not find [project].version in {}",
        path.display()
    ))
}

fn read_rust_string_const(path: &Path, name: &str) -> Result<String, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let prefix = format!("pub(crate) const {name}: &str = \"");
    let Some(start) = content.find(&prefix) else {
        return Err(format!("Could not find {name} in {}", path.display()));
    };
    let value_start = start + prefix.len();
    let Some(end) = content[value_start..].find('"') else {
        return Err(format!(
            "Could not find the closing quote for {name} in {}",
            path.display()
        ));
    };
    Ok(content[value_start..value_start + end].to_string())
}

fn quoted_assignment<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let remainder = line.strip_prefix(name)?.trim_start();
    let remainder = remainder.strip_prefix('=')?.trim_start();
    remainder.strip_prefix('"')?.strip_suffix('"')
}

/// Update every workspace version target and return the changed paths.
pub(crate) fn update_workspace(root: &Path, version: &str) -> Result<Vec<PathBuf>, String> {
    let release = ReleaseVersion::parse(version).ok_or_else(|| {
        format!(
            "Invalid release version: {version}. Expected major.minor.patch or \
             major.minor.patch-hotfix.number"
        )
    })?;
    let versions = VersionValues::new(release);
    let targets = discover_targets(root);
    let mut updated = Vec::with_capacity(targets.len());

    for target in &targets {
        if apply_update(execute_update(target, &versions), target)? {
            updated.push(target.path.clone());
        }
    }

    Ok(updated)
}

pub fn run(version: Option<&str>) -> ExitCode {
    let Some(version) = version else {
        eprintln!(
            "  {} Usage: cargo xtask version <release-version>",
            console::style("✘").red().bold()
        );
        eprintln!("  Examples: cargo xtask version 0.2.0");
        eprintln!("            cargo xtask version 0.2.0-hotfix.1");
        return ExitCode::FAILURE;
    };

    if ReleaseVersion::parse(version).is_none() {
        eprintln!(
            "  {} Invalid release version: {version}",
            console::style("✘").red().bold()
        );
        eprintln!(
            "  {} Expected major.minor.patch or major.minor.patch-hotfix.number",
            console::style("help:").yellow()
        );
        return ExitCode::FAILURE;
    }

    let root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "  {} Failed to read current directory: {e}",
                console::style("✘").red().bold()
            );
            return ExitCode::FAILURE;
        }
    };

    eprintln!(
        "\n  {} Updating workspace to {}\n",
        console::style("⚡").cyan().bold(),
        console::style(version).bold()
    );

    let updated = match update_workspace(&root, version) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("  {} {error}", console::style("✘").red().bold());
            return ExitCode::FAILURE;
        }
    };
    for path in &updated {
        let relative = path.strip_prefix(&root).unwrap_or(path).display();
        eprintln!("  {} {relative}", console::style("✔").green());
    }

    eprintln!(
        "\n  {} Updated {} file{}\n",
        console::style("✨").green(),
        console::style(updated.len()).bold(),
        if updated.len() == 1 { "" } else { "s" }
    );

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_release_versions() {
        assert!(ReleaseVersion::parse("0.1.0").is_some());
        assert!(ReleaseVersion::parse("1.0.0").is_some());
        assert!(ReleaseVersion::parse("12.34.56-hotfix.7").is_some());
    }

    #[test]
    fn test_invalid_release_versions() {
        assert!(ReleaseVersion::parse("").is_none());
        assert!(ReleaseVersion::parse("1.0").is_none());
        assert!(ReleaseVersion::parse("1.0.0.0").is_none());
        assert!(ReleaseVersion::parse("abc").is_none());
        assert!(ReleaseVersion::parse("1.0.beta").is_none());
        assert!(ReleaseVersion::parse("v1.0.0").is_none());
        assert!(ReleaseVersion::parse("1.0.0-hotfix.0").is_none());
    }

    #[test]
    fn test_update_package_json_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(&pkg, r#"{"name":"test","version":"0.0.1"}"#).unwrap();

        update_package_json(&pkg, "1.2.3", "latest").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["version"], "1.2.3");
    }

    #[test]
    fn test_update_root_package_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(&pkg, r#"{"name":"webui","version":"1.0.0","private":true}"#).unwrap();

        update_package_json(&pkg, "2.0.0", "latest").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["version"], "2.0.0");
        assert_eq!(val["name"], "webui");
    }

    #[test]
    fn test_update_package_json_optional_deps() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(
            &pkg,
            r#"{"name":"test","version":"0.0.1","optionalDependencies":{"@microsoft/webui-darwin-arm64":"0.0.1","unrelated-pkg":"3.0.0"}}"#,
        )
        .unwrap();

        update_package_json(&pkg, "2.0.0", "latest").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["version"], "2.0.0");
        assert_eq!(
            val["optionalDependencies"]["@microsoft/webui-darwin-arm64"],
            "2.0.0"
        );
        // Non-webui deps should be untouched
        assert_eq!(val["optionalDependencies"]["unrelated-pkg"], "3.0.0");
    }

    #[test]
    fn test_update_package_json_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = update_package_json(&dir.path().join("nope.json"), "1.0.0", "latest");
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn test_update_package_json_preserves_formatting() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        let original =
            "{\n  \"name\": \"webui\",\n  \"version\": \"1.0.0\",\n  \"private\": true\n}\n";
        fs::write(&pkg, original).unwrap();

        update_package_json(&pkg, "2.0.0", "latest").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let expected =
            "{\n  \"name\": \"webui\",\n  \"version\": \"2.0.0\",\n  \"private\": true\n}\n";
        assert_eq!(content, expected, "only version value should change");
    }

    #[test]
    fn test_update_public_package_json_uses_release_line_hotfix_tag() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(
            &pkg,
            "{\n  \"name\": \"@microsoft/webui-router\",\n  \"version\": \"1.2.3\",\n  \
             \"publishConfig\": {\n    \"tag\": \"latest\"\n  }\n}\n",
        )
        .unwrap();

        update_package_json(&pkg, "1.2.3-hotfix.4", "hotfix-1.2.3").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["version"], "1.2.3-hotfix.4");
        assert_eq!(value["publishConfig"]["tag"], "hotfix-1.2.3");

        update_package_json(&pkg, "1.2.4", "latest").unwrap();
        let content = fs::read_to_string(&pkg).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["version"], "1.2.4");
        assert_eq!(value["publishConfig"]["tag"], "latest");
    }

    #[test]
    fn test_update_public_package_json_requires_dist_tag_policy() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(
            &pkg,
            r#"{"name":"@microsoft/webui-router","version":"1.2.3"}"#,
        )
        .unwrap();

        let error = update_package_json(&pkg, "1.2.3-hotfix.1", "hotfix-1.2.3").unwrap_err();

        assert!(error.contains("must define a string publishConfig.tag"));
    }

    #[test]
    fn test_update_cargo_workspace_version() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace.dependencies]\nserde = \"1.0\"\n",
        )
        .unwrap();

        update_cargo_workspace_version(dir.path(), "3.0.0").unwrap();

        let content = fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(content.contains("version = \"3.0.0\""));
        assert!(content.contains("edition = \"2021\""));
        // non-webui deps should be untouched
        assert!(content.contains("serde = \"1.0\""));
    }

    #[test]
    fn test_replace_inline_version() {
        let line =
            r#"microsoft-webui-protocol = { path = "../webui-protocol", version = "0.0.1" }"#;
        let result = replace_inline_version(line, "1.2.3").unwrap();
        assert_eq!(
            result,
            r#"microsoft-webui-protocol = { path = "../webui-protocol", version = "1.2.3" }"#
        );
    }

    #[test]
    fn test_update_cargo_lock_versions() {
        let dir = tempfile::TempDir::new().unwrap();
        let lock = dir.path().join("Cargo.lock");
        fs::write(
            &lock,
            r#"[[package]]
name = "microsoft-webui"
version = "0.0.1"

[[package]]
name = "serde"
version = "1.0.0"

[[package]]
name = "xtask"
version = "0.1.0"

[[package]]
name = "microsoft-webui-handler"
version = "0.0.1"

[[package]]
name = "microsoft-webui-python"
version = "0.0.1"
"#,
        )
        .unwrap();

        assert!(update_cargo_lock_versions(&lock, "0.0.1-hotfix.1", "0.0.1-post.1").unwrap());

        let content = fs::read_to_string(&lock).unwrap();
        assert_eq!(content.matches("version = \"0.0.1-hotfix.1\"").count(), 2);
        assert!(content.contains("name = \"microsoft-webui-python\"\nversion = \"0.0.1-post.1\""));
        assert!(content.contains("name = \"serde\"\nversion = \"1.0.0\""));
        assert!(content.contains("name = \"xtask\"\nversion = \"0.1.0\""));
    }

    #[test]
    fn test_update_crate_dep_versions() {
        let dir = tempfile::TempDir::new().unwrap();
        let toml = dir.path().join("Cargo.toml");
        fs::write(
            &toml,
            r#"[package]
name = "test"
version = "0.0.1"

[dependencies]
microsoft-webui-protocol = { path = "../webui-protocol", version = "0.0.1" }
serde = { workspace = true }
microsoft-webui-handler = { path = "../webui-handler", version = "0.0.1" }

[dev-dependencies]
microsoft-webui-test-utils = { path = "../webui-test-utils", version = "0.0.1" }
"#,
        )
        .unwrap();

        let changed = update_crate_dep_versions(&toml, "2.0.0").unwrap();
        assert!(changed);

        let content = fs::read_to_string(&toml).unwrap();
        // Package-level version should be untouched
        assert!(content.contains("version = \"0.0.1\""));
        // But all microsoft-webui dep versions should be updated
        assert!(!content.contains(
            r#"microsoft-webui-protocol = { path = "../webui-protocol", version = "0.0.1" }"#
        ));
        assert!(content.contains(
            r#"microsoft-webui-protocol = { path = "../webui-protocol", version = "2.0.0" }"#
        ));
        assert!(content.contains(
            r#"microsoft-webui-handler = { path = "../webui-handler", version = "2.0.0" }"#
        ));
        assert!(content.contains(
            r#"microsoft-webui-test-utils = { path = "../webui-test-utils", version = "2.0.0" }"#
        ));
        // workspace deps should be untouched
        assert!(content.contains("serde = { workspace = true }"));
    }

    #[test]
    fn test_hotfix_crate_dependencies_use_exact_requirements() {
        let dir = tempfile::TempDir::new().unwrap();
        let toml = dir.path().join("Cargo.toml");
        fs::write(
            &toml,
            r#"[dependencies]
microsoft-webui-handler = { path = "../webui-handler", version = "0.0.27" }
"#,
        )
        .unwrap();

        assert!(update_crate_dep_versions(&toml, "0.0.27-hotfix.1").unwrap());
        assert!(fs::read_to_string(&toml)
            .unwrap()
            .contains(r#"version = "=0.0.27-hotfix.1""#));
    }

    #[test]
    fn test_update_dotnet_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let dotnet_dir = dir.path().join("dotnet");
        fs::create_dir_all(&dotnet_dir).unwrap();
        let props = dotnet_dir.join("Directory.Build.props");
        fs::write(
            &props,
            "<Project>\n  <PropertyGroup>\n    <Version>0.0.1</Version>\n  </PropertyGroup>\n</Project>\n",
        )
        .unwrap();

        let changed = update_dotnet_version(&props, "1.2.3").unwrap();
        assert!(changed);

        let content = fs::read_to_string(&props).unwrap();
        assert!(content.contains("<Version>1.2.3</Version>"));
        assert!(!content.contains("<Version>0.0.1</Version>"));
    }

    #[test]
    fn test_update_dotnet_version_missing_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        // File doesn't exist — should error
        let result = update_dotnet_version(&dir.path().join("nope.props"), "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_update_pyproject_version_replaces_dynamic_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let pyproject = dir.path().join("pyproject.toml");
        fs::write(
            &pyproject,
            "[build-system]\nrequires = []\n\n[project]\nname = \"test\"\ndynamic = [\"version\"]\n",
        )
        .unwrap();

        assert!(update_pyproject_version(&pyproject, "1.2.3.post4").unwrap());

        let content = fs::read_to_string(&pyproject).unwrap();
        assert!(content.contains("version = \"1.2.3.post4\""));
        assert!(!content.contains("dynamic = [\"version\"]"));
    }

    #[test]
    fn test_update_pyproject_version_updates_static_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let pyproject = dir.path().join("pyproject.toml");
        fs::write(
            &pyproject,
            "[project]\nname = \"test\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();

        assert!(update_pyproject_version(&pyproject, "1.2.3.post1").unwrap());
        assert!(fs::read_to_string(&pyproject)
            .unwrap()
            .contains("version = \"1.2.3.post1\""));
    }

    #[test]
    fn test_update_rust_string_const() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = dir.path().join("version.rs");
        fs::write(
            &source,
            "pub(crate) const PYTHON_PACKAGE_VERSION: &str = \"1.2.3\";\n",
        )
        .unwrap();

        assert!(
            update_rust_string_const(&source, "PYTHON_PACKAGE_VERSION", "1.2.3.post1").unwrap()
        );
        assert!(fs::read_to_string(&source)
            .unwrap()
            .contains("PYTHON_PACKAGE_VERSION: &str = \"1.2.3.post1\""));
    }

    #[test]
    fn test_update_workspace_maps_python_hotfix_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let python_root = dir.path().join("crates").join("webui-python");
        let npm_root = dir.path().join("packages").join("webui-router");
        fs::create_dir_all(python_root.join("src")).unwrap();
        fs::create_dir_all(&npm_root).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        fs::write(
            python_root.join("pyproject.toml"),
            "[project]\nname = \"test\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        fs::write(
            python_root.join("Cargo.toml"),
            "[package]\nname = \"microsoft-webui-python\"\nversion = \"1.2.3\"\n\n[dependencies]\n\
             microsoft-webui-handler = { path = \"../webui-handler\", version = \"1.2.3\" }\n",
        )
        .unwrap();
        fs::write(
            python_root.join("src").join("version.rs"),
            "pub(crate) const PYTHON_PACKAGE_VERSION: &str = \"1.2.3\";\n",
        )
        .unwrap();
        fs::write(
            npm_root.join("package.json"),
            r#"{"name":"@microsoft/webui-router","version":"1.2.3","publishConfig":{"tag":"latest"}}"#,
        )
        .unwrap();

        let updated = update_workspace(dir.path(), "1.2.3-hotfix.4").unwrap();

        assert_eq!(updated.len(), 5);
        assert!(fs::read_to_string(dir.path().join("Cargo.toml"))
            .unwrap()
            .contains("version = \"1.2.3-hotfix.4\""));
        let python_cargo = fs::read_to_string(python_root.join("Cargo.toml")).unwrap();
        assert!(python_cargo.contains("version = \"1.2.3-post.4\""));
        assert!(python_cargo.contains(r#"version = "=1.2.3-hotfix.4""#));
        assert!(fs::read_to_string(python_root.join("pyproject.toml"))
            .unwrap()
            .contains("version = \"1.2.3.post4\""));
        assert!(
            fs::read_to_string(python_root.join("src").join("version.rs"))
                .unwrap()
                .contains("PYTHON_PACKAGE_VERSION: &str = \"1.2.3.post4\"")
        );
        let npm: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(npm_root.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(npm["version"], "1.2.3-hotfix.4");
        assert_eq!(npm["publishConfig"]["tag"], "hotfix-1.2.3");
        assert_eq!(
            python_package_version(dir.path(), "1.2.3-hotfix.4"),
            Ok("1.2.3.post4".to_string())
        );
    }

    #[test]
    fn public_npm_packages_follow_release_dist_tag_policy() {
        let root = crate::util::workspace_root().unwrap();
        let release = ReleaseVersion::parse(&read_version().unwrap()).unwrap();
        let expected_tag = release.npm_dist_tag();
        for path in find_package_jsons(&root) {
            let content = fs::read_to_string(&path).unwrap();
            let package: serde_json::Value = serde_json::from_str(&content).unwrap();
            let is_public = package["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("@microsoft/webui"))
                && package["private"].as_bool() != Some(true);
            if is_public {
                assert_eq!(
                    package["publishConfig"]["tag"],
                    expected_tag,
                    "{} must use the dist-tag for the current release",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn test_python_package_version_rejects_mismatched_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let python_root = dir.path().join("crates").join("webui-python");
        fs::create_dir_all(python_root.join("src")).unwrap();
        fs::write(
            python_root.join("pyproject.toml"),
            "[project]\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        fs::write(
            python_root.join("Cargo.toml"),
            "[package]\nversion = \"1.2.3-post.1\"\n",
        )
        .unwrap();
        fs::write(
            python_root.join("src").join("version.rs"),
            "pub(crate) const PYTHON_PACKAGE_VERSION: &str = \"1.2.3\";\n",
        )
        .unwrap();

        let error = python_package_version(dir.path(), "1.2.3-hotfix.1").unwrap_err();
        assert!(error.contains("run `cargo xtask version 1.2.3-hotfix.1`"));
    }

    #[test]
    fn test_read_version_from_workspace() {
        // read_version reads from the real workspace Cargo.toml
        let version = read_version();
        assert!(version.is_ok(), "should read version from workspace");
        let v = version.unwrap();
        assert!(
            ReleaseVersion::parse(&v).is_some(),
            "version '{v}' should be valid"
        );
    }
}
