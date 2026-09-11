// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use webui_discovery::{discover_source, discover_source_with_plugin, FastDiscoveryPlugin};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write_component(root: &Path, tag: &str) -> std::io::Result<()> {
    let dir = root.join("components").join(tag);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{tag}.html")), "<button>Example</button>")
}

#[test]
fn native_catalog_supports_ancestor_resolution_and_cache_invalidation() -> TestResult {
    let root = tempfile::tempdir()?;
    let site = root.path().join("site");
    fs::create_dir_all(site.join("node_modules"))?;
    let package = root.path().join("node_modules/@fixture/catalog");
    write_component(&package, "test-button")?;
    fs::write(
        package.join("components/test-button/test-button.ts"),
        "export class TestButton {}",
    )?;
    fs::write(
        package.join("package.json"),
        r#"{"name":"@fixture/catalog","exports":{"./button.js":"./dist/button.js"}}"#,
    )?;
    let first = discover_source("@fixture/catalog", &site)?;
    assert_eq!(first.components.len(), 1);
    assert_eq!(first.components[0].tag_name, "test-button");
    assert!(first.components[0].is_client_owned);
    assert_eq!(first.components[0].source, "@fixture/catalog");
    assert!(first.components[0].css_content.is_none());

    fs::write(
        package.join("components/test-button/test-button.css"),
        "button { color: blue; }",
    )?;
    write_component(&package, "test-alert")?;
    // Compiled duplicates and documentation must not become extra components.
    fs::create_dir_all(package.join("dist/components/test-button"))?;
    fs::write(
        package.join("dist/components/test-button/test-button.html"),
        "Duplicate",
    )?;
    fs::write(
        package.join("components/test-button/index.html"),
        "Documentation",
    )?;
    let second = discover_source("@fixture/catalog", &site)?;
    assert_eq!(second.components.len(), 2);
    assert_eq!(second.components[0].tag_name, "test-alert");
    assert_eq!(
        second.components[1].css_content.as_deref(),
        Some("button { color: blue; }")
    );
    fs::remove_dir_all(package.join("components/test-alert"))?;
    assert_eq!(
        discover_source("@fixture/catalog", &site)?.components.len(),
        1
    );
    Ok(())
}

#[test]
fn mixed_catalog_ownership_tracks_component_scripts_not_package_exports() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/mixed");
    write_component(&package, "test-button")?;
    write_component(&package, "test-text")?;
    fs::write(
        package.join("package.json"),
        r#"{"exports":{"./button.js":"./dist/components/test-button/test-button.js"}}"#,
    )?;
    let button = package.join("components/test-button/test-button.ts");
    let text = package.join("components/test-text/test-text.js");
    fs::write(&button, "export class TestButton {}")?;
    fs::write(
        package.join("components/test-text/test-text.spec.ts"),
        "throw new Error('Test-only source');",
    )?;
    let discovered = discover_source("@fixture/mixed", root.path())?;
    assert_eq!(discovered.components.len(), 2);
    assert!(discovered.components[0].is_client_owned);
    assert!(!discovered.components[1].is_client_owned);

    fs::write(&text, "export class TestText {}")?;
    assert!(discover_source("@fixture/mixed", root.path())?.components[1].is_client_owned);
    fs::remove_file(&text)?;
    fs::remove_file(&button)?;
    assert!(discover_source("@fixture/mixed", root.path())?
        .components
        .iter()
        .all(|component| !component.is_client_owned));
    Ok(())
}

#[test]
fn invalid_nearest_package_does_not_fall_back_to_an_ancestor() -> TestResult {
    let root = tempfile::tempdir()?;
    let site = root.path().join("site");
    for directory in [root.path(), site.as_path()] {
        let package = directory.join("node_modules/@fixture/catalog");
        write_component(&package, "test-button")?;
        fs::write(package.join("package.json"), "{}")?;
    }
    fs::remove_dir_all(site.join("node_modules/@fixture/catalog/components"))?;
    fs::write(
        site.join("node_modules/@fixture/catalog/components"),
        "not a directory",
    )?;
    assert!(discover_source("@fixture/catalog", &site).is_err());
    Ok(())
}

