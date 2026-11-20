// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use webui::browser_build::{build, ClientBuildOptions};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn options(root: &Path) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("index".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn explicit_defines_override_the_default_development_policy() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("index.ts"),
        "globalThis.result = __WEBUI_DEV__ ? 'EXPLICIT_OVERRIDE' : 'DEFAULT_PRODUCTION';",
    )?;
    let mut config = options(temp.path());
    config.defines.insert("__WEBUI_DEV__".into(), "true".into());
    let result = build(config).await?;
    let code = String::from_utf8_lossy(&result.files[Path::new("dist/index.js")]);
    assert!(code.contains("EXPLICIT_OVERRIDE"), "{code}");
    assert!(!code.contains("DEFAULT_PRODUCTION"), "{code}");
    Ok(())
}

#[tokio::test]
async fn package_external_boundaries_cover_subpaths_but_aliases_still_win() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fs::write(root.join("index.ts"), "export { remote } from 'external-package/subpath'; export { local } from 'external-package/local'; export { other } from 'external-package-extra';")?;
    fs::write(root.join("local.ts"), "export const local = 1;")?;
    fs::write(root.join("other.ts"), "export const other = 2;")?;
    let mut config = options(root);
    config.external.push("external-package".into());
    config
        .aliases
        .insert("external-package/local$".into(), "./local.ts".into());
    config
        .aliases
        .insert("external-package-extra".into(), "./other.ts".into());
    let result = build(config).await?;
    let imports = &result.metadata.outputs["dist/index.js"].imports;
    assert_eq!(imports.len(), 1);
    assert!(imports[0].external);
    assert_eq!(imports[0].path, "external-package/subpath");
    Ok(())
}

#[tokio::test]
async fn dependency_original_typescript_uses_native_legacy_decorator_lowering() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let package = root.join("node_modules/component-library");
    let framework = root.join("node_modules/@microsoft/webui-framework");
    fs::create_dir_all(&package)?;
    fs::create_dir_all(&framework)?;
    fs::write(
        package.join("package.json"),
        r#"{"name":"component-library","type":"module","exports":"./index.ts"}"#,
    )?;
    fs::write(
        framework.join("package.json"),
        r#"{"name":"@microsoft/webui-framework","type":"module","exports":"./index.js"}"#,
    )?;
    fs::write(framework.join("index.js"), "export const observable = (target, key) => { Object.defineProperty(target, key, {set(value) {this._value=value;}, get() {return this._value;}}); };")?;
    let source = "import { observable } from '@microsoft/webui-framework'; export class Counter { @observable count: number = 0; } Counter.define('library-counter');";
    fs::write(package.join("index.ts"), source)?;
    fs::write(root.join("index.ts"), "import 'aliased-library';")?;
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"experimentalDecorators":true,"useDefineForClassFields":false}}"#,
    )?;
    let mut config = options(&root);
    config.tsconfig = Some("tsconfig.json".into());
    config.aliases.insert(
        "aliased-library".into(),
        "./node_modules/component-library/index.ts".into(),
    );
    let result = build(config).await?;
    let entry = result.graph.entries.first().ok_or("missing entry module")?;
    let edge = result.graph.modules[entry]
        .imports
        .first()
        .ok_or("missing library import")?;
    assert_eq!(edge.specifier, "aliased-library");
    assert_eq!(
        Path::new(&edge.resolved).canonicalize()?,
        package.join("index.ts")
    );
    assert_eq!(
        result.graph.modules[&edge.resolved].source.as_deref(),
        Some(source)
    );
    let code = String::from_utf8_lossy(&result.files[Path::new("dist/index.js")]);
    let allocator = oxc::allocator::Allocator::default();
    let parsed = oxc::parser::Parser::new(&allocator, &code, oxc::span::SourceType::mjs()).parse();
    let built = oxc::semantic::SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program);
    assert!(
        !built
            .semantic
            .nodes()
            .iter()
            .any(|node| matches!(node.kind(), oxc::ast::AstKind::Decorator(_))),
        "{code}"
    );
    let proof = result.projection_manifest(Path::new("dist/projection.json"))?;
    assert_eq!(
        proof.components["library-counter"].hydration_keys,
        ["count"]
    );
    Ok(())
}

#[tokio::test]
async fn script_grammar_capture_preserves_commonjs_module_kind() -> TestResult {
    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("index.ts"),
        "import {value} from './dependency.cjs'; export {value};",
    )?;
    fs::write(
        temp.path().join("dependency.cjs"),
        "if (globalThis.skip) return; exports.value = 42; globalThis.defineMarker = true;",
    )?;
    let result = build(options(temp.path())).await?;
    result.projection_manifest(Path::new("dist/projection.json"))?;
    Ok(())
}
