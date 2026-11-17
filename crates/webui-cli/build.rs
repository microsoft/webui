// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[path = "resources/build_support.rs"]
mod support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/commands/init/templates");
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?,
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing build output")?)
        .join("native-assets");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    let sources = support::stage_for_build(&manifest, &out, env!("CARGO_PKG_VERSION"))?;
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    println!("cargo:rustc-env=WEBUI_NATIVE_ASSETS={}", out.display());
    println!(
        "cargo:rustc-env=WEBUI_NATIVE_TARGET={}",
        std::env::var("TARGET")?
    );
    Ok(())
}
