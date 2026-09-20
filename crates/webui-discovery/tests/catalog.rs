// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use webui_discovery::{
    discover_source, discover_source_with_plugin, DiscoveryPlugin, FastDiscoveryPlugin,
    WebUIDiscoveryPlugin,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write_component(root: &Path, tag: &str) -> std::io::Result<()> {
    let dir = root.join("components").join(tag);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{tag}.html")), "<button>Example</button>")
}

#[test]
fn large_script_siblings_preserve_discovery_and_ownership() -> TestResult {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/large-catalog");
    fs::create_dir_all(&package)?;
    fs::write(package.join("package.json"), "{}")?;
    fs::write(package.join("test-card.html"), "<span>Card</span>")?;
    let script = package.join("test-card.js");
    fs::File::create(&script)?.set_len(64 * 1024 * 1024)?;
    let plugins: [&dyn DiscoveryPlugin; 2] =
        [&WebUIDiscoveryPlugin::new(), &FastDiscoveryPlugin::new()];
    for plugin in plugins {
        let result = discover_source_with_plugin("large-catalog", root.path(), plugin)?;
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].html_content, "<span>Card</span>");
        assert!(result.components[0].is_client_owned);
    }
    fs::remove_file(script)?;
    assert!(!discover_source("large-catalog", root.path())?.components[0].is_client_owned);
    Ok(())
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
fn fast_aggregate_manifest_resolves_bare_module_specifiers_and_cache_inputs() -> TestResult {
    let root = tempfile::tempdir()?;
    let node_modules = root.path().join("node_modules");
    let aggregate = node_modules.join("@fixture/aggregate");
    fs::create_dir_all(aggregate.join("components"))?;
    fs::write(
        aggregate.join("package.json"),
        r#"{
        "customElements":"custom-elements.json",
        "dependencies":{"plain-module":"1.0.0"}
    }"#,
    )?;
    fs::write(
        aggregate.join("custom-elements.json"),
        r#"{
        "modules": [
            {
                "path": "components/local.js",
                "declarations": [{"name": "LocalCard", "tagName": "fixture-local"}]
            },
            {
                "path": "@fixture/scoped-module/component.js",
                "declarations": [{"name": "ScopedCard", "tagName": "fixture-scoped"}]
            },
            {
                "path": "plain-module/component.js",
                "declarations": [{"name": "PlainCard", "tagName": "fixture-plain"}]
            }
        ]
    }"#,
    )?;
    fs::write(aggregate.join("components/local.js"), "export {};")?;
    fs::write(
        aggregate.join("components/local.template-webui.html"),
        "<template>Local package path</template>",
    )?;

    let local_name_collision = node_modules.join("components");
    fs::create_dir_all(&local_name_collision)?;
    fs::write(
        local_name_collision.join("package.json"),
        r#"{"exports":{"./local.js":"./wrong.js"}}"#,
    )?;
    fs::write(
        local_name_collision.join("wrong.template-webui.html"),
        "<template>Wrong package</template>",
    )?;

    let scoped = node_modules.join("@fixture/scoped-module");
    fs::create_dir_all(scoped.join("dist"))?;
    fs::create_dir_all(scoped.join("assets"))?;
    fs::write(
        scoped.join("package.json"),
        r#"{
        "exports":{
            "./component.js":{"import":"./dist/scoped-entry.js"},
            "./template-webui.html":"./assets/scoped.template-webui.html",
            "./styles.css":"./assets/scoped.css"
        }
    }"#,
    )?;
    fs::write(scoped.join("dist/scoped-entry.js"), "export {};")?;
    fs::write(
        scoped.join("dist/scoped-entry.template-webui.html"),
        "<template>Wrong sibling</template>",
    )?;
    fs::write(
        scoped.join("dist/scoped-entry.styles.css"),
        ":host { color: red; }",
    )?;
    fs::write(
        scoped.join("assets/scoped.template-webui.html"),
        "<template>Scoped package</template>",
    )?;
    fs::write(scoped.join("assets/scoped.css"), ":host { color: blue; }")?;

    let plain = node_modules.join("plain-module");
    fs::create_dir_all(plain.join("lib"))?;
    fs::write(
        plain.join("package.json"),
        r#"{"exports":{"./component.js":"./lib/plain-entry.js"}}"#,
    )?;
    fs::write(plain.join("lib/plain-entry.js"), "export {};")?;
    fs::write(
        plain.join("lib/plain-entry.template-webui.html"),
        "<template>Plain package</template>",
    )?;
    fs::write(
        plain.join("lib/plain-entry.styles.css"),
        ":host { color: green; }",
    )?;

    let plugin = FastDiscoveryPlugin::new();
    let first = discover_source_with_plugin("@fixture/aggregate", root.path(), &plugin)?;
    let local = first
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-local")
        .ok_or("local component missing")?;
    assert_eq!(
        local.html_content,
        "<template>Local package path</template>"
    );
    assert!(!local.is_client_owned);
    let scoped_component = first
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-scoped")
        .ok_or("scoped component missing")?;
    assert_eq!(
        scoped_component.html_content,
        "<template>Scoped package</template>"
    );
    assert_eq!(
        scoped_component.css_content.as_deref(),
        Some(":host { color: blue; }")
    );
    assert!(scoped_component.is_client_owned);
    let plain_component = first
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-plain")
        .ok_or("plain component missing")?;
    assert_eq!(
        plain_component.html_content,
        "<template>Plain package</template>"
    );
    assert!(plain_component.is_client_owned);

    fs::write(
        scoped.join("assets/scoped.template-webui.html"),
        "<template>Scoped package updated</template>",
    )?;
    fs::write(
        plain.join("lib/plain-entry.styles.css"),
        ":host { color: purple; }",
    )?;
    let changed = discover_source_with_plugin("@fixture/aggregate", root.path(), &plugin)?;
    let scoped_component = changed
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-scoped")
        .ok_or("updated scoped component missing")?;
    assert_eq!(
        scoped_component.html_content,
        "<template>Scoped package updated</template>"
    );
    let plain_component = changed
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-plain")
        .ok_or("updated plain component missing")?;
    assert_eq!(
        plain_component.css_content.as_deref(),
        Some(":host { color: purple; }")
    );

    fs::write(
        plain.join("package.json"),
        r#"{"exports":{"./component.js":"./lib/next-entry.js"}}"#,
    )?;
    fs::write(plain.join("lib/next-entry.js"), "export {};")?;
    fs::write(
        plain.join("lib/next-entry.template-webui.html"),
        "<template>Remapped package</template>",
    )?;
    let remapped = discover_source_with_plugin("@fixture/aggregate", root.path(), &plugin)?;
    let plain_component = remapped
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-plain")
        .ok_or("remapped plain component missing")?;
    assert_eq!(
        plain_component.html_content,
        "<template>Remapped package</template>"
    );

    fs::create_dir_all(aggregate.join("plain-module"))?;
    fs::write(aggregate.join("plain-module/component.js"), "export {};")?;
    fs::write(
        aggregate.join("plain-module/component.template-webui.html"),
        "<template>New local package path</template>",
    )?;
    let local_override = discover_source_with_plugin("@fixture/aggregate", root.path(), &plugin)?;
    let plain_component = local_override
        .components
        .iter()
        .find(|component| component.tag_name == "fixture-plain")
        .ok_or("local override component missing")?;
    assert_eq!(
        plain_component.html_content,
        "<template>New local package path</template>"
    );
    Ok(())
}

