// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn stage_for_build(crate_root: &Path, out: &Path, version: &str) -> io::Result<Vec<PathBuf>> {
    let checkout = crate_root.join("../..");
    if checkout
        .join("packages/webui-framework/package.json")
        .is_file()
    {
        stage_checkout(&checkout, out, version)
    } else {
        let packaged = crate_root.join("assets");
        validate_version(&packaged.join("framework/package.json"), version)?;
        let mut inputs = Vec::new();
        copy_tree(&packaged, out, CopyKind::All, &mut inputs)?;
        inputs.push(packaged);
        Ok(inputs)
    }
}

pub fn stage_checkout(checkout: &Path, out: &Path, version: &str) -> io::Result<Vec<PathBuf>> {
    let framework = checkout.join("packages/webui-framework");
    let manifest = framework.join("package.json");
    let mut metadata = validate_version(&manifest, version)?;
    let exports = metadata
        .get_mut("exports")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| invalid("framework package has no export map"))?;
    for (_, value) in exports {
        let import = value
            .get_mut("import")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| invalid("framework exports must select an ESM import"))?;
        for field in ["default", "types"] {
            let target = import
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid("framework export has no source target"))?;
            let source = target
                .strip_prefix("./dist/")
                .ok_or_else(|| invalid("framework export must target dist/"))?;
            let source = source
                .strip_suffix(".d.ts")
                .or_else(|| source.strip_suffix(".js"))
                .ok_or_else(|| invalid("framework export must target a JavaScript module"))?;
            import.insert(
                field.into(),
                serde_json::Value::from(format!("./src/{source}.ts")),
            );
        }
    }
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| invalid("framework metadata must be an object"))?;
    object.retain(|name, _| {
        matches!(
            name.as_str(),
            "name" | "version" | "type" | "license" | "exports" | "sideEffects"
        )
    });
    if let Some(side_effects) = object
        .get_mut("sideEffects")
        .and_then(serde_json::Value::as_array_mut)
    {
        for item in side_effects {
            if let Some(path) = item
                .as_str()
                .and_then(|path| path.strip_prefix("./dist/"))
                .and_then(|path| path.strip_suffix(".js"))
            {
                *item = serde_json::Value::from(format!("./src/{path}.ts"));
            }
        }
    }
    let framework_sources = framework.join("src");
    let docs = checkout.join("docs");
    let skill = checkout.join("ai/SKILL.md");
    let license = checkout.join("LICENSE");
    let mut inputs = vec![manifest];
    copy_tree(
        &framework_sources,
        &out.join("framework/src"),
        CopyKind::TypeScript,
        &mut inputs,
    )?;
    write(
        &out.join("framework/package.json"),
        &serde_json::to_vec_pretty(&metadata)?,
    )?;
    write(&out.join("framework/tsconfig.json"), br#"{"compilerOptions":{"target":"ES2022","experimentalDecorators":true,"useDefineForClassFields":false}}"#)?;
    copy_tree(
        &docs,
        &out.join("reference/docs"),
        CopyKind::Markdown,
        &mut inputs,
    )?;
    for (source, target) in [
        (&docs.join("ai.md"), out.join("reference/ai.md")),
        (&skill, out.join("reference/SKILL.md")),
        (&license, out.join("LICENSE")),
    ] {
        write(&target, &fs::read(source)?)?;
    }
    inputs.extend([framework_sources, docs, skill, license]);
    Ok(inputs)
}

fn validate_version(path: &Path, version: &str) -> io::Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value["name"] != "@microsoft/webui-framework" || value["version"] != version {
        return Err(invalid("native framework resources must match the CLI release; stage canonical resources before packaging"));
    }
    Ok(value)
}

#[derive(Clone, Copy)]
enum CopyKind {
    All,
    TypeScript,
    Markdown,
}

fn copy_tree(
    input: &Path,
    output: &Path,
    kind: CopyKind,
    inputs: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let mut pending = vec![(input.to_owned(), output.to_owned())];
    while let Some((source, target)) = pending.pop() {
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid("native resource inputs must not be symlinks"));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&source)? {
                let entry = entry?;
                let name = entry.file_name();
                let text = name.to_string_lossy();
                if text.starts_with('.')
                    || matches!(text.as_ref(), "node_modules" | "dist" | "dist-test")
                {
                    continue;
                }
                pending.push((entry.path(), target.join(name)));
            }
        } else if metadata.is_file() {
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("native resource names must be UTF-8"))?;
            let selected = match kind {
                CopyKind::All => true,
                CopyKind::TypeScript => {
                    name.ends_with(".ts")
                        && !name.ends_with(".test.ts")
                        && !name.ends_with(".spec.ts")
                }
                CopyKind::Markdown => name.ends_with(".md"),
            };
            if selected {
                write(&target, &fs::read(&source)?)?;
                inputs.push(source);
            }
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("resource output has no parent"))?,
    )?;
    fs::write(path, bytes)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
