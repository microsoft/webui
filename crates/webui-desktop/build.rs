// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[path = "runtime/build_support.rs"]
mod build_support;
#[path = "runtime/deployment.rs"]
mod deployment;
#[path = "runtime/ipc_asset_build.rs"]
mod ipc_asset_build;

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_NATIVE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_APPLICATION_IPC");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");
    println!("cargo:rerun-if-env-changed=WEBUI_DESKTOP_ASSET_SOURCE_DIR");
    if env::var_os("CARGO_FEATURE_APPLICATION_IPC").is_some() {
        if let Err(error) = stage_ipc_assets() {
            eprintln!("Desktop browser asset staging failed: {error}");
            std::process::exit(1);
        }
    }
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

fn stage_ipc_assets() -> Result<(), Box<dyn std::error::Error>> {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing CARGO_MANIFEST_DIR")?);
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing Cargo OUT_DIR")?);
    let source_override = env::var_os("WEBUI_DESKTOP_ASSET_SOURCE_DIR");
    let (source, staged) =
        ipc_asset_build::stage_assets(&manifest, &out, source_override.as_deref())?;
    for name in ipc_asset_build::ASSET_NAMES {
        println!("cargo:rerun-if-changed={}", source.join(name).display());
    }
    println!(
        "cargo:rustc-env=WEBUI_DESKTOP_IPC_ASSET_DIR={}",
        staged.display()
    );
    Ok(())
}
