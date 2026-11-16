// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The shipped macOS C ABI must link outside the Cargo build directory.

#[cfg(target_os = "macos")]
#[test]
fn cdylib_install_name_is_relocatable() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let artifacts = executable
        .parent()
        .ok_or("test executable has no directory")?;
    let library = artifacts.join("libwebui_ffi.dylib");
    let output = std::process::Command::new("otool")
        .arg("-D")
        .arg(&library)
        .output()?;
    assert!(
        output.status.success(),
        "could not inspect {}: {}",
        library.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let install_names = String::from_utf8(output.stdout)?;
    assert_eq!(
        install_names.lines().nth(1),
        Some("@rpath/libwebui_ffi.dylib"),
        "the C ABI must not retain an absolute Cargo build path"
    );
    Ok(())
}