#[test]
fn fast_package_template_still_resolves_external_module_ownership() -> TestResult {
    let root = tempfile::tempdir()?;
    let node_modules = root.path().join("node_modules");
    let aggregate = node_modules.join("@fixture/single");
    fs::create_dir_all(&aggregate)?;
    fs::write(
        aggregate.join("package.json"),
        r#"{
        "customElements":"custom-elements.json",
        "dependencies":{"external-module":"1.0.0"},
        "exports":{"./template-webui.html":"./aggregate.template-webui.html"}
    }"#,
    )?;
    fs::write(
        aggregate.join("custom-elements.json"),
        r#"{
        "modules":[{
            "path":"external-module/component.js",
            "declarations":[{"name":"Card","tagName":"fixture-card"}]
        }]
    }"#,
    )?;
    fs::write(
        aggregate.join("aggregate.template-webui.html"),
        "<template>Aggregate template</template>",
    )?;

    let external = node_modules.join("external-module");
    fs::create_dir_all(external.join("dist"))?;
    fs::write(
        external.join("package.json"),
        r#"{"exports":{"./component.js":"./dist/component.js"}}"#,
    )?;
    fs::write(external.join("dist/component.js"), "export {};")?;
    fs::write(
        external.join("dist/component.template-webui.html"),
        "<template>External template</template>",
    )?;

    let plugin = FastDiscoveryPlugin::new();
    let result = discover_source_with_plugin("@fixture/single", root.path(), &plugin)?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(
        result.components[0].html_content,
        "<template>Aggregate template</template>"
    );
    assert!(result.components[0].is_client_owned);

    fs::write(
        external.join("package.json"),
        r#"{"exports":{"./component.js":null}}"#,
    )?;
    let error = discover_source_with_plugin("@fixture/single", root.path(), &plugin)
        .err()
        .ok_or("blocked external module must invalidate the aggregate cache")?;
    let message = format!("{error:#}");
    assert!(message.contains("fixture-card"));
    assert!(message.contains("@fixture/single"));
    Ok(())
}

