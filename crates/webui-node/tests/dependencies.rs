// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Guard the production addon graph independently of compiler-backed test fixtures.

#[test]
fn runtime_dependencies_exclude_compiler_and_build_addon() -> Result<(), Box<dyn std::error::Error>>
{
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--package",
            "microsoft-webui-node",
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
    for package in tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        assert!(
            !matches!(
                package,
                "microsoft-webui"
                    | "microsoft-webui-client-build"
                    | "microsoft-webui-node-build"
                    | "microsoft-webui-projection"
                    | "microsoft-webui-parser"
                    | "microsoft-webui-discovery"
            ) && !package.starts_with("rolldown")
                && !package.starts_with("oxc"),
            "compiler dependency {package} reached the runtime addon:\n{tree}"
        );
    }
    Ok(())
}
