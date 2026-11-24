// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::util::build_command;

#[cfg(any(windows, test))]
#[path = "../../crates/webui-desktop/runtime/architecture.rs"]
mod architecture;
#[cfg(any(windows, test))]
#[path = "../../crates/webui-desktop/runtime/deployment.rs"]
mod deployment;

// Keep these package-local: workspace feature unification can hide missing gates.
const FEATURES: &[&str] = &[
    "",
    "packaging",
    "native",
    "source",
    "native,source",
    "cli",
    "application-ipc",
    "native,application-ipc",
    "source,application-ipc",
    "native,source,application-ipc",
];

#[cfg(any(windows, test))]
const WINDOWS_NATIVE_SUITES: &[(&str, &[&str])] = &[
    (
        "Windows native desktop SDK",
        &[
            "test",
            "--locked",
            "-p",
            "microsoft-webui-desktop",
            "--no-default-features",
            "--features",
            "native,native-dwm-frame",
            "--lib",
            "--tests",
            "--",
            "--include-ignored",
            "--test-threads=1",
        ],
    ),
    (
        "Windows native services",
        &[
            "test",
            "--locked",
            "-p",
            "microsoft-webui-desktop",
            "--no-default-features",
            "--features",
            "native-picker,native-dialogs,native-clipboard,application-ipc,startup-failure",
            "--lib",
            "--",
            "--test-threads=1",
        ],
    ),
];

pub(crate) fn run() -> Result<(), String> {
    for features in FEATURES {
        let mut command = build_command(
            "cargo",
            &[
                "test",
                "-p",
                "microsoft-webui-desktop",
                "--no-default-features",
                "--lib",
                "--tests",
            ],
        );
        if !features.is_empty() {
            command.args(["--features", features]);
        }
        command.args(["--", "--format=pretty"]);
        run_suite(&mut command, &format!("Desktop features [{features}]"))?;
    }
    for features in [
        "local-server",
        "local-server,application-ipc",
        "native-url-activation",
        "native-services",
    ] {
        run_suite(
            &mut build_command(
                "cargo",
                &[
                    "test",
                    "-p",
                    "microsoft-webui-desktop",
                    "--no-default-features",
                    "--features",
                    features,
                    "--lib",
                    "--test",
                    "local_server_api",
                    "--",
                    "--format=pretty",
                ],
            ),
            &format!("Desktop local-server features [{features}]"),
        )?;
    }
    run_suite(
        &mut build_command(
            "cargo",
            &[
                "test",
                "-p",
                "microsoft-webui-desktop",
                "--no-default-features",
                "--features",
                "local-server",
                "--doc",
                "local_server::LocalServerFrame",
            ],
        ),
        "Desktop URL activation API opt-out",
    )?;
    #[cfg(target_os = "macos")]
    run_suite(
        &mut build_command(
            "cargo",
            &[
                "test",
                "--release",
                "-p",
                "microsoft-webui-desktop",
                "--no-default-features",
                "--features",
                "native-capture",
                "--test",
                "capture_allocations",
            ],
        ),
        "Desktop capture allocations",
    )?;
    #[cfg(windows)]
    run_windows_native_with(|args| {
        prepare_windows_native(args)?;
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push(String::from("cargo"));
        command.extend(args.iter().map(|arg| String::from(*arg)));
        crate::windows_desktop::run(&command)
    })?;
    Ok(())
}

#[cfg(windows)]
fn prepare_windows_native(args: &[&str]) -> Result<(), String> {
    let mut command = build_command("cargo", &[]);
    command.args(args.iter().take_while(|arg| **arg != "--"));
    command.args(["--no-run", "--message-format=json-render-diagnostics"]);
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Cannot build Windows native tests:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let runtime = crate::util::workspace_root()?.join("crates/webui-desktop/runtime");
    stage_windows_test_assets(&output.stdout, &runtime, std::env::consts::ARCH)
}

