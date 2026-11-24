// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub(crate) fn runtime_architecture(arch: &str) -> std::io::Result<&'static str> {
    match arch {
        "x86_64" => Ok("win-x64"),
        "aarch64" => Ok("win-arm64"),
        "x86" => Ok("win-x86"),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("unsupported Windows architecture {arch}; use x86_64, aarch64, or i686"),
        )),
    }
}
