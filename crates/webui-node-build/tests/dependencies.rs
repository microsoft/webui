// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The build addon owns compilation but must not load the runtime addon.

#[test]
fn build_dependencies_include_compilers_without_runtime_addon(
) -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--package",
            "microsoft-webui-node-build",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout)?;
    let packages: std::collections::HashSet<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert!(packages.contains("microsoft-webui"));
    assert!(packages.contains("microsoft-webui-client-build"));
    assert!(
        !packages.contains("microsoft-webui-node"),
        "runtime addon reached the compiler addon:\n{tree}"
    );
    Ok(())
}
