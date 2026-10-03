// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::path::{Path, PathBuf};
use webui::{ComponentAssetFile, WebUIError};

pub(super) use webui::component_asset_output::manifest_path;

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
