//! App and integration build tasks.

use crate::util::{collect_child_dirs, display_name, run_command};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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
        name: "hyper",
        commands: &[BuildCommand {
            cmd: "cargo",
            args: &["build"],
            cwd: Some("examples/integration/hyper"),
        }],
        run_commands: &[BuildCommand {
            cmd: "cargo",
            args: &["run", "--"],
            cwd: Some("examples/integration/hyper"),
        }],
    },
    IntegrationBuild {
        name: "tiny_http",
        commands: &[BuildCommand {
            cmd: "cargo",
            args: &["build"],
            cwd: Some("examples/integration/tiny_http"),
        }],
        run_commands: &[BuildCommand {
            cmd: "cargo",
            args: &["run", "--"],
            cwd: Some("examples/integration/tiny_http"),
        }],
    },
    IntegrationBuild {
        name: "node-express",
        commands: &[
            BuildCommand {
                cmd: "cargo",
                args: &["build", "-p", "webui-node"],
                cwd: None,
            },
            BuildCommand {
                cmd: "npm",
                args: &["ci", "--no-audit", "--no-fund"],
                cwd: Some("examples/integration/node-express"),
            },
        ],
        run_commands: &[BuildCommand {
            cmd: "node",
            args: &["src/index.js"],
            cwd: Some("examples/integration/node-express"),
        }],
    },
];

pub fn run_integration_builds() -> Result<(), String> {
    if INTEGRATION_BUILDS.is_empty() {
        eprintln!("  • no integration build entries configured");
        return Ok(());
    }

    for integration in INTEGRATION_BUILDS {
        eprintln!("  • integration: {}", integration.name);
        for command in integration.commands {
            let cwd = command.cwd.map(Path::new);
            run_command(command.cmd, command.args, cwd).map_err(|message| {
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

pub fn run_app_builds() -> Result<(), String> {
    let apps_root = Path::new("examples/app");
    let app_dirs = collect_child_dirs(apps_root)?;

    if app_dirs.is_empty() {
        eprintln!("  • no example apps found under examples/app");
        return Ok(());
    }

    for app_dir in app_dirs {
        let app_name = display_name(&app_dir);
        let templates_dir = app_dir.join("templates");
        if !templates_dir.is_dir() {
            return Err(format!(
                "app '{}' is missing templates directory at {}",
                app_name,
                templates_dir.display()
            ));
        }

        let output_dir = PathBuf::from("target")
            .join("xtask")
            .join("app-builds")
            .join(app_name.as_str());

        eprintln!("  • app: {}", app_name);
        run_command(
            "cargo",
            &[
                "run",
                "-p",
                "webui-cli",
                "--",
                "build",
                templates_dir.to_string_lossy().as_ref(),
                "--out",
                output_dir.to_string_lossy().as_ref(),
            ],
            None,
        )
        .map_err(|message| format!("app '{}' build failed: {}", app_name, message))?;
    }

    Ok(())
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

    eprintln!("▸ running {} with app {}", integration_name, app_name);
    for cmd in build.run_commands {
        let mut args: Vec<&str> = cmd.args.to_vec();
        args.extend_from_slice(&["--app", app_name]);
        let cwd = cmd.cwd.map(Path::new);
        if let Err(message) = run_command(cmd.cmd, &args, cwd) {
            eprintln!("  ✘ {}", message);
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
    let names: Vec<String> = dirs.iter().map(|d| display_name(d)).collect();
    Ok(names.join(", "))
}
