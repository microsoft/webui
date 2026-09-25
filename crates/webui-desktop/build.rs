// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[path = "runtime/build_support.rs"]
mod build_support;
#[path = "runtime/deployment.rs"]
mod deployment;

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_NATIVE");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var_os("CARGO_FEATURE_NATIVE").is_none()
    {
        return;
    }
    if let Err(error) = stage_runtime() {
        eprintln!("Windows App SDK bootstrap staging failed: {error}");
        std::process::exit(1);
    }
}

fn stage_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let rid = build_support::runtime_architecture(&arch)?;
    let runtime =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing CARGO_MANIFEST_DIR")?)
            .join("runtime");
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing Cargo OUT_DIR")?);
    let profile = build_support::profile_directory(&out)?;
    let bootstrap = runtime.join(rid).join(deployment::BOOTSTRAP_DLL);
    println!("cargo:rerun-if-changed={}", bootstrap.display());
    for name in deployment::NOTICES {
        println!("cargo:rerun-if-changed={}", runtime.join(name).display());
    }
    build_support::stage_runtime(&runtime, &bootstrap, &profile)?;
    Ok(())
}
