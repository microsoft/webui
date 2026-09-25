// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{DesktopError, Result};

#[path = "../runtime/deployment.rs"]
mod deployment;

pub(super) fn validate_inputs(runner: &Path, output: &Path) -> Result<Vec<PathBuf>> {
    require_file(runner)?;
    let directory = runner
        .parent()
        .ok_or_else(|| DesktopError::InvalidAssetPath {
            path: runner.display().to_string(),
        })?;
    let mut files = Vec::with_capacity(deployment::NOTICES.len() + 1);
    for name in std::iter::once(deployment::BOOTSTRAP_DLL).chain(deployment::NOTICES) {
        let path = directory.join(name);
        if path == runner {
            return Err(DesktopError::InvalidAssetPath {
                path: format!("runner name is reserved for Windows App SDK deployment: {name}"),
            });
        }
        super::validate_input_overlap(output, &path, "Windows App SDK deployment file")?;
        require_file(&path)?;
        files.push(path);
    }
    Ok(files)
}

fn require_file(path: &Path) -> Result<()> {
    let validate = || -> io::Result<()> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a nonempty regular file",
            ));
        }
        Ok(())
    };
    validate().map_err(|source| DesktopError::Io {
        context: format!(
            "validating Windows portable input {}; rebuild the runner with the Windows native feature and keep its matching bootstrap DLL, license, notices, and provenance beside it (also required for custom runners)",
            path.display()
        ),
        source,
    })
}
