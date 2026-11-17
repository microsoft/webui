// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

use microsoft_webui_client_build::{build, ClientBuildOptions, ImportKind, SourceMap};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixture(root: &Path) -> TestResult {
    fs::write(root.join("index.ts"), "export function first(Promise) { return import('./a.ts'); } export const second = () => import('./b.ts');")?;
    for (name, value) in [("a", "alpha"), ("b", "beta")] {
        fs::write(root.join(format!("{name}.ts")), format!("import {{ Shared }} from './shared.ts'; export const value = new Shared()[globalThis.method]('{value}');"))?;
    }
    let mut shared = String::from("export class Shared {");
    for index in 0..100 {
        shared.push_str(&format!(
            "method{index}(value) {{ return value + '-shared-{index}'; }}"
        ));
    }
    shared.push('}');
    fs::write(root.join("shared.ts"), shared)?;
    Ok(())
}

fn options(root: &Path, parallel: bool) -> ClientBuildOptions {
    ClientBuildOptions {
        project: root.to_owned(),
        entries: [("pages/main".into(), "index.ts".into())].into(),
        out_dir: "dist".into(),
        chunk_names: "chunks/[name]-[hash].js".into(),
        parallel_dynamic_imports: parallel,
        sourcemap: SourceMap::File,
        ..Default::default()
    }
}

#[tokio::test]
async fn opt_in_parallel_groups_keep_static_edges_and_hash_actual_injected_imports() -> TestResult {
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    let ordinary = build(options(temp.path(), false)).await?;
    assert!(ordinary
        .metadata
        .outputs
        .values()
        .all(|output| output.parallel_dynamic_imports.is_empty()));
    let result = build(options(temp.path(), true)).await?;
    let entry = &result.metadata.outputs["dist/pages/main.js"];
    let code = String::from_utf8_lossy(&result.files[Path::new("dist/pages/main.js")]);
    assert!(code.contains(".constructor.all"), "{code}");
    assert!(
        !code.contains("!~{"),
        "preliminary hash placeholders must be finalized"
    );
    assert_eq!(entry.parallel_dynamic_imports.len(), 2);
    for (target, dependencies) in &entry.parallel_dynamic_imports {
        assert!(
            !dependencies.is_empty(),
            "{target} must reproduce the shared-chunk case"
        );
        let target_metadata = &result.metadata.outputs[target];
        for dependency in dependencies {
            assert!(target_metadata
                .imports
                .iter()
                .any(|edge| edge.path == *dependency
                    && !edge.external
                    && matches!(edge.kind, ImportKind::Static)));
            assert!(entry.imports.iter().any(|edge| edge.path == *dependency
                && !edge.external
                && matches!(edge.kind, ImportKind::Dynamic)));
            let filename = Path::new(dependency)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("filename")?;
            assert!(
                code.contains(filename),
                "injected call must reference final {dependency}: {code}"
            );
        }
    }
    let proof = result.projection_manifest(Path::new("dist/projection.json"))?;
    let ordinary_proof = ordinary.projection_manifest(Path::new("dist/projection.json"))?;
    assert_ne!(proof.build_id, ordinary_proof.build_id);
    assert!(
        proof.entry_closures["dist/pages/main.js"].is_empty(),
        "parallel targets must remain deferred"
    );
    assert!(result
        .files
        .contains_key(Path::new("dist/pages/main.js.map")));
    Ok(())
}

#[tokio::test]
async fn parallel_groups_honor_public_paths_and_rehash_when_the_shared_module_changes() -> TestResult
{
    let temp = tempfile::tempdir()?;
    fixture(temp.path())?;
    let mut config = options(temp.path(), true);
    config.public_path = Some("/static/".into());
    let first = build(config.clone()).await?;
    let entry = &first.metadata.outputs["dist/pages/main.js"];
    let code = String::from_utf8_lossy(&first.files[Path::new("dist/pages/main.js")]);
    for dependency in entry.parallel_dynamic_imports.values().flatten() {
        let suffix = dependency.strip_prefix("dist/").ok_or("output root")?;
        assert!(code.contains(&format!("/static/{suffix}")), "{code}");
    }
    fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("shared.ts"))?
        .write_all(b"\nglobalThis.sharedChanged = true;")?;
    let second = build(config).await?;
    assert_ne!(
        first.files[Path::new("dist/pages/main.js")],
        second.files[Path::new("dist/pages/main.js")]
    );
    assert_ne!(
        first
            .projection_manifest(Path::new("dist/projection.json"))?
            .build_id,
        second
            .projection_manifest(Path::new("dist/projection.json"))?
            .build_id
    );
    Ok(())
}

use std::io::Write;
