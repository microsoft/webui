// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::ExitCode;

const METADATA: [(&str, &str); 3] = [
    (
        "Microsoft.Foundation.winmd",
        "bad48725f457085588aa5445983a57ecd1fa761f054006a1e1767e62ba9d1cfe",
    ),
    (
        "Microsoft.Graphics.winmd",
        "ae25bc072fc2067eb8d1a0001fe2efffa7285d50a68a7c2cfb8a91d17c2001d7",
    ),
    (
        "Microsoft.UI.winmd",
        "d1ca121756ddef18bfb143c221fc9136d538a2187be1bffb73df4d9676e25841",
    ),
];

pub(crate) fn run(metadata: Option<&String>) -> ExitCode {
    match generate(metadata) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{} {error}",
                console::style("Windows App SDK binding generation failed:").red()
            );
            ExitCode::FAILURE
        }
    }
}

fn generate(metadata: Option<&String>) -> Result<(), String> {
    let directory = metadata.ok_or(
        "pass the metadata/10.0.17763.0 directory from Microsoft.WindowsAppSDK.InteractiveExperiences 1.8.260708001",
    )?;
    let directory = Path::new(directory);
    let inputs = METADATA.map(|(file, _)| directory.join(file));
    for (input, (_, expected)) in inputs.iter().zip(METADATA) {
        verify_metadata(input, expected)?;
    }
    let output = "crates/webui-desktop/src/windows/app_sdk/bindings.rs";
    let arguments = vec![
        "--in".to_owned(),
        "default".to_owned(),
        inputs[0].to_string_lossy().into_owned(),
        inputs[1].to_string_lossy().into_owned(),
        inputs[2].to_string_lossy().into_owned(),
        "--out".to_owned(),
        output.to_owned(),
        "--filter".to_owned(),
        "Microsoft.UI.WindowId".to_owned(),
        "Microsoft.UI.Dispatching".to_owned(),
        "Microsoft.UI.Windowing.AppWindow".to_owned(),
        "Microsoft.UI.Windowing.AppWindowPresenter".to_owned(),
        "Microsoft.UI.Windowing.AppWindowPresenterKind".to_owned(),
        "Microsoft.UI.Windowing.AppWindowTitleBar".to_owned(),
        "Microsoft.UI.Windowing.IconShowOptions".to_owned(),
        "Microsoft.UI.Windowing.TitleBarTheme".to_owned(),
        "Microsoft.UI.Windowing.TitleBarHeightOption".to_owned(),
        "Microsoft.UI.Input.InputNonClientPointerSource".to_owned(),
        "Microsoft.UI.Input.NonClientRegionKind".to_owned(),
        "--reference".to_owned(),
        "windows".to_owned(),
    ];
    let warnings = windows_bindgen::bindgen(arguments);
    if !warnings.is_empty() {
        eprintln!("Narrow projection omits unused APIs:\n{warnings}");
    }
    let generated = std::fs::read_to_string(output).map_err(|error| error.to_string())?;
    std::fs::write(
        output,
        format!(
            "// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n\n// Generated from Windows App SDK 1.8.11 metadata; do not edit.\n{generated}"
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn verify_metadata(path: &Path, expected: &str) -> Result<(), String> {
    use std::fmt::Write;
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("cannot read metadata {}: {error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let mut actual = String::with_capacity(64);
    for byte in hash.finalize() {
        let _ = write!(actual, "{byte:02x}");
    }
    if actual != expected {
        return Err(format!(
            "metadata hash mismatch for {}; help: restore Microsoft.WindowsAppSDK.InteractiveExperiences 1.8.260708001 instead of generating against another SDK",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn generator_requires_explicit_pinned_metadata_directory() {
        assert!(generate(None).unwrap_err().contains("1.8.260708001"));
        let root = tempfile::tempdir().unwrap();
        assert!(generate(Some(&root.path().to_string_lossy().into_owned()))
            .unwrap_err()
            .contains("cannot read metadata"));
    }

    #[test]
    fn metadata_hash_must_match_before_generating_code() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("metadata.winmd");
        std::fs::write(&path, b"abc").unwrap();
        assert!(verify_metadata(&path, "wrong")
            .unwrap_err()
            .contains("hash mismatch"));
        verify_metadata(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
    }
}
