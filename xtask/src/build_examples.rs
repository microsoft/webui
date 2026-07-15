// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Examples build tasks.

use crate::util::{
    build_command, collect_child_dirs, display_name, run_command, run_command_quiet,
};
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::process::Stdio;

// ── Integration builds ──────────────────────────────────────────────────

pub struct BuildCommand {
    pub cmd: &'static str,
    pub args: &'static [&'static str],
    pub cwd: Option<&'static str>,
}

pub struct IntegrationBuild {
    pub name: &'static str,
    pub commands: &'static [BuildCommand],
    pub run_commands: &'static [BuildCommand],
}

pub const INTEGRATION_BUILDS: &[IntegrationBuild] = &[
    IntegrationBuild {
        name: "node",
        commands: &[BuildCommand {
            cmd: "node",
            args: &["--check", "index.js"],
            cwd: Some("examples/integration/node"),
        }],
        run_commands: &[],
    },
    IntegrationBuild {
        name: "electron",
        commands: &[BuildCommand {
            cmd: "pnpm",
            args: &["run", "build"],
            cwd: Some("examples/integration/electron"),
        }],
        run_commands: &[],
    },
    IntegrationBuild {
        name: "rust",
        commands: &[BuildCommand {
            cmd: "cargo",
            args: &[
                "check",
                "--manifest-path",
                "examples/integration/rust/Cargo.toml",
            ],
            cwd: None,
        }],
        run_commands: &[],
    },
    IntegrationBuild {
        name: "ssr-performance-showdown",
        commands: &[BuildCommand {
            cmd: "cargo",
            args: &[
                "check",
                "--manifest-path",
                "examples/integration/ssr-performance-showdown/Cargo.toml",
            ],
            cwd: None,
        }],
        run_commands: &[],
    },
];

pub fn run_integration_builds() -> Result<(), String> {
    if INTEGRATION_BUILDS.is_empty() {
        eprintln!(
            "  {} no integration build entries configured",
            console::style("•").dim()
        );
        return Ok(());
    }

    for integration in INTEGRATION_BUILDS {
        eprintln!(
            "  {} integration: {}",
            console::style("•").dim(),
            integration.name
        );
        for command in integration.commands {
            let cwd = command.cwd.map(Path::new);
            run_command_quiet(command.cmd, command.args, cwd).map_err(|message| {
                format!(
                    "integration '{}' command failed: {}",
                    integration.name, message
                )
            })?;
        }
    }

    Ok(())
}

// ── App builds ──────────────────────────────────────────────────────────

fn is_example_app_dir(app_dir: &Path) -> bool {
    app_dir.join("package.json").is_file()
}

