// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::util::{build_command, run_command_quiet};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn run() -> Result<(), String> {
    let output = run_checked(
        &mut build_command(
            "cargo",
            &[
                "build",
                "--locked",
                "-p",
                "microsoft-webui-node",
                "--message-format=json-render-diagnostics",
            ],
        ),
        "WebUI test addon build",
    )?;
    let addon = addon_artifact(&output.stdout)?;
    if !addon.is_file() {
        return Err(format!(
            "Cargo reported a WebUI test addon that does not exist: {}",
            addon.display()
        ));
    }

    run_command_quiet("pnpm", &["--filter", "@microsoft/webui", "build"], None)
        .map_err(|error| format!("WebUI test package build failed:\n{error}"))?;
    run_checked(&mut test_command(&addon), "WebUI Node package tests")?;
    Ok(())
}

fn test_command(addon: &Path) -> Command {
    let mut command = build_command("pnpm", &["--filter", "@microsoft/webui", "test"]);
    command.env("WEBUI_ADDON_PATH", addon);
    command
}

fn run_checked(command: &mut Command, label: &str) -> Result<Output, String> {
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

fn addon_artifact(output: &[u8]) -> Result<PathBuf, String> {
    let mut artifact = None;
    for line in output.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message: Value = serde_json::from_slice(line)
            .map_err(|error| format!("Invalid Cargo addon build message: {error}"))?;
        if message["reason"] != "compiler-artifact"
            || message["target"]["name"] != "webui_node"
            || !message["target"]["crate_types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|kind| kind == "cdylib"))
        {
            continue;
        }
        let Some(files) = message["filenames"].as_array() else {
            continue;
        };
        for file in files.iter().filter_map(Value::as_str) {
            let path = Path::new(file);
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("dll" | "dylib" | "so")
            ) {
                continue;
            }
            if artifact.is_some() {
                return Err("Cargo reported multiple WebUI test addons".to_owned());
            }
            artifact = Some(path.to_path_buf());
        }
    }
    artifact.ok_or_else(|| {
        "Cargo did not report the webui_node cdylib; rebuild microsoft-webui-node for the host"
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::{addon_artifact, run_checked, test_command};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    fn selects_the_reported_addon_across_platforms_and_target_directories() {
        for file in [
            "/custom target/debug/libwebui_node.so",
            "/custom target/aarch64-apple-darwin/debug/libwebui_node.dylib",
            "C:/custom target/debug/webui_node.dll",
        ] {
            let output = format!(
                "{{\"reason\":\"compiler-artifact\",\"target\":{{\"name\":\"dependency\",\"crate_types\":[\"cdylib\"]}},\"filenames\":[\"/other/libdependency.so\"]}}\n\
                 {{\"reason\":\"compiler-artifact\",\"target\":{{\"name\":\"webui_node\",\"crate_types\":[\"cdylib\"]}},\"filenames\":[\"{file}\",\"webui_node.pdb\",\"webui_node.dll.lib\"]}}\n\
                 {{\"reason\":\"build-finished\",\"success\":true}}\n"
            );
            assert_eq!(addon_artifact(output.as_bytes()), Ok(PathBuf::from(file)));
        }
    }

    #[test]
    fn rejects_missing_malformed_and_ambiguous_artifacts() {
        for output in [
            "",
            "not json",
            r#"{"reason":"build-finished","success":true}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"webui_node","crate_types":["rlib"]},"filenames":["libwebui_node.so"]}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"webui_node","crate_types":["cdylib"]},"filenames":["one.so","two.so"]}"#,
        ] {
            assert!(addon_artifact(output.as_bytes()).is_err(), "{output}");
        }
    }

    #[test]
    fn runs_the_package_script_with_an_explicit_fresh_addon() {
        let addon = Path::new("custom target/libwebui_node.so");
        let command = test_command(addon);
        let args: Vec<_> = command.get_args().collect();
        assert!(args.ends_with(&[
            OsStr::new("--filter"),
            OsStr::new("@microsoft/webui"),
            OsStr::new("test"),
        ]));
        assert!(command
            .get_envs()
            .any(|(key, value)| { key == "WEBUI_ADDON_PATH" && value == Some(addon.as_os_str()) }));
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
