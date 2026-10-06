// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use webui::{ComponentAssetFile, WebUIError};

use super::build::output_paths;

pub(super) fn publish_with<F>(
    output_dir: &Path,
    files: &[ComponentAssetFile],
    finish: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    webui::component_asset_output::publish_with(output_dir, files, || {
        finish().map_err(|source| WebUIError::ComponentAssetPublication {
            context: "Failed to finalize component asset outputs".to_string(),
            source: source.into_boxed_dyn_error(),
        })
    })?;
    Ok(())
}

pub(super) fn watch_ignore_paths(output_dir: &Path) -> [PathBuf; 1] {
    [output_dir.to_path_buf()]
}

pub(super) fn validate_metafile_path(
    output_dir: &Path,
    metafile: &Path,
    files: &[ComponentAssetFile],
) -> Result<()> {
    for file in files {
        reject_metafile_collision(output_dir, metafile, &output_dir.join(&file.name))?;
    }
    for asset in existing_asset_paths(output_dir)? {
        reject_metafile_collision(output_dir, metafile, &asset)?;
    }
    Ok(())
}

fn reject_metafile_collision(output_dir: &Path, metafile: &Path, asset: &Path) -> Result<()> {
    if output_paths::paths_collide(asset, metafile)? {
        anyhow::bail!(
            "Metafile output {} collides with generated component asset {}.\nhelp: Choose a distinct --metafile path outside {}.",
            metafile.display(),
            asset.display(),
            output_dir.display()
        );
    }
    Ok(())
}

fn existing_asset_paths(output_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_existing_assets(output_dir, &mut paths)?;
    collect_existing_assets(&output_dir.join("components"), &mut paths)?;
    Ok(paths)
}

fn collect_existing_assets(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".webui.js"))
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}