#[test]
fn native_filenames_ignore_template_exports_and_custom_elements_metadata() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/catalog");
    write_component(&package, "test-button")?;
    fs::write(
        package.join("package.json"),
        r#"{
            "exports": {
                "./template-webui.html": "../outside.html",
                "./styles.css": "../outside.css"
            },
            "customElements": "../missing-manifest.json",
            "main": "./dist/unrelated.js"
        }"#,
    )?;
    let result = discover_source("@fixture/catalog", root.path())?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "test-button");
    assert!(!result.components[0].is_client_owned);
    assert!(result.components[0].css_content.is_none());
    Ok(())
}

#[test]
fn native_packages_use_html_basenames_without_directory_name_requirements() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/names");
    fs::create_dir_all(&package)?;
    fs::write(package.join("package.json"), r#"{"name":"@fixture/names"}"#)?;
    fs::write(package.join("test-flat.html"), "<span>Flat</span>")?;
    fs::write(package.join("test-flat.css"), "span { color: blue; }")?;
    fs::create_dir_all(package.join("node_modules/other"))?;
    fs::create_dir_all(package.join(".hidden"))?;
    fs::write(
        package.join("node_modules/other/other-widget.html"),
        "<span>Dependency</span>",
    )?;
    fs::write(
        package.join(".hidden/hidden-widget.html"),
        "<span>Hidden</span>",
    )?;
    let result = discover_source("@fixture/names", root.path())?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "test-flat");
    assert_eq!(
        result.components[0].css_content.as_deref(),
        Some("span { color: blue; }")
    );

    let components = package.join("components");
    fs::create_dir_all(components.join("nested/deep"))?;
    fs::write(components.join("test-root.html"), "<span>Root</span>")?;
    fs::write(
        components.join("nested/deep/test-nested.html"),
        "<span>Nested</span>",
    )?;
    let result = discover_source("@fixture/names", root.path())?;
    let tags: Vec<_> = result
        .components
        .iter()
        .map(|component| component.tag_name.as_str())
        .collect();
    assert_eq!(tags, ["test-nested", "test-root"]);
    Ok(())
}

#[test]
fn fast_keeps_manifest_names_and_special_template_style_paths() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/fast");
    fs::create_dir_all(package.join("dist"))?;
    fs::write(
        package.join("package.json"),
        r#"{
        "customElements": "./custom-elements.json",
        "exports": {"./card.js": "./dist/card.js"}
    }"#,
    )?;
    fs::write(
        package.join("custom-elements.json"),
        r#"{
        "modules": [{
            "path": "dist/card.js",
            "declarations": [{"name": "Card", "tagName": "fast-card"}]
        }]
    }"#,
    )?;
    fs::write(package.join("dist/card.js"), "export {};")?;
    fs::write(
        package.join("dist/card.template-webui.html"),
        "<template>Card</template>",
    )?;
    fs::write(
        package.join("dist/card.styles.css"),
        ":host { color: blue; }",
    )?;
    let result =
        discover_source_with_plugin("@fixture/fast", root.path(), &FastDiscoveryPlugin::new())?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "fast-card");
    assert_eq!(
        result.components[0].css_content.as_deref(),
        Some(":host { color: blue; }")
    );
    assert!(result.components[0].is_client_owned);
    let native = discover_source("@fixture/fast", root.path())?;
    assert_eq!(native.components[0].tag_name, "card.template-webui");
    Ok(())
}