pub fn run_app_builds() -> Result<(), String> {
    use std::thread;

    let apps_root = Path::new("examples/app");
    let app_dirs: Vec<_> = collect_child_dirs(apps_root)?
        .into_iter()
        .filter(|app_dir| is_example_app_dir(app_dir))
        .collect();

    if app_dirs.is_empty() {
        eprintln!(
            "  {} no example apps found under examples/app",
            console::style("•").dim()
        );
        return Ok(());
    }

    // Build all apps in parallel — each is independent
    let handles: Vec<_> = app_dirs
        .into_iter()
        .map(|app_dir| {
            thread::spawn(move || {
                let app_name = display_name(&app_dir);
                if !has_script(&app_dir, "build") {
                    return Err(format!(
                        "app '{}' is missing a package.json build script at {}",
                        app_name,
                        app_dir.join("package.json").display()
                    ));
                }

                run_app_build_script(&app_dir)
                    .map_err(|message| format!("app '{}' build failed: {}", app_name, message))?;

                eprintln!("  {} app: {}", console::style("•").dim(), app_name);
                Ok(())
            })
        })
        .collect();

    let mut errors = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(_) => errors.push("thread panicked during app build".to_string()),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn run_example_builds() -> Result<(), String> {
    run_integration_builds()?;
    run_app_builds()
}

fn run_app_build_script(app_dir: &Path) -> Result<(), String> {
    let mut command = build_command("pnpm", &["run", "build"]);
    command
        .current_dir(app_dir)
        .env("CARGO_INCREMENTAL", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match command.output() {
        Ok(output) if output.status.success() => {
            if has_script(app_dir, "test:projection-contract") {
                run_app_projection_contract(app_dir)?;
            }
            Ok(())
        }
        Ok(output) => {
            let mut msg = String::new();
            if let Ok(s) = String::from_utf8(output.stdout) {
                msg.push_str(&s);
            }

            if let Ok(s) = String::from_utf8(output.stderr) {
                msg.push_str(&s);
            }
            if msg.is_empty() {
                msg = format!("exit code {}", output.status.code().unwrap_or(1));
            }
            Err(msg)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn run_app_projection_contract(app_dir: &Path) -> Result<(), String> {
    let mut command = build_command("pnpm", &["run", "test:projection-contract"]);
    command
        .current_dir(app_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let mut message = String::from_utf8_lossy(&output.stdout).into_owned();
    message.push_str(&String::from_utf8_lossy(&output.stderr));
    Err(message)
}

/// Check whether an app package declares a non-empty npm script.
fn has_script(app_dir: &Path, script: &str) -> bool {
    let Ok(content) = fs::read_to_string(app_dir.join("package.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(|scripts| scripts.get(script))
        .and_then(|script_value| script_value.as_str())
        .is_some_and(|script_text| !script_text.is_empty())
}

// ── Run integration with app ────────────────────────────────────────────

pub fn run_integration_app(integration: Option<&str>, app: Option<&str>) -> ExitCode {
    let (Some(integration_name), Some(app_name)) = (integration, app) else {
        eprintln!(
            "Usage: cargo xtask run <integration> <app>\n\n\
             Available integrations: {}\n\
             Available apps: {}",
            available_integrations(),
            available_apps().unwrap_or_else(|_| "(unable to list)".into()),
        );
        return ExitCode::FAILURE;
    };

    let Some(build) = find_integration(integration_name) else {
        eprintln!(
            "Unknown integration '{}'\nAvailable: {}",
            integration_name,
            available_integrations(),
        );
        return ExitCode::FAILURE;
    };

    let app_dir = Path::new("examples/app").join(app_name);
    if !app_dir.is_dir() {
        eprintln!(
            "Unknown app '{}'\nAvailable: {}",
            app_name,
            available_apps().unwrap_or_else(|_| "(unable to list)".into()),
        );
        return ExitCode::FAILURE;
    }

    eprintln!(
        "{} running {} with app {}",
        console::style("▸").cyan().bold(),
        integration_name,
        app_name
    );
    for cmd in build.run_commands {
        let mut args: Vec<&str> = cmd.args.to_vec();
        args.extend_from_slice(&["--app", app_name]);
        let cwd = cmd.cwd.map(Path::new);
        if let Err(message) = run_command(cmd.cmd, &args, cwd) {
            eprintln!("  {} {}", console::style("✘").red().bold(), message);
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

fn find_integration(name: &str) -> Option<&'static IntegrationBuild> {
    INTEGRATION_BUILDS.iter().find(|b| b.name == name)
}

fn available_integrations() -> String {
    INTEGRATION_BUILDS
        .iter()
        .map(|b| b.name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn available_apps() -> Result<String, String> {
    let dirs = collect_child_dirs(Path::new("examples/app"))?;
    let names: Vec<String> = dirs
        .iter()
        .filter(|d| is_example_app_dir(d))
        .map(|d| display_name(d))
        .collect();
    Ok(names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::is_example_app_dir;
    use std::fs;

    #[test]
    fn ignores_generated_directories_without_app_manifest() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::tempdir()?;
        let app_dir = temp.path().join("routes-advanced");
        fs::create_dir(&app_dir)?;
        fs::create_dir(app_dir.join("dist"))?;

        assert!(!is_example_app_dir(&app_dir));
        Ok(())
    }

    #[test]
    fn treats_package_directories_as_apps() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let app_dir = temp.path().join("calculator");
        fs::create_dir(&app_dir)?;
        fs::write(app_dir.join("package.json"), "{}\n")?;

        assert!(is_example_app_dir(&app_dir));
        Ok(())
    }
}
