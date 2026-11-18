// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-only HTTPS transport. The Rust owner always checks frozen SHA-512
//! after this helper returns and before interpreting any downloaded bytes.

use std::fs;
use std::path::Path;
use std::process::Stdio;

use crate::native_toolchain::producer_go::command as go_command;

/// Acquire a first-party artifact, never execute a target compiler or use Node.
pub(super) fn acquire(url: &str, output: &Path) -> Result<(), String> {
    let root = output.parent().ok_or("checker download has no parent")?;
    let source = root.join("download.go");
    fs::write(&source, include_str!("download.go")).map_err(|e| e.to_string())?;
    let mut command = go_command(root)?;
    let status = command
        .args(["run", "-p=2", "-trimpath"])
        .arg(source)
        .arg(url)
        .arg(output)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| {
            format!("explicit checker acquisition requires the native Go toolchain: {e}")
        })?;
    if !status.success() {
        return Err(format!(
            "could not acquire the pinned first-party checker asset from {url} ({status}); \
             provide its frozen-hash-matching tarball with --checker-package for offline staging"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_go_environment_is_isolated_and_runner_native() -> Result<(), String> {
        let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
        let command = go_command(temporary.path())?;
        let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        for key in ["GOOS", "GOARCH", "GOARM64", "GOAMD64"] {
            assert_eq!(environment.get(std::ffi::OsStr::new(key)), Some(&None));
        }
        assert_eq!(
            environment.get(std::ffi::OsStr::new("GOTOOLCHAIN")),
            Some(&Some(std::ffi::OsStr::new("local")))
        );
        assert!(!environment.contains_key(std::ffi::OsStr::new("GOSUMDB")));
        Ok(())
    }

    #[test]
    fn go_https_transport_guards() -> Result<(), String> {
        let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
        let source = temporary.path().join("download.go");
        let tests = temporary.path().join("download_test.go");
        fs::write(&source, include_str!("download.go")).map_err(|e| e.to_string())?;
        fs::write(&tests, include_str!("download_test.go")).map_err(|e| e.to_string())?;
        let result = go_command(temporary.path())?
            .args(["test", "-p=2", "-count=1"])
            .arg(source)
            .arg(tests)
            .status()
            .map_err(|e| format!("HTTPS transport tests require Go: {e}"))?;
        if !result.success() {
            return Err(format!("Go HTTPS transport tests failed ({result})"));
        }
        Ok(())
    }
}