#[test]
fn fast_selects_only_exported_webui_template_variant() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/button");
    fs::create_dir_all(package.join("dist/esm"))?;
    fs::write(
        package.join("package.json"),
        r#"{
        "customElements": "./custom-elements.json",
        "exports": {
            ".": "./dist/esm/button.js",
            "./template.html": "./dist/button.template.html",
            "./template-webui.html": {"default": "./dist/button.template-webui.html"},
            "./styles.css": "./dist/button.styles.css"
        }
    }"#,
    )?;
    fs::write(
        package.join("custom-elements.json"),
        r#"{
        "modules": [{"path":"dist/esm/button.js",
            "declarations":[{"name":"Button","tagName":"fast-button"}]}]
    }"#,
    )?;
    fs::write(package.join("dist/esm/button.js"), "export {};")?;
    let converted = "<template shadowrootmode=\"open\">Converted</template>";
    fs::write(package.join("dist/button.template-webui.html"), converted)?;
    fs::write(
        package.join("dist/button.template.html"),
        "<template>Wrong variant</template>",
    )?;
    fs::write(
        package.join("dist/button.styles.css"),
        ":host { color: blue; }",
    )?;
    let result =
        discover_source_with_plugin("@fixture/button", root.path(), &FastDiscoveryPlugin::new())?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "fast-button");
    assert_eq!(result.components[0].html_content, converted);
    assert_eq!(
        result.components[0].css_content.as_deref(),
        Some(":host { color: blue; }")
    );
    assert!(result.components[0].is_client_owned);
    let manifest = fs::read_to_string(package.join("package.json"))?;
    fs::write(
        package.join("package.json"),
        r#"{"customElements":"custom-elements.json","main":"./dist/esm/button.js"}"#,
    )?;
    let inferred =
        discover_source_with_plugin("@fixture/button", root.path(), &FastDiscoveryPlugin::new())?;
    assert_eq!(inferred.components[0].html_content, converted);
    assert_eq!(
        inferred.components[0].css_content.as_deref(),
        Some(":host { color: blue; }")
    );
    fs::write(package.join("package.json"), manifest)?;
    fs::write(
        package.join("dist/button.styles.css"),
        ":host { color: red; }",
    )?;
    let changed =
        discover_source_with_plugin("@fixture/button", root.path(), &FastDiscoveryPlugin::new())?;
    assert_eq!(
        changed.components[0].css_content.as_deref(),
        Some(":host { color: red; }")
    );

    fs::remove_file(package.join("dist/button.styles.css"))?;
    assert!(discover_source_with_plugin(
        "@fixture/button",
        root.path(),
        &FastDiscoveryPlugin::new(),
    )
    .is_err());
    fs::write(
        package.join("dist/button.styles.css"),
        ":host { color: red; }",
    )?;
    fs::remove_file(package.join("dist/button.template-webui.html"))?;
    assert!(discover_source_with_plugin(
        "@fixture/button",
        root.path(),
        &FastDiscoveryPlugin::new()
    )
    .is_err());
    Ok(())
}

#[test]
fn fast_rejects_invalid_webui_template_exports_without_falling_back() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/@fixture/invalid");
    fs::create_dir_all(package.join("dist"))?;
    fs::write(
        package.join("custom-elements.json"),
        r#"{
        "modules":[{"path":"dist/button.js",
            "declarations":[{"name":"Button","tagName":"fast-button"}]}]
    }"#,
    )?;
    fs::write(package.join("dist/button.js"), "export {};")?;
    fs::write(
        package.join("dist/button.template.html"),
        "<template>Wrong raw variant</template>",
    )?;
    fs::write(
        package.join("dist/button.template-webui.html"),
        "<template>Sibling</template>",
    )?;
    for target in [
        r#""../escape.template-webui.html""#,
        "true",
        r#""./dist/button.template.html""#,
    ] {
        fs::write(
            package.join("package.json"),
            format!(
                r#"{{"customElements":"custom-elements.json","exports":{{"./template-webui.html":{target}}}}}"#
            ),
        )?;
        assert!(discover_source_with_plugin(
            "@fixture/invalid",
            root.path(),
            &FastDiscoveryPlugin::new(),
        )
        .is_err());
    }
    Ok(())
}
