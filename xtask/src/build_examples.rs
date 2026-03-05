//! Examples build tasks.

use crate::util::{collect_child_dirs, display_name, run_command};
use std::path::Path;
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
        eprintln!(
            "  {} no example apps found under examples/app",
            console::style("•").dim()
        );
        return Ok(());
    }

    for app_dir in app_dirs {
        let app_name = display_name(&app_dir);
        let src_dir = app_dir.join("src");
        if !src_dir.is_dir() {
            return Err(format!(
                "app '{}' is missing src directory at {}",
                app_name,
                src_dir.display()
            ));
        }

        let output_dir = app_dir.join("dist");

        eprintln!("  {} app: {}", console::style("•").dim(), app_name);

        let src_str = src_dir.to_string_lossy().to_string();
        let output_str = output_dir.to_string_lossy().to_string();

        // Apps ending in "-fast" use the FAST parser plugin
        let mut args: Vec<&str> = vec![
            "run",
            "-p",
            "webui-cli",
            "--",
            "build",
            &src_str,
            "--out",
            &output_str,
        ];

        if app_name.ends_with("-fast") {
            args.push("--plugin");
            args.push("fast");
        }

        run_command("cargo", &args, None)
            .map_err(|message| format!("app '{}' build failed: {}", app_name, message))?;
    }

    Ok(())
}

pub fn run_example_builds() -> Result<(), String> {
    run_integration_builds()?;
    run_app_builds()
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
    let names: Vec<String> = dirs.iter().map(|d| display_name(d)).collect();
    Ok(names.join(", "))
}
