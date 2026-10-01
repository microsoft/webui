// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "desktop")]
    tauri_build::try_build(Default::default())?;
    Ok(())
}
