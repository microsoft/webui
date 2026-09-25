// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::util::build_command;

// Keep these package-local: workspace feature unification can hide missing gates.
const FEATURES: &[&str] = &[
    "",
    "native",
    "source",
    "native,source",
    "cli",
    "application-ipc",
    "native,application-ipc",
    "source,application-ipc",
    "native,source,application-ipc",
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
        let output = command.output().map_err(|error| error.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return Err(format!(
                "Desktop features [{features}] failed:\n{stdout}{stderr}"
            ));
        }
        nonempty_suites(&stdout)
            .map_err(|error| format!("Desktop features [{features}]: {error}\n{stdout}{stderr}"))?;
    }
    Ok(())
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
    use super::nonempty_suites;

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
