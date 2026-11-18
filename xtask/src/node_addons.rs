// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build and select the independent handler and compiler Node addons.

use crate::cargo_artifacts::{self, Kind};
use crate::util::build_command;
use std::path::PathBuf;
use std::process::{Command, Output};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Addons {
    pub(crate) runtime: PathBuf,
    pub(crate) builder: PathBuf,
}

impl Addons {
    pub(crate) fn apply(&self, command: &mut Command) {
        command
            .env("WEBUI_ADDON_PATH", &self.runtime)
            .env("WEBUI_BUILD_ADDON_PATH", &self.builder);
    }
}

pub(crate) fn build(release: bool) -> Result<Addons, String> {
    let output = run_checked(&mut build_command_for(release), "WebUI Node addons build")?;
    let addons = artifacts(&output.stdout)?;
    for path in [&addons.runtime, &addons.builder] {
        if !path.is_file() {
            return Err(format!(
                "Cargo reported a WebUI Node addon that does not exist: {}",
                path.display()
            ));
        }
    }
    Ok(addons)
}

fn build_command_for(release: bool) -> Command {
    let mut command = build_command(
        "cargo",
        &[
            "build",
            "--locked",
            "-p",
            "microsoft-webui-node",
            "-p",
            "microsoft-webui-node-build",
            "--message-format=json-render-diagnostics",
        ],
    );
    if release {
        command.arg("--release");
    }
    command
}

fn artifacts(output: &[u8]) -> Result<Addons, String> {
    Ok(Addons {
        runtime: cargo_artifacts::find(output, "webui_node", Kind::Cdylib)?,
        builder: cargo_artifacts::find(output, "webui_node_build", Kind::Cdylib)?,
    })
}

pub(crate) fn run_checked(command: &mut Command, label: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn message(target: &str, file: &str) -> String {
        format!(
            "{{\"reason\":\"compiler-artifact\",\"target\":{{\"name\":\"{target}\",\"crate_types\":[\"cdylib\"]}},\"filenames\":[\"{file}\",\"{target}.pdb\",\"{target}.dll.lib\"]}}\n"
        )
    }

    #[test]
    fn builds_both_independent_packages_in_each_profile() {
        for release in [false, true] {
            let command = build_command_for(release);
            let args: Vec<_> = command.get_args().collect();
            for package in ["microsoft-webui-node", "microsoft-webui-node-build"] {
                assert!(args
                    .windows(2)
                    .any(|pair| pair == [OsStr::new("-p"), OsStr::new(package)]));
            }
            assert!(args.contains(&OsStr::new("--locked")));
            assert_eq!(args.contains(&OsStr::new("--release")), release);
        }
    }

    #[test]
    fn selects_both_reported_addons_across_platforms_and_target_directories() {
        for (runtime, builder) in [
            (
                "/custom target/debug/libwebui_node.so",
                "/custom target/debug/libwebui_node_build.so",
            ),
            (
                "/custom target/aarch64-apple-darwin/debug/libwebui_node.dylib",
                "/custom target/aarch64-apple-darwin/debug/libwebui_node_build.dylib",
            ),
            (
                "C:/custom target/debug/webui_node.dll",
                "C:/custom target/debug/webui_node_build.dll",
            ),
        ] {
            let output = message("dependency", "/other/libdependency.so")
                + &message("webui_node_build", builder)
                + &message("webui_node", runtime)
                + "{\"reason\":\"build-finished\",\"success\":true}\n";
            assert_eq!(
                artifacts(output.as_bytes()),
                Ok(Addons {
                    runtime: PathBuf::from(runtime),
                    builder: PathBuf::from(builder),
                })
            );
        }
    }

    #[test]
    fn rejects_missing_malformed_and_ambiguous_artifacts_for_either_addon() {
        let runtime = message("webui_node", "runtime.so");
        let builder = message("webui_node_build", "builder.so");
        for output in [
            String::new(),
            "not json".into(),
            runtime.clone(),
            builder.clone(),
            runtime.clone() + &builder + &builder,
            runtime.clone() + &runtime + &builder,
            runtime.clone()
                + r#"{"reason":"compiler-artifact","target":{"name":"webui_node_build","crate_types":["rlib"]},"filenames":["builder.so"]}"#,
            runtime
                + r#"{"reason":"compiler-artifact","target":{"name":"webui_node_build","crate_types":["cdylib"]},"filenames":["one.so","two.so"]}"#,
        ] {
            assert!(artifacts(output.as_bytes()).is_err(), "{output}");
        }
    }

    #[test]
    fn propagates_failed_child_process_output() {
        let result = run_checked(
            Command::new("node")
                .args(["-e", "console.error('node-suite-failure'); process.exit(1)"]),
            "WebUI Node package tests",
        );
        let Err(error) = result else {
            panic!("a failing Node suite must fail the gate");
        };
        assert!(error.contains("WebUI Node package tests failed"));
        assert!(error.contains("node-suite-failure"));
    }
}
