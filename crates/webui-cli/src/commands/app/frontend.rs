// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use microsoft_webui_client_build::{build, ClientBuildOptions, SourceMap};

use super::config::{validate_relative, Project};
use crate::commands::native::control::Stop;
use crate::commands::native::{self, NativeError};
use crate::utils::output;

const STAGE: &str = ".webui/build";
const BACKUP: &str = ".webui/previous";

pub(super) fn prepare_resources(project: &Project) -> Result<PathBuf> {
    let framework = native::materialize(&project.root, "framework")?;
    let metadata = native::read_metadata(&framework.join("package.json"))?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata)
        .map_err(|error| NativeError::Assets(error.to_string()))?;
    let exports = metadata["exports"]
        .as_object()
        .ok_or_else(|| NativeError::Assets("framework has no export map".into()))?;
    let mut paths = BTreeMap::new();
    for (specifier, export) in exports {
        let target = export["import"]["default"]
            .as_str()
            .ok_or_else(|| NativeError::Assets("framework export has no source target".into()))?;
        let suffix = if specifier == "." {
            ""
        } else {
            specifier
                .strip_prefix('.')
                .ok_or_else(|| NativeError::Assets("invalid framework export".into()))?
        };
        let target = framework.join(target);
        let relative = target.strip_prefix(project.root.join(".webui"))?;
        paths.insert(
            format!("@microsoft/webui-framework{suffix}"),
            vec![format!(
                "./{}",
                relative.to_string_lossy().replace('\\', "/")
            )],
        );
    }
    let mut options = serde_json::Map::new();
    options.insert("paths".into(), serde_json::to_value(paths)?);
    let mut config = serde_json::Map::new();
    config.insert("compilerOptions".into(), serde_json::Value::Object(options));
    native::write_metadata(
        &project.root,
        Path::new(".webui/tsconfig.framework.json"),
        &serde_json::Value::Object(config),
    )?;
    Ok(framework)
}

pub(super) async fn build_frontend(
    project: &Project,
    development: bool,
    stop: &mut Stop,
) -> Result<()> {
    native::ensure_directory(&project.root, Path::new(".webui"))?;
    if project.root.join(BACKUP).exists() {
        return Err(NativeError::Assets("an interrupted promotion left .webui/previous; inspect and restore that generation before rebuilding".into()).into());
    }
    remove_stage(project)?;
    native::ensure_directory(&project.root, Path::new(".webui/build/public"))?;
    let options = webui::BuildOptions {
        app_dir: project.source(),
        entry: project.config.entry.clone(),
        plugin: Some(webui::Plugin::WebUI),
        css: webui::CssStrategy::Link,
        asset_base: Some("/assets/".into()),
        modules_out: Some(project.root.join(".webui/modules")),
        ..Default::default()
    };
    let mut outputs = BTreeMap::new();
    let mut written = BTreeSet::new();
    let result = if let Some(entry) = &project.config.browser {
        let framework = prepare_resources(project)?;
        let prepared = webui::prepare_build(options)?;
        native::ensure_directory(&project.root, Path::new(".webui/modules"))?;
        prepared.write_modules()?;
        let client = tokio::select! {
            reason = stop.requested() => {
                reason?;
                return Err(NativeError::Cancelled.into());
            }
            result = build(ClientBuildOptions {
                project: project.root.clone(),
                entries: BTreeMap::from([("index".into(), project.config.app.join(entry))]),
                out_dir: Path::new(STAGE).join("public"),
                framework: Some(framework),
                development,
                minify: !development,
                sourcemap: if development { SourceMap::File } else { SourceMap::None },
                ..Default::default()
            }) => result.map_err(NativeError::Build)?,
        };
        let manifest_path = Path::new(STAGE).join("projection.json");
        for warning in &client.warnings {
            output::native_build_warning(&warning.code, &warning.message);
        }
        let manifest = client
            .projection_manifest(&manifest_path)
            .map_err(NativeError::Build)?;
        for (path, content) in client.files {
            insert(&mut outputs, path, content)?;
        }
        write_outputs(project, &outputs)?;
        written.extend(outputs.keys().cloned());
        prepared.finish(vec![webui::ProjectionManifestSource::Inline {
            manifest_path: project.root.join(manifest_path),
            json: serde_json::to_string(&manifest)?,
        }])?
    } else {
        let result = webui::build(options)?;
        if result
            .component_templates
            .iter()
            .any(|component| component.is_scripted)
        {
            return Err(NativeError::Config("scripted components require a browser entry; HTML/CSS-only builds must contain no component scripts".into()).into());
        }
        result
    };
    for (name, content) in &result.css_files {
        validate_relative(Path::new(name))?;
        insert(
            &mut outputs,
            Path::new(STAGE).join("public").join(name),
            content.as_bytes().to_vec(),
        )?;
    }
    for pack in &result.lazy_pack_files {
        validate_relative(Path::new(&pack.file_name))?;
        insert(
            &mut outputs,
            Path::new(STAGE)
                .join("public")
                .join(webui::PACK_OUTPUT_DIR)
                .join(&pack.file_name),
            pack.content.as_bytes().to_vec(),
        )?;
    }
    insert(
        &mut outputs,
        Path::new(STAGE).join("public/webui-LICENSE.txt"),
        native::license()?.to_vec(),
    )?;
    insert(
        &mut outputs,
        Path::new(STAGE).join("protocol.bin"),
        result.protocol_bytes,
    )?;
    outputs.retain(|path, _| !written.contains(path));
    write_outputs(project, &outputs)?;
    for warning in &result.warnings {
        output::warning_diagnostic(warning);
    }
    Ok(())
}

