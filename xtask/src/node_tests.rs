// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::node_addons::{self, run_checked, Addons, Variant};
use crate::util::{build_command, run_command_quiet};
use std::process::Command;

pub(crate) fn run() -> Result<(), String> {
    let dependencies = run_checked(
        &mut runtime_dependency_command(),
        "WebUI runtime dependency audit",
    )?;
    check_runtime_dependencies(&dependencies.stdout)?;
    // Workspace tests exercise the default handler; keep the compiler's Rust
    // regressions in the same Node test sequence without feature unification.
    let target_dir = node_addons::target_directory(&crate::util::workspace_root()?)?;
    run_checked(
        &mut node_addons::variant_command("test", Variant::Build, &target_dir),
        "WebUI compiler addon Rust tests",
    )?;
    let addons = node_addons::build(false)?;

    run_command_quiet("pnpm", &["--filter", "@microsoft/webui", "build"], None)
        .map_err(|error| format!("WebUI test package build failed:\n{error}"))?;
    run_checked(&mut test_command(&addons), "WebUI Node package tests")?;
    Ok(())
}

fn runtime_dependency_command() -> Command {
    build_command(
        "cargo",
        &[
            "tree",
            "--locked",
            "-p",
            "microsoft-webui-node",
            "--no-default-features",
            "--features",
            "handler",
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
    )
}

fn check_runtime_dependencies(output: &[u8]) -> Result<(), String> {
    let tree = std::str::from_utf8(output)
        .map_err(|error| format!("Invalid WebUI runtime dependency tree: {error}"))?;
    let mut has_runtime = false;
    for package in tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        has_runtime |= package == "microsoft-webui-node";
        if matches!(
            package,
            "microsoft-webui"
                | "microsoft-webui-parser"
                | "microsoft-webui-discovery"
                | "microsoft-webui-projection"
                | "rolldown"
                | "oxc"
        ) || package.starts_with("rolldown_")
            || package.starts_with("oxc_")
        {
            return Err(format!(
                "WebUI handler addon has a production dependency on {package}; \
                 gate compiler dependencies behind the build feature (parser dev-tests are allowed)"
            ));
        }
    }
    if !has_runtime {
        return Err("Cargo did not report the WebUI runtime dependency graph".to_string());
    }
    Ok(())
}

fn test_command(addons: &Addons) -> Command {
    let mut command = build_command("pnpm", &["--filter", "@microsoft/webui", "test"]);
    addons.apply(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::{check_runtime_dependencies, runtime_dependency_command, test_command, Addons};
    use std::ffi::OsStr;

    #[test]
    fn runs_the_package_script_with_both_explicit_fresh_addons() {
        let addons = Addons {
            runtime: "custom target/libwebui_node.so".into(),
            builder: "custom target/node-build/libwebui_node.so".into(),
        };
        let command = test_command(&addons);
        let args: Vec<_> = command.get_args().collect();
        assert!(args.ends_with(&[
            OsStr::new("--filter"),
            OsStr::new("@microsoft/webui"),
            OsStr::new("test"),
        ]));
        for (name, path) in [
            ("WEBUI_ADDON_PATH", &addons.runtime),
            ("WEBUI_BUILD_ADDON_PATH", &addons.builder),
        ] {
            assert!(command
                .get_envs()
                .any(|(key, value)| key == name && value == Some(path.as_os_str())));
        }
    }

    #[test]
    fn audits_normal_and_build_dependencies_without_parser_dev_tests() {
        let command = runtime_dependency_command();
        let args: Vec<_> = command.get_args().collect();
        assert!(args.contains(&OsStr::new("--no-default-features")));
        assert!(args
            .windows(2)
            .any(|pair| pair == [OsStr::new("--features"), OsStr::new("handler")]));
        assert!(args
            .windows(2)
            .any(|pair| { pair == [OsStr::new("--edges"), OsStr::new("normal,build")] }));
        assert!(args
            .windows(2)
            .any(|pair| pair == [OsStr::new("--target"), OsStr::new("all")]));
        assert!(check_runtime_dependencies(
            b"microsoft-webui-node v3.1.2\nmicrosoft-webui-handler v3.1.2\nnapi v3.12.4\n"
        )
        .is_ok());
        assert!(check_runtime_dependencies(b"").is_err());
        for forbidden in [
            "microsoft-webui",
            "microsoft-webui-parser",
            "microsoft-webui-discovery",
            "microsoft-webui-projection",
            "rolldown",
            "rolldown_common",
            "oxc",
            "oxc_parser",
        ] {
            let tree = format!("microsoft-webui-node v3.1.2\n{forbidden} v1.0.0\n");
            let Err(error) = check_runtime_dependencies(tree.as_bytes()) else {
                panic!("compiler code must not enter the runtime production graph");
            };
            assert!(error.contains(forbidden), "{error}");
        }
    }
}
