// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Isolated, host-native standard-library Go producer helpers.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Never inherit a matrix's foreign Go target or mutate a global build cache.
pub(crate) fn command(root: &Path) -> Result<Command, String> {
    let cache = root.join("go-build");
    let temporary = root.join("go-tmp");
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    fs::create_dir_all(&temporary).map_err(|e| e.to_string())?;
    let mut command = Command::new("go");
    command
        .current_dir(root)
        .env("GO111MODULE", "off")
        .env("GOWORK", "off")
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GOFLAGS", "")
        .env("CGO_ENABLED", "0")
        .env("GOMAXPROCS", "2")
        .env("GOTELEMETRY", "off")
        .env("GOCACHE", cache)
        .env("GOTMPDIR", temporary);
    for variable in [
        "GOOS",
        "GOARCH",
        "GOARM",
        "GOARM64",
        "GOAMD64",
        "GO386",
        "GOMIPS",
        "GOMIPS64",
        "GOWASM",
        "GOEXPERIMENT",
    ] {
        command.env_remove(variable);
    }
    Ok(command)
}
