// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Atomic version bumping across all Cargo.toml and package.json files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Apply a version update and log the result.
///
/// On `Ok(true)` prints a success tick and increments the counter.
/// On `Ok(false)` (already up-to-date) does nothing.
/// On `Err` prints the error and returns `ExitCode::FAILURE`.
fn apply_update(
    result: Result<bool, String>,
    path: &Path,
    root: &Path,
    total_updated: &mut usize,
) -> Result<(), ExitCode> {
    match result {
        Ok(true) => {
            let relative = path.strip_prefix(root).unwrap_or(path).display();
            eprintln!("  {} {relative}", console::style("✔").green());
            *total_updated += 1;
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => {
            eprintln!("  {} {e}", console::style("✘").red().bold());
            Err(ExitCode::FAILURE)
        }
    }
}

/// Validate a semver string (basic check: major.minor.patch).
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u64>().is_ok())
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

/// Update `version = "..."` in `[workspace.package]` of root Cargo.toml.
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
            if let Some(new_line) = replace_inline_version(line, version) {
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

/// Update version in a package.json file. Also updates optionalDependencies
/// that reference @microsoft/webui-* packages.
///
/// Uses serde_json to read the structure, then performs surgical string
/// replacement so only the version values change — all formatting is preserved.
fn update_package_json(path: &Path, version: &str) -> Result<bool, String> {
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

    if changed {
        fs::write(path, &result).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    }

    Ok(changed)
}

/// Update `<Version>...</Version>` in dotnet/Directory.Build.props.
fn update_dotnet_version(root: &Path, version: &str) -> Result<(), String> {
    let props_path = root.join("dotnet").join("Directory.Build.props");
    if !props_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&props_path)
        .map_err(|e| format!("Failed to read Directory.Build.props: {e}"))?;

    let Some(start) = content.find("<Version>") else {
        return Err("Could not find <Version> tag in Directory.Build.props".to_string());
    };
    let tag_value_start = start + "<Version>".len();
    let Some(end) = content[tag_value_start..].find("</Version>") else {
        return Err("Could not find closing </Version> tag in Directory.Build.props".to_string());
    };

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..tag_value_start]);
    result.push_str(version);
    result.push_str(&content[tag_value_start + end..]);

    fs::write(&props_path, result)
        .map_err(|e| format!("Failed to write Directory.Build.props: {e}"))?;
    Ok(())
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
    let content =
        fs::read_to_string(&cargo_path).map_err(|e| format!("Failed to read Cargo.toml: {e}"))?;

    let mut in_workspace_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
        } else if trimmed.starts_with('[') {
            in_workspace_package = false;
        }
        if in_workspace_package && trimmed.starts_with("version") && trimmed.contains('=') {
            // Extract the version value between quotes
            if let Some(start) = trimmed.find('"') {
                if let Some(end) = trimmed[start + 1..].find('"') {
                    return Ok(trimmed[start + 1..start + 1 + end].to_string());
                }
            }
        }
    }

    Err("Could not find version in [workspace.package] of Cargo.toml".to_string())
}