#[cfg(any(windows, test))]
fn stage_windows_test_assets(
    output: &[u8],
    runtime: &std::path::Path,
    arch: &str,
) -> Result<(), String> {
    use std::{collections::BTreeSet, fs, path::PathBuf};

    let mut directories = BTreeSet::new();
    for line in output.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let artifact: serde_json::Value = serde_json::from_slice(line)
            .map_err(|error| format!("Invalid Cargo test artifact: {error}"))?;
        if artifact["reason"] != "compiler-artifact" || artifact["profile"]["test"] != true {
            continue;
        }
        let Some(executable) = artifact["executable"].as_str() else {
            continue;
        };
        let executable = PathBuf::from(executable);
        if !executable.is_absolute() || !executable.is_file() {
            return Err(format!(
                "Missing absolute Cargo test executable: {}",
                executable.display()
            ));
        }
        let directory = executable
            .parent()
            .ok_or("Cargo test executable has no directory")?;
        directories.insert(directory.to_path_buf());
    }
    if directories.is_empty() {
        return Err("Cargo did not report Windows native test executables".into());
    }
    let rid = architecture::runtime_architecture(arch).map_err(|error| error.to_string())?;
    let sources = std::iter::once((
        deployment::BOOTSTRAP_DLL,
        runtime.join(rid).join(deployment::BOOTSTRAP_DLL),
    ))
    .chain(deployment::NOTICES.map(|name| (name, runtime.join(name))));
    let assets: Vec<_> = sources
        .map(|(name, path)| {
            let bytes = fs::read(&path)
                .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
            if bytes.is_empty() {
                return Err(format!("Empty Windows runtime asset: {}", path.display()));
            }
            Ok((name, bytes))
        })
        .collect::<Result<_, String>>()?;
    for directory in directories {
        for (name, bytes) in &assets {
            let path = directory.join(name);
            fs::write(&path, bytes)
                .map_err(|error| format!("Cannot stage {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn run_windows_native_with(
    mut execute: impl FnMut(&[&'static str]) -> Result<u32, String>,
) -> Result<(), String> {
    for (label, args) in WINDOWS_NATIVE_SUITES {
        eprintln!("  {} {label}", console::style("▸").cyan().bold());
        let code = execute(args).map_err(|error| format!("{label}: {error}"))?;
        if code != 0 {
            return Err(format!("{label} failed with exit code {code}"));
        }
    }
    Ok(())
}

fn run_suite(command: &mut std::process::Command, label: &str) -> Result<(), String> {
    let output = command.output().map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!("{label} failed:\n{stdout}{stderr}"));
    }
    nonempty_suites(&stdout).map_err(|error| format!("{label}: {error}\n{stdout}{stderr}"))
}

fn nonempty_suites(output: &str) -> Result<(), &'static str> {
    let mut suites = 0;
    for line in output.lines() {
        let Some(summary) = line.strip_prefix("test result: ok. ") else {
            continue;
        };
        let count = summary
            .split_whitespace()
            .next()
            .and_then(|count| count.parse::<usize>().ok());
        if !count.is_some_and(|count| count > 0) {
            return Err("a required test suite ran zero passing tests");
        }
        suites += 1;
    }
    if suites == 0 {
        return Err("no Rust test suite results were reported");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{deployment, nonempty_suites, run_windows_native_with, stage_windows_test_assets};

    #[test]
    fn windows_runtime_follows_exact_cargo_test_artifacts() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let runtime = root.path().join("runtime");
        std::fs::create_dir_all(runtime.join("win-arm64"))?;
        std::fs::write(
            runtime.join("win-arm64").join(deployment::BOOTSTRAP_DLL),
            b"arm64",
        )?;
        for name in deployment::NOTICES {
            std::fs::write(runtime.join(name), name)?;
        }
        let mut output = Vec::new();
        let mut executables = Vec::new();
        for relative in [
            "debug/deps/sdk.exe",
            "custom/build/desktop/unit/out/sdk.exe",
        ] {
            let executable = root.path().join(relative);
            std::fs::create_dir_all(executable.parent().ok_or("missing test directory")?)?;
            std::fs::write(&executable, b"test")?;
            serde_json::to_writer(
                &mut output,
                &serde_json::json!({
                    "reason": "compiler-artifact", "profile": {"test": true},
                    "executable": executable, "fresh": true
                }),
            )?;
            output.push(b'\n');
            executables.push(executable);
        }
        stage_windows_test_assets(&output, &runtime, "aarch64")?;
        for executable in executables {
            let directory = executable.parent().ok_or("missing test directory")?;
            assert_eq!(
                std::fs::read(directory.join(deployment::BOOTSTRAP_DLL))?,
                b"arm64"
            );
            for name in deployment::NOTICES {
                assert_eq!(std::fs::read_to_string(directory.join(name))?, name);
            }
        }
        Ok(())
    }

    #[test]
    fn windows_runtime_staging_requires_real_test_artifacts() {
        let root = tempfile::tempdir().unwrap();
        for output in [
            &b"not json"[..],
            br#"{"reason":"compiler-artifact","profile":{"test":false},"executable":"app.exe"}"#,
            br#"{"reason":"compiler-artifact","profile":{"test":true},"executable":"missing.exe"}"#,
        ] {
            assert!(stage_windows_test_assets(output, root.path(), "x86_64").is_err());
        }
    }

    #[test]
    fn windows_gate_runs_native_sdk_and_services_with_original_assertions() {
        let mut suites = Vec::new();
        run_windows_native_with(|args| {
            suites.push(args.to_vec());
            Ok(0)
        })
        .unwrap();
        assert_eq!(suites.len(), 2);
        assert!(suites[0].contains(&"native,native-dwm-frame"));
        assert!(suites[0].contains(&"--include-ignored"));
        assert!(suites[0].contains(&"--tests"));
        assert!(suites[1].contains(
            &"native-picker,native-dialogs,native-clipboard,application-ipc,startup-failure"
        ));
        for suite in suites {
            assert!(suite.contains(&"--test-threads=1"));
            assert!(suite.contains(&"--locked"));
            assert!(suite.contains(&"--no-default-features"));
        }
    }

    #[test]
    fn windows_gate_stops_after_failure_or_isolation_error() {
        for result in [Ok(37), Err(String::from("desktop creation failed"))] {
            let mut calls = 0;
            let error = run_windows_native_with(|_| {
                calls += 1;
                result.clone()
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            assert!(error.contains("Windows native desktop SDK"));
            assert!(error.contains("37") || error.contains("desktop creation failed"));
        }
    }

    #[test]
    fn accepts_nonempty_suites() {
        assert!(nonempty_suites(
            "test result: ok. 12 passed; 0 failed;\ntest result: ok. 1 passed; 0 failed;\n"
        )
        .is_ok());
    }

    #[test]
    fn rejects_missing_empty_or_ignored_only_suites() {
        for output in [
            "",
            "test result: ok. 0 passed; 0 failed; 0 ignored;",
            "test result: ok. 0 passed; 0 failed; 4 ignored;",
            "test result: ok. 8 passed; 0 failed;\ntest result: ok. 0 passed; 0 failed;",
            "test result: ok. unknown passed;",
        ] {
            assert!(nonempty_suites(output).is_err(), "{output}");
        }
    }
}
