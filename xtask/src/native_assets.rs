// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;

#[path = "../../crates/webui-cli/resources/build_support.rs"]
mod resources;

pub(crate) fn stage(root: &Path) -> Result<(), String> {
    let cli = root.join("crates/webui-cli");
    let assets = cli.join("assets");
    if assets.exists() {
        fs::remove_dir_all(&assets).map_err(|error| error.to_string())?;
    }
    resources::stage_for_build(
        &cli,
        &assets,
        &crate::version::workspace_package_version(root)?,
    )
    .map_err(|error| format!("cannot stage native CLI resources: {error}"))?;
    Ok(())
}
