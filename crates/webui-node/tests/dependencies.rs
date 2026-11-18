// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Guard each production addon graph independently of compiler-backed test fixtures.

use std::collections::HashSet;

fn dependency_tree(mode: &str) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--package",
            "microsoft-webui-node",
            "--no-default-features",
            "--features",
            mode,
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p} features=[{f}]",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn packages(tree: &str) -> HashSet<&str> {
    tree.lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

#[test]
fn runtime_dependencies_exclude_compiler_and_build_addon() -> Result<(), Box<dyn std::error::Error>>
{
    // Select handler explicitly even when this test runs in the build variant.
    let tree = dependency_tree("handler")?;
    assert!(
        tree.lines()
            .next()
            .is_some_and(|root| root.ends_with("features=[handler]")),
        "runtime graph must enable only the handler mode:\n{tree}"
    );
    let packages = packages(&tree);
    assert!(packages.contains("microsoft-webui-handler"));
    assert!(packages.contains("microsoft-webui-protocol"));
    for package in packages {
        assert!(
            !matches!(
                package,
                "microsoft-webui"
                    | "microsoft-webui-client-build"
                    | "microsoft-webui-node-build"
                    | "microsoft-webui-projection"
                    | "microsoft-webui-parser"
                    | "microsoft-webui-discovery"
                    | "tokio"
            ) && !package.starts_with("rolldown")
                && !package.starts_with("oxc"),
            "compiler dependency {package} reached the runtime addon:\n{tree}"
        );
    }
    Ok(())
}

#[cfg(feature = "build")]
#[test]
fn build_dependencies_include_compilers_without_runtime_addon(
) -> Result<(), Box<dyn std::error::Error>> {
    let build_tree = dependency_tree("build")?;
    let handler_tree = dependency_tree("handler")?;
    let build_packages = packages(&build_tree);
    let handler_packages = packages(&handler_tree);
    assert!(
        build_tree
            .lines()
            .next()
            .is_some_and(|root| root.ends_with("features=[build]")),
        "compiler graph must enable only the build mode:\n{build_tree}"
    );
    for compiler in [
        "microsoft-webui",
        "microsoft-webui-projection",
        "microsoft-webui-parser",
        "microsoft-webui-discovery",
        "tokio",
    ] {
        assert!(
            build_packages.contains(compiler),
            "compiler dependency {compiler} missing from build mode:\n{build_tree}"
        );
        assert!(
            !handler_packages.contains(compiler),
            "compiler dependency {compiler} reached handler mode:\n{handler_tree}"
        );
    }
    // Both variants compile the same package, never a second addon dependency.
    assert!(build_packages.contains("microsoft-webui-node"));
    assert!(!build_packages.contains("microsoft-webui-node-build"));
    assert!(!build_packages.contains("microsoft-webui-client-build"));
    assert!(
        build_tree
            .lines()
            .any(|line| { line.starts_with("microsoft-webui ") && line.contains("browser-build") }),
        "build mode must select the core browser compiler:\n{build_tree}"
    );
    assert!(build_packages
        .iter()
        .any(|name| name.starts_with("rolldown")));
    assert!(build_packages.iter().any(|name| name.starts_with("oxc")));
    assert_ne!(build_packages, handler_packages);
    Ok(())
}
