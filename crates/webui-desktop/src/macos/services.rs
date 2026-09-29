// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::Path;
use std::process::{Command, Stdio};

use super::NativeServiceError;

pub(super) fn open_url(url: &str) -> Result<(), NativeServiceError> {
    // Fixed OS binary and argv: no shell, interpolation, or renderer path.
    open(url)
}

pub(super) fn open_document(path: &Path) -> Result<(), NativeServiceError> {
    open(path.as_os_str())
}

fn open(target: impl AsRef<std::ffi::OsStr>) -> Result<(), NativeServiceError> {
    let status = Command::new("/usr/bin/open")
        .arg("--")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| NativeServiceError::Os(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(NativeServiceError::Os(format!(
            "system opener exited with {status}"
        )))
    }
}
