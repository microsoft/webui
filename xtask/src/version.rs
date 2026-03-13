//! Atomic version bumping across all Cargo.toml and package.json files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Validate a semver string (basic check: major.minor.patch).
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u64>().is_ok())
}

/// Update `version = "..."` in workspace.package section of root Cargo.toml.
fn update_cargo_workspace_version(root: &Path, version: &str) -> Result<(), String> {
    let cargo_path = root.join("Cargo.toml");
    let content =
        fs::read_to_string(&cargo_path).map_err(|e| format!("Failed to read Cargo.toml: {e}"))?;

    let mut result = String::with_capacity(content.len());
    let mut in_workspace_package = false;
    let mut updated = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
        } else if trimmed.starts_with('[') {
            in_workspace_package = false;
        }

        if in_workspace_package && trimmed.starts_with("version") && trimmed.contains('=') {
            result.push_str(&format!("version = \"{version}\"\n"));
            updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if !updated {
        return Err("Could not find version in [workspace.package]".to_string());
    }

    fs::write(&cargo_path, result).map_err(|e| format!("Failed to write Cargo.toml: {e}"))?;
    Ok(())
}

/// Update version in a package.json file. Also updates optionalDependencies
/// that reference @microsoft/webui-* packages.
fn update_package_json(path: &Path, version: &str) -> Result<bool, String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

    let obj = value
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;

    // Update top-level version
    if obj.contains_key("version") {
        obj.insert(
            "version".to_string(),
            serde_json::Value::String(version.to_string()),
        );
    }

    // Update optionalDependencies for @microsoft/webui-* packages
    // Skip workspace: protocol values (pnpm resolves them at publish time)
    if let Some(deps) = obj.get_mut("optionalDependencies") {
        if let Some(deps_obj) = deps.as_object_mut() {
            for (key, val) in deps_obj.iter_mut() {
                if key.starts_with("@microsoft/webui") {
                    let current = val.as_str().unwrap_or_default();
                    if !current.starts_with("workspace:") {
                        *val = serde_json::Value::String(version.to_string());
                    }
                }
            }
        }
    }

    let updated = serde_json::to_string_pretty(&value)
        .map_err(|e| format!("Failed to serialize {}: {e}", path.display()))?;

    fs::write(path, format!("{updated}\n"))
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    Ok(true)
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
    let root = std::env::current_dir().unwrap_or_default();

    if let Err(e) = update_cargo_workspace_version(&root, version) {
        eprintln!("  {} {e}", console::style("✘").red().bold());
        return ExitCode::FAILURE;
    }
    eprintln!("  {} Cargo.toml (workspace)", console::style("✔").green());

    // 2. Update dotnet/Directory.Build.props
    if let Err(e) = update_dotnet_version(&root, version) {
        eprintln!("  {} {e}", console::style("✘").red().bold());
        return ExitCode::FAILURE;
    }
    if root.join("dotnet").join("Directory.Build.props").exists() {
        eprintln!(
            "  {} dotnet/Directory.Build.props",
            console::style("✔").green()
        );
    }

    let dotnet_count: usize = if root.join("dotnet").join("Directory.Build.props").exists() {
        1
    } else {
        0
    };

    // 3. Update all package.json files under packages/
    let package_jsons = find_package_jsons(&root);
    let mut count = 0;
    for pkg_path in &package_jsons {
        match update_package_json(pkg_path, version) {
            Ok(true) => {
                let relative = pkg_path.strip_prefix(&root).unwrap_or(pkg_path).display();
                eprintln!("  {} {relative}", console::style("✔").green());
                count += 1;
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("  {} {e}", console::style("✘").red().bold());
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!(
        "\n  {} Updated {} file{}\n",
        console::style("✨").green(),
        console::style(1 + dotnet_count + count).bold(),
        if (dotnet_count + count) == 0 { "" } else { "s" }
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
    fn test_update_cargo_workspace_version() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        update_cargo_workspace_version(dir.path(), "3.0.0").unwrap();

        let content = fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(content.contains("version = \"3.0.0\""));
        assert!(content.contains("edition = \"2021\""));
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
}
