// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The shipped macOS C ABI must link outside the Cargo build directory.

#[cfg(target_os = "macos")]
#[test]
fn cdylib_install_name_is_relocatable() -> Result<(), Box<dyn std::error::Error>> {
    let library = build_cdylib()?;
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

#[cfg(target_os = "macos")]
fn build_cdylib() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "build",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--lib",
            "--message-format=json-render-diagnostics",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "could not build the C ABI: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in output.stdout.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let message: serde_json::Value = serde_json::from_slice(line)?;
        if message["reason"] != "compiler-artifact" || message["target"]["name"] != "webui_ffi" {
            continue;
        }
        let filenames = message["filenames"]
            .as_array()
            .ok_or("C ABI artifact has no filenames")?;
        for filename in filenames {
            let path = std::path::Path::new(
                filename
                    .as_str()
                    .ok_or("C ABI artifact filename is not a string")?,
            );
            if path
                .file_name()
                .is_some_and(|name| name == "libwebui_ffi.dylib")
            {
                return Ok(path.to_owned());
            }
        }
    }
    Err("Cargo did not report the C ABI dynamic library".into())
}
