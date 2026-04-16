// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

extern crate cbindgen;

use std::env;
use std::io::{Error, ErrorKind};
use std::path::PathBuf; // Removed 'self' import

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get the crate directory with proper error handling
    let crate_dir = env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| Error::new(ErrorKind::NotFound, "CARGO_MANIFEST_DIR not found"))?;

    // Set the output directory
    let out_dir = PathBuf::from(crate_dir.clone()).join("include"); // Added clone here

    // Create the output directory with proper error handling
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| Error::new(e.kind(), format!("Failed to create directory: {}", e)))?;

    // C header
    let config = cbindgen::Config::default();
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .map_err(|e| Error::other(format!("Unable to generate C bindings: {e}")))?;
    bindings.write_to_file(out_dir.join("webui_ffi.h"));

    Ok(())
}
