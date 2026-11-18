// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use webui::browser_build::{build, ClientBuildOptions, ImportKind, SourceMap};

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

#[tokio::test]
async fn parallel_groups_do_not_assimilate_dependency_or_target_exports() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    fixture(root)?;
    fs::write(root.join("package.json"), r#"{"type":"module"}"#)?;
    fs::write(
        root.join("a.ts"),
        r#"import { Shared } from './shared.ts';
        import { then as direct } from './direct.ts';
        import { then as named } from './named.ts';
        import { then as quoted } from './quoted.ts';
        import { then as star } from './star.ts';
        import { label } from './opaque.ts';
        export const value = new Shared()[globalThis.method](
            direct('direct') + named('named') + quoted('quoted') + star('star') + label
        );"#,
    )?;
    fs::OpenOptions::new()
        .append(true)
        .open(root.join("b.ts"))?
        .write_all(b"\nexport { resolveResult as then } from './target-result.ts';")?;
    fs::write(
        root.join("target-result.ts"),
        r#"export function resolveResult(resolve) {
            resolve({
                value: 'authored result',
                get then() { globalThis.targetThenReads++; }
            });
        }"#,
    )?;
    let callable = r#"(value) {
        if (typeof value !== 'string') throw Error('unexpected then call on a static dependency');
        return value + '-';
    }"#;
    for (name, source) in [
        ("direct", format!("export function then{callable}")),
        (
            "named",
            format!("function impl{callable} export {{ impl as then }};"),
        ),
        (
            "quoted",
            format!(r#"function impl{callable} export {{ impl as "th\u0065n" }};"#),
        ),
        ("star", "export * from './star-source.ts';".into()),
        ("star-source", format!("export function then{callable}")),
        (
            "opaque",
            "export * from './opaque-barrel.ts'; export const label = 'opaque';".into(),
        ),
        ("opaque-barrel", "export * from 'runtime-only';".into()),
    ] {
        fs::write(root.join(format!("{name}.ts")), source)?;
    }
    let external = root.join("node_modules/runtime-only");
    fs::create_dir_all(&external)?;
    fs::write(
        external.join("package.json"),
        r#"{"exports":"./index.mjs"}"#,
    )?;
    fs::write(
        external.join("index.mjs"),
        format!("export function then{callable}"),
    )?;

    let mut config = options(root, false);
    for name in ["direct", "named", "quoted", "star", "opaque"] {
        config
            .entries
            .insert(format!("shared/{name}"), format!("{name}.ts").into());
    }
    config.external.push("runtime-only".into());
    let mut ordinary_output = Vec::new();
    for parallel in [false, true] {
        let out = if parallel { "parallel" } else { "ordinary" };
        config.out_dir = out.into();
        config.parallel_dynamic_imports = parallel;
        let result = build(config.clone()).await?;
        result.write_outputs(&Path::new(out).join("projection.json"))?;
        let executed = Command::new("node")
            .args([
                "--input-type=module",
                "--eval",
                r#"import assert from 'node:assert/strict';
                globalThis.method = 'method0';
                globalThis.targetThenReads = 0;
                const { first, second } = await import(process.argv[1]);
                assert.equal((await first()).value, 'direct-named-quoted-star-opaque-shared-0');
                assert.equal((await second()).value, 'authored result');
                console.log(globalThis.targetThenReads);"#,
            ])
            .arg(format!("./{out}/pages/main.js"))
            .current_dir(root)
            .output()?;
        assert!(
            executed.status.success(),
            "{out} emitted JavaScript failed: {}",
            String::from_utf8_lossy(&executed.stderr)
        );
        if !parallel {
            ordinary_output = executed.stdout;
            continue;
        }
        assert_eq!(
            String::from_utf8_lossy(&executed.stdout),
            String::from_utf8_lossy(&ordinary_output),
            "parallel mode must not re-assimilate the authored target's result"
        );

        let entry = &result.metadata.outputs[&format!("{out}/pages/main.js")];
        assert_eq!(entry.parallel_dynamic_imports.len(), 1);
        let (target, dependencies) = entry
            .parallel_dynamic_imports
            .first_key_value()
            .ok_or("only the ordinary lazy target should have a parallel plan")?;
        assert!(target.starts_with(&format!("{out}/chunks/a-")), "{target}");
        let safe = dependencies
            .iter()
            .find(|path| path.starts_with(&format!("{out}/chunks/shared-")))
            .ok_or("the safe shared chunk should still be injected")?;
        let static_edges = &result.metadata.outputs[target].imports;
        for name in ["direct", "named", "quoted", "star", "opaque"] {
            let path = format!("{out}/shared/{name}.js");
            assert!(
                static_edges.iter().any(|edge| edge.path == path
                    && !edge.external
                    && matches!(edge.kind, ImportKind::Static)),
                "missing native static edge: {path}; got {static_edges:?}"
            );
            assert!(
                !entry.imports.iter().any(|edge| edge.path == path),
                "unexpected injected edge: {path}"
            );
        }
        assert!(entry.imports.iter().any(|edge| edge.path == *safe
            && !edge.external
            && matches!(edge.kind, ImportKind::Dynamic)));
    }
    Ok(())
}