#[test]
fn fast_bare_module_cache_tracks_nearer_package_candidates() -> TestResult {
    let root = tempfile::tempdir()?;
    let site = root.path().join("apps/site");
    let aggregate = site.join("node_modules/@fixture/aggregate");
    fs::create_dir_all(&aggregate)?;
    fs::write(
        aggregate.join("package.json"),
        r#"{
        "customElements":"custom-elements.json",
        "dependencies":{
            "plain-module":"1.0.0",
            "@external/scoped-module":"1.0.0"
        }
    }"#,
    )?;
    fs::write(
        aggregate.join("custom-elements.json"),
        r#"{
        "modules":[
            {
                "path":"plain-module/component.js",
                "declarations":[{"name":"PlainCard","tagName":"fixture-plain"}]
            },
            {
                "path":"@external/scoped-module/component.js",
                "declarations":[{"name":"ScopedCard","tagName":"fixture-scoped"}]
            }
        ]
    }"#,
    )?;

    fn write_external_package(package: &Path, template: &str) -> std::io::Result<()> {
        fs::create_dir_all(package.join("dist"))?;
        fs::write(
            package.join("package.json"),
            r#"{"exports":{"./component.js":"./dist/component.js"}}"#,
        )?;
        fs::write(package.join("dist/component.js"), "export {};")?;
        fs::write(package.join("dist/component.template-webui.html"), template)
    }

    write_external_package(
        &root.path().join("node_modules/plain-module"),
        "<template>Outer plain</template>",
    )?;
    write_external_package(
        &root.path().join("node_modules/@external/scoped-module"),
        "<template>Outer scoped</template>",
    )?;

    let plugin = FastDiscoveryPlugin::new();
    let first = discover_source_with_plugin("@fixture/aggregate", &site, &plugin)?;
    let first_templates: Vec<_> = first
        .components
        .iter()
        .map(|component| component.html_content.as_str())
        .collect();
    assert!(first_templates.contains(&"<template>Outer plain</template>"));
    assert!(first_templates.contains(&"<template>Outer scoped</template>"));

    write_external_package(
        &site.join("node_modules/plain-module"),
        "<template>Near plain</template>",
    )?;
    write_external_package(
        &site.join("node_modules/@external/scoped-module"),
        "<template>Near scoped</template>",
    )?;

    let changed = discover_source_with_plugin("@fixture/aggregate", &site, &plugin)?;
    let changed_templates: Vec<_> = changed
        .components
        .iter()
        .map(|component| component.html_content.as_str())
        .collect();
    assert!(changed_templates.contains(&"<template>Near plain</template>"));
    assert!(changed_templates.contains(&"<template>Near scoped</template>"));
    assert!(!changed_templates.contains(&"<template>Outer plain</template>"));
    assert!(!changed_templates.contains(&"<template>Outer scoped</template>"));
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
