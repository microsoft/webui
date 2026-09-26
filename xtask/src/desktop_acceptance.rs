// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::desktop_tests;
use crate::util::run_command;

const COMMANDS: &[(&str, &[&str])] = &[
    (
        "cargo",
        &[
            "test",
            "--locked",
            "-p",
            "microsoft-webui-desktop-build",
            "--test",
            "generate",
        ],
    ),
    ("pnpm", &["--filter", "@microsoft/webui-desktop", "test"]),
    (
        "node",
        &[
            "packages/webui-desktop/scripts/sync-bootstrap.mjs",
            "--check",
            "crates/webui-desktop/src/generated/ipc",
        ],
    ),
    ("node", &["crates/webui-desktop-build/tests/run-rust.mjs"]),
    (
        "node",
        &["crates/webui-desktop-build/tests/run-typescript.mjs"],
    ),
    (
        "node",
        &["packages/webui-desktop/scripts/test-lazy-browser.mjs"],
    ),
];

pub(crate) fn run() -> Result<(), String> {
    run_with(
        std::env::var_os("WEBUI_UPDATE_IPC_FIXTURE").is_some(),
        |program, args| run_command(program, args, None),
        desktop_tests::run,
    )
}

fn run_with(
    update_fixture: bool,
    mut execute: impl FnMut(&str, &[&str]) -> Result<(), String>,
    isolated_tests: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if update_fixture {
        return Err("acceptance must check committed bindings, not regenerate them".into());
    }
    for (program, args) in COMMANDS {
        println!("+ {program} {}", args.join(" "));
        execute(program, args).map_err(|error| format!("{program} {}: {error}", args.join(" ")))?;
    }
    isolated_tests()?;

    let args = [
        "build",
        "--locked",
        "-p",
        "microsoft-webui-desktop",
        "--no-default-features",
        "--features",
        "cli",
        "--bin",
        "webui-desktop",
    ];
    println!("+ cargo {}", args.join(" "));
    execute("cargo", &args).map_err(|error| format!("cargo {}: {error}", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::{run_with, COMMANDS};
    use std::cell::Cell;

    #[test]
    fn checks_generated_bindings_before_consumers_without_regenerating() {
        assert_eq!(
            COMMANDS[0],
            (
                "cargo",
                &[
                    "test",
                    "--locked",
                    "-p",
                    "microsoft-webui-desktop-build",
                    "--test",
                    "generate"
                ][..]
            )
        );
        let sync = COMMANDS.iter().position(|(_, args)| {
            args.contains(&"packages/webui-desktop/scripts/sync-bootstrap.mjs")
        });
        let rust = COMMANDS
            .iter()
            .position(|(_, args)| args.contains(&"crates/webui-desktop-build/tests/run-rust.mjs"));
        assert!(sync.is_some_and(|index| COMMANDS[index].1.contains(&"--check")));
        assert!(sync
            .zip(rust)
            .is_some_and(|(check, consumer)| check < consumer));
        assert!(COMMANDS.iter().all(|(_, args)| !args.contains(&"--write")));
    }

    #[test]
    fn rejects_regeneration_even_before_running_commands() {
        let called = Cell::new(false);
        let result = run_with(
            true,
            |_, _| {
                called.set(true);
                Ok(())
            },
            || {
                called.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!called.get());
    }

    #[test]
    fn failed_gate_prevents_later_consumers() {
        let commands = Cell::new(0);
        let isolated = Cell::new(false);
        let result = run_with(
            false,
            |program, args| {
                assert_eq!(program, "cargo");
                assert_eq!(args, COMMANDS[0].1);
                commands.set(commands.get() + 1);
                Err("generator failed".into())
            },
            || {
                isolated.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(commands.get(), 1);
        assert!(!isolated.get());
    }
}
