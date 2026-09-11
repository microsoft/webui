// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};

use webui_discovery::{
    discover_source, discover_source_with_plugin, DiscoveredComponent, DiscoveryPlugin,
    FastDiscoveryPlugin, PackageContext,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn package(root: &Path, name: &str, manifest: &str) -> std::io::Result<PathBuf> {
    let package = root.join("node_modules").join(name);
    fs::create_dir_all(&package)?;
    fs::write(package.join("package.json"), manifest)?;
    Ok(package)
}

#[test]
fn native_scope_skips_utilities_but_reports_invalid_component_files() -> TestResult {
    let root = tempfile::tempdir()?;
    let child = root.path().join("app");
    fs::create_dir_all(child.join("node_modules/unrelated"))?;
    let component = package(root.path(), "@fixture/button", "{}")?;
    package(root.path(), "@fixture/utils", r#"{"main":"./index.js"}"#)?;
    fs::write(
        component.join("test-button.html"),
        "<button>Example</button>",
    )?;
    let result = discover_source("@fixture", &child)?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "test-button");
    let wildcard = discover_source("@fixture/*", &child)?;
    assert_eq!(wildcard.components.len(), 1);
    assert_eq!(wildcard.components[0].tag_name, "test-button");

    fs::write(component.join("test-button.html"), [0xff])?;
    let error = discover_source("@fixture", &child)
        .err()
        .ok_or("scope must surface bad HTML")?;
    assert!(format!("{error:#}").contains("test-button.html"));
    Ok(())
}

#[test]
fn fast_scope_skips_non_components_but_reports_declared_missing_assets() -> TestResult {
    let root = tempfile::tempdir()?;
    let child = root.path().join("app");
    fs::create_dir_all(child.join("node_modules/unrelated"))?;
    let component = package(
        root.path(),
        "@fixture/button",
        r#"{
        "customElements":"custom-elements.json",
        "exports":{"./template-webui.html":"./button.template-webui.html"}
    }"#,
    )?;
    package(root.path(), "@fixture/tokens", r#"{"main":"./index.js"}"#)?;
    fs::write(
        component.join("custom-elements.json"),
        r#"{
        "modules":[{"declarations":[{"name":"Button","tagName":"fast-button"}]}]
    }"#,
    )?;
    fs::write(
        component.join("button.template-webui.html"),
        "<f-template name=\"fast-button\"><template>Button</template></f-template>",
    )?;
    let plugin = FastDiscoveryPlugin::new();
    let result = discover_source_with_plugin("@fixture", &child, &plugin)?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "fast-button");

    fs::remove_file(component.join("button.template-webui.html"))?;
    let error = discover_source_with_plugin("@fixture", &child, &plugin)
        .err()
        .ok_or("scope must surface missing declared templates")?;
    assert!(format!("{error:#}").contains("button.template-webui.html"));
    Ok(())
}

struct InlinePlugin;

impl DiscoveryPlugin for InlinePlugin {
    fn cache_namespace(&self) -> &'static str {
        "fixture-inline"
    }
    fn discover_local(&self, _root: &Path) -> anyhow::Result<Vec<DiscoveredComponent>> {
        Ok(Vec::new())
    }
    fn package_cache_files(&self, _package: PackageContext<'_>) -> anyhow::Result<Vec<PathBuf>> {
        Ok(Vec::new())
    }
    fn discover_package(
        &self,
        package: PackageContext<'_>,
    ) -> anyhow::Result<Vec<DiscoveredComponent>> {
        Ok(vec![DiscoveredComponent {
            tag_name: "inline-component".to_string(),
            html_content: "<span>Inline</span>".to_string(),
            css_content: None,
            is_client_owned: false,
            source: package.name.to_string(),
        }])
    }
}

#[test]
fn custom_plugins_keep_default_scope_support_without_file_dependencies() -> TestResult {
    let root = tempfile::tempdir()?;
    package(root.path(), "@fixture/inline", "{}")?;
    let result = discover_source_with_plugin("@fixture", root.path(), &InlinePlugin)?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "inline-component");
    Ok(())
}

#[test]
fn fast_uses_default_html_when_manifest_metadata_is_absent_or_empty() -> TestResult {
    let root = tempfile::tempdir()?;
    let plain = package(
        root.path(),
        "@fixture/plain",
        r#"{"main":"./unrelated.js"}"#,
    )?;
    fs::create_dir_all(plain.join("components"))?;
    fs::write(
        plain.join("components/plain-card.html"),
        "<span>Plain</span>",
    )?;
    fs::write(
        plain.join("components/plain-card.css"),
        "span { color: blue; }",
    )?;
    let plugin = FastDiscoveryPlugin::new();
    let result = discover_source_with_plugin("@fixture/*", root.path(), &plugin)?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "plain-card");
    assert_eq!(result.components[0].html_content, "<span>Plain</span>");
    assert!(!result.components[0].is_client_owned);

    fs::write(
        plain.join("package.json"),
        r#"{"customElements":"custom-elements.json"}"#,
    )?;
    fs::write(plain.join("custom-elements.json"), r#"{"modules":[]}"#)?;
    fs::write(
        plain.join("components/plain-card.ts"),
        "export class PlainCard {}",
    )?;
    let result = discover_source_with_plugin("@fixture/plain/*", root.path(), &plugin)?;
    assert_eq!(result.components.len(), 1);
    assert!(result.components[0].is_client_owned);
    Ok(())
}

#[test]
fn fast_manifest_components_take_precedence_and_plain_components_fill_gaps() -> TestResult {
    let root = tempfile::tempdir()?;
    let mixed = package(
        root.path(),
        "@fixture/mixed",
        r#"{
        "customElements":"custom-elements.json",
        "exports":{"./template-webui.html":"./dist/button.template-webui.html"}
    }"#,
    )?;
    fs::create_dir_all(mixed.join("dist"))?;
    fs::create_dir_all(mixed.join("components"))?;
    fs::write(
        mixed.join("custom-elements.json"),
        r#"{
        "modules":[{"declarations":[{"name":"Button","tagName":"fast-button"}]}]
    }"#,
    )?;
    fs::write(
        mixed.join("dist/button.template-webui.html"),
        "<f-template name=\"fast-button\"><template>FAST</template></f-template>",
    )?;
    fs::write(
        mixed.join("components/plain-card.html"),
        "<span>Fallback</span>",
    )?;
    fs::write(mixed.join("components/fast-button.html"), [0xff])?;
    fs::write(
        mixed.join("components/unused.template-webui.html"),
        "Must not register",
    )?;
    let result =
        discover_source_with_plugin("@fixture/mixed", root.path(), &FastDiscoveryPlugin::new())?;
    assert_eq!(result.components.len(), 2);
    let declared = result
        .components
        .iter()
        .find(|component| component.tag_name == "fast-button")
        .ok_or("manifest component missing")?;
    assert!(declared.html_content.contains("<f-template"));
    assert!(result
        .components
        .iter()
        .any(|component| component.tag_name == "plain-card"));
    Ok(())
}
