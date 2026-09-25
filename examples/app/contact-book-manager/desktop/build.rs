// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("icon.ico")
            .compile()?;
    }
    Ok(())
}