fn insert(outputs: &mut BTreeMap<PathBuf, Vec<u8>>, path: PathBuf, bytes: Vec<u8>) -> Result<()> {
    if outputs.insert(path.clone(), bytes).is_some() {
        return Err(NativeError::Assets(format!(
            "generated output collision at {}",
            path.display()
        ))
        .into());
    }
    Ok(())
}

fn write_outputs(project: &Project, outputs: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    for (relative, bytes) in outputs {
        validate_relative(relative)?;
        let parent = relative
            .parent()
            .ok_or_else(|| NativeError::Assets("output has no parent".into()))?;
        native::ensure_directory(&project.root, parent)?;
        let path = project.root.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(NativeError::Assets(format!(
                    "generated file is redirected: {}",
                    path.display()
                ))
                .into())
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(native::io(&path, error).into())
            }
            _ => {}
        }
        fs::write(&path, bytes).map_err(|source| native::io(&path, source))?;
    }
    Ok(())
}

fn remove_stage(project: &Project) -> Result<()> {
    let path = project.root.join(STAGE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(&path).map_err(|source| native::io(&path, source))?
        }
        Ok(_) => {
            return Err(NativeError::Assets(
                "native staging directory is redirected or not a directory".into(),
            )
            .into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(native::io(&path, source).into()),
    }
    Ok(())
}

pub(super) fn publish(project: &Project) -> Result<()> {
    let dist = project.root.join("dist");
    let backup = project.root.join(BACKUP);
    if backup.exists() {
        return Err(NativeError::Assets(
            "previous generation already exists; inspect .webui/previous before promotion".into(),
        )
        .into());
    }
    let previous = match fs::symlink_metadata(&dist) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::rename(&dist, &backup).map_err(|source| native::io(&dist, source))?;
            true
        }
        Ok(_) => {
            return Err(NativeError::Assets(
                "dist must be an ordinary generated directory, not a redirected path".into(),
            )
            .into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => return Err(native::io(&dist, source).into()),
    };
    if let Err(source) = fs::rename(project.root.join(STAGE), &dist) {
        if previous {
            if let Err(rollback) = fs::rename(&backup, &dist) {
                return Err(NativeError::Assets(format!(
                    "promotion failed ({source}) and rollback failed ({rollback}); preserve .webui/previous and dist for recovery")).into());
            }
        }
        return Err(native::io(&dist, source).into());
    }
    if previous {
        fs::remove_dir_all(&backup).map_err(|source| native::io(&backup, source))?;
    }
    Ok(())
}
