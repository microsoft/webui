// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Invalid addon modes are compile-time configuration errors, not host panics.

fn assert_invalid_modes(
    features: &[&str],
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = tempfile::tempdir()?;
    // Invalid modes gate out both modules, so rustc needs no dependencies and
    // cannot contend with the outer Cargo process for the target directory.
    let mut command =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
    command.args([
        "--crate-name",
        "webui_node",
        "--crate-type",
        "lib",
        "--edition",
        "2021",
        "--emit",
        "metadata",
        "--out-dir",
    ]);
    command.arg(output_dir.path());
    for feature in features {
        command.arg("--cfg").arg(format!("feature=\"{feature}\""));
    }
    let output = command
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(!output.status.success(), "invalid addon modes compiled");
    assert!(stderr.contains(message), "{stderr}");
    assert!(
        stderr.contains("--no-default-features --features handler"),
        "{stderr}"
    );
    assert!(
        stderr.contains("--no-default-features --features build"),
        "{stderr}"
    );
    assert!(!stderr.contains("unresolved import"), "{stderr}");
    Ok(())
}

#[test]
fn no_mode_is_an_actionable_configuration_error() -> Result<(), Box<dyn std::error::Error>> {
    assert_invalid_modes(&[], "webui-node requires exactly one mode")
}

#[test]
fn combined_modes_are_an_actionable_configuration_error() -> Result<(), Box<dyn std::error::Error>>
{
    assert_invalid_modes(
        &["handler", "build"],
        "webui-node modes are mutually exclusive",
    )
}