pub fn run(version: Option<&str>) -> ExitCode {
    let Some(version) = version else {
        eprintln!(
            "  {} Usage: cargo xtask version <semver>",
            console::style("✘").red().bold()
        );
        eprintln!("  Example: cargo xtask version 0.2.0");
        return ExitCode::FAILURE;
    };

    if !is_valid_semver(version) {
        eprintln!(
            "  {} Invalid semver: {version}. Expected format: major.minor.patch",
            console::style("✘").red().bold()
        );
        return ExitCode::FAILURE;
    }

    eprintln!(
        "\n  {} Updating all versions to {}\n",
        console::style("⚡").cyan().bold(),
        console::style(version).bold()
    );

    // 1. Update workspace Cargo.toml
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

    let mut total_updated: usize = 0;

    if let Err(e) = update_cargo_workspace_version(&root, version) {
        eprintln!("  {} {e}", console::style("✘").red().bold());
        return ExitCode::FAILURE;
    }
    eprintln!("  {} Cargo.toml (workspace)", console::style("✔").green());
    total_updated += 1;

    // 2. Update root package.json
    let root_pkg = root.join("package.json");
    if let Err(code) = apply_update(
        update_package_json(&root_pkg, version),
        &root_pkg,
        &root,
        &mut total_updated,
    ) {
        return code;
    }

    // 3. Update dotnet/Directory.Build.props
    if let Err(e) = update_dotnet_version(&root, version) {
        eprintln!("  {} {e}", console::style("✘").red().bold());
        return ExitCode::FAILURE;
    }
    let dotnet_props = root.join("dotnet").join("Directory.Build.props");
    if dotnet_props.exists() {
        eprintln!(
            "  {} dotnet/Directory.Build.props",
            console::style("✔").green()
        );
        total_updated += 1;
    }

    // 4. Update inter-crate dependency versions in crates/*/Cargo.toml
    let crate_tomls = find_crate_cargo_tomls(&root);
    for toml_path in &crate_tomls {
        if let Err(code) = apply_update(
            update_crate_dep_versions(toml_path, version),
            toml_path,
            &root,
            &mut total_updated,
        ) {
            return code;
        }
    }

    // 5. Update all package.json files under packages/
    let package_jsons = find_package_jsons(&root);
    for pkg_path in &package_jsons {
        if let Err(code) = apply_update(
            update_package_json(pkg_path, version),
            pkg_path,
            &root,
            &mut total_updated,
        ) {
            return code;
        }
    }

    // 6. Update commerce example (server/Cargo.toml + package.json)
    let commerce_root = root.join("examples/app/commerce");
    let commerce_cargo = commerce_root.join("server/Cargo.toml");
    if let Err(code) = apply_update(
        update_toml_section_version(&commerce_cargo, "[package]", version),
        &commerce_cargo,
        &root,
        &mut total_updated,
    ) {
        return code;
    }
    let commerce_pkg = commerce_root.join("package.json");
    if let Err(code) = apply_update(
        update_package_json(&commerce_pkg, version),
        &commerce_pkg,
        &root,
        &mut total_updated,
    ) {
        return code;
    }

    eprintln!(
        "\n  {} Updated {} file{}\n",
        console::style("✨").green(),
        console::style(total_updated).bold(),
        if total_updated == 1 { "" } else { "s" }
    );

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_semver() {
        assert!(is_valid_semver("0.1.0"));
        assert!(is_valid_semver("1.0.0"));
        assert!(is_valid_semver("12.34.56"));
    }

    #[test]
    fn test_invalid_semver() {
        assert!(!is_valid_semver(""));
        assert!(!is_valid_semver("1.0"));
        assert!(!is_valid_semver("1.0.0.0"));
        assert!(!is_valid_semver("abc"));
        assert!(!is_valid_semver("1.0.beta"));
        assert!(!is_valid_semver("v1.0.0"));
    }

    #[test]
    fn test_update_package_json_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(&pkg, r#"{"name":"test","version":"0.0.1"}"#).unwrap();

        update_package_json(&pkg, "1.2.3").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["version"], "1.2.3");
    }

    #[test]
    fn test_update_root_package_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        fs::write(&pkg, r#"{"name":"webui","version":"1.0.0","private":true}"#).unwrap();

        update_package_json(&pkg, "2.0.0").unwrap();

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

        update_package_json(&pkg, "2.0.0").unwrap();

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
        let result = update_package_json(&dir.path().join("nope.json"), "1.0.0");
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn test_update_package_json_preserves_formatting() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        let original =
            "{\n  \"name\": \"webui\",\n  \"version\": \"1.0.0\",\n  \"private\": true\n}\n";
        fs::write(&pkg, original).unwrap();

        update_package_json(&pkg, "2.0.0").unwrap();

        let content = fs::read_to_string(&pkg).unwrap();
        let expected =
            "{\n  \"name\": \"webui\",\n  \"version\": \"2.0.0\",\n  \"private\": true\n}\n";
        assert_eq!(content, expected, "only version value should change");
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

        update_dotnet_version(dir.path(), "1.2.3").unwrap();

        let content = fs::read_to_string(&props).unwrap();
        assert!(content.contains("<Version>1.2.3</Version>"));
        assert!(!content.contains("<Version>0.0.1</Version>"));
    }

    #[test]
    fn test_update_dotnet_version_missing_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        // No dotnet dir — should silently succeed
        let result = update_dotnet_version(dir.path(), "1.0.0");
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_version_from_workspace() {
        // read_version reads from the real workspace Cargo.toml
        let version = read_version();
        assert!(version.is_ok(), "should read version from workspace");
        let v = version.unwrap();
        assert!(is_valid_semver(&v), "version '{v}' should be valid semver");
    }
}
