// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::fs;

#[test]
fn fast_local_template_uses_component_filename_prefix() {
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("todo-item.template-webui.html"),
        "<template>item</template>",
    )
    .unwrap();
    fs::write(
        root.path().join("todo-item.template.html"),
        "<template>Alternate</template>",
    )
    .unwrap();

    let components = FastDiscoveryPlugin::new()
        .discover_local(root.path())
        .unwrap();

    assert_eq!(components.len(), 1);
    assert_eq!(components[0].tag_name, "todo-item");
}

#[test]
fn virtual_root_module_candidates_stay_inside_package() {
    let root = tempfile::TempDir::new().unwrap();
    let candidates = fast_template_candidates(root.path(), Path::new("index.js"), "MyButton");

    assert!(candidates
        .iter()
        .all(|candidate| candidate.starts_with(root.path())));
    assert!(candidates.contains(
        &root
            .path()
            .join("my-button")
            .join("my-button.template-webui.html")
    ));
    assert!(
        fast_template_candidates(root.path(), Path::new("../escape/index.js"), "Escape").is_empty()
    );
}

#[test]
fn undeclared_unscoped_module_stays_package_relative_when_name_is_hoisted() {
    let root = tempfile::TempDir::new().unwrap();
    let aggregate = root.path().join("node_modules/aggregate");
    let hoisted = root.path().join("node_modules/dist");
    fs::create_dir_all(&aggregate).unwrap();
    fs::create_dir_all(&hoisted).unwrap();
    fs::write(
        hoisted.join("package.json"),
        r#"{"exports":{"./button.js":"./external.js"}}"#,
    )
    .unwrap();
    let manifest = serde_json::json!({"name":"aggregate"});
    let package = PackageContext {
        name: "aggregate",
        root: &aggregate,
        manifest: Some(&manifest),
    };
    let declaration = ComponentDeclaration {
        tag_name: "fixture-button".to_string(),
        name: Some("Button".to_string()),
        module_specifier: Some("dist/button.js".to_string()),
    };

    let resolved = resolve_declaration_module(package, &declaration, true).unwrap();

    assert_eq!(resolved.root, aggregate);
    assert_eq!(resolved.relative_path, PathBuf::from("dist/button.js"));
}

#[test]
fn package_relative_inferred_template_symlink_cannot_escape_package() {
    let root = tempfile::TempDir::new().unwrap();
    let aggregate = root.path().join("aggregate");
    let external = root.path().join("external");
    let outside = root.path().join("outside");
    fs::create_dir_all(&aggregate).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(external.join("component.js"), "export {};").unwrap();
    fs::write(
        outside.join("component.template-webui.html"),
        "<template>Outside</template>",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.join("component.template-webui.html"),
        external.join("component.template-webui.html"),
    )
    .unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(
        outside.join("component.template-webui.html"),
        external.join("component.template-webui.html"),
    )
    .unwrap();
    let manifest = serde_json::json!({"name":"aggregate"});
    let package = PackageContext {
        name: "aggregate",
        root: &aggregate,
        manifest: Some(&manifest),
    };
    let declaration = ComponentDeclaration {
        tag_name: "fixture-card".to_string(),
        name: Some("Card".to_string()),
        module_specifier: Some("external/component.js".to_string()),
    };
    let module = ResolvedCemModule {
        root: external,
        relative_path: PathBuf::from("component.js"),
        package_json: None,
        resolution_dependencies: Vec::new(),
        exported_template: None,
        exported_styles: None,
        is_client_owned: false,
    };

    let error = inferred_template(package, &declaration, &module).unwrap_err();

    assert!(format!("{error:#}").contains("outside package"));
}

#[test]
fn package_self_reference_resolves_against_containing_root() {
    let root = tempfile::TempDir::new().unwrap();
    let package_root = root.path().join("package");
    fs::create_dir_all(&package_root).unwrap();
    let manifest = serde_json::json!({
        "name": "self-package",
        "exports": {
            "./component.js": "./dist/component.js"
        }
    });
    fs::write(
        package_root.join("package.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    let package = PackageContext {
        name: "self-package",
        root: &package_root,
        manifest: Some(&manifest),
    };
    let declaration = ComponentDeclaration {
        tag_name: "fixture-card".to_string(),
        name: Some("Card".to_string()),
        module_specifier: Some("self-package/component.js".to_string()),
    };

    let resolved = resolve_declaration_module(package, &declaration, true).unwrap();

    assert_eq!(resolved.root, package_root);
    assert_eq!(resolved.relative_path, PathBuf::from("./dist/component.js"));
}
