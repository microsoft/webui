// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::util::build_command;

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
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push(String::from("cargo"));
        command.extend(args.iter().map(|arg| String::from(*arg)));
        crate::windows_desktop::run(&command)
    })?;
    Ok(())
}

#[cfg(any(windows, test))]
fn run_windows_native_with(
    mut execute: impl FnMut(&[&'static str]) -> Result<u32, String>,
) -> Result<(), String> {
    for (label, args) in WINDOWS_NATIVE_SUITES {
        eprintln!(
            "  {} {label} (isolated desktop)",
            console::style("▸").cyan().bold()
        );
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
    use super::{nonempty_suites, run_windows_native_with};

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
