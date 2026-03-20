// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Dev mode: run server + client watch concurrently for an example app.
//!
//! Usage: `cargo xtask dev todo-fast`

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::process::{self, ManagedChild};
use crate::util::{collect_child_dirs, display_name};

pub fn run(app: Option<&str>) -> ExitCode {
    let Some(app_name) = app else {
        print_usage();
        return ExitCode::FAILURE;
    };

    let app_dir = Path::new("examples/app").join(app_name);
    if !app_dir.is_dir() {
        eprintln!(
            "\n  {} Unknown app {}",
            console::style("✘").red().bold(),
            console::style(app_name).bold(),
        );
        print_usage();
        return ExitCode::FAILURE;
    }

    let port = read_port(&app_dir).unwrap_or_else(|| "?".into());
    let has_api = has_script(&app_dir, "start:api");

    // Header
    eprintln!();
    eprintln!(
        "  {} {}",
        console::style("⚡").cyan(),
        console::style(format!("WebUI Dev — {app_name}")).bold(),
    );
    eprintln!(
        "  {} URL        {}",
        console::style("▸").dim(),
        console::style(format!("http://127.0.0.1:{port}/"))
            .cyan()
            .bold(),
    );
    if has_api {
        let api_port = read_api_port(&app_dir).unwrap_or_else(|| "?".into());
        eprintln!(
            "  {} API        {}",
            console::style("▸").dim(),
            console::style(format!("http://127.0.0.1:{api_port}/")).dim(),
        );
    }
    eprintln!();

    let mut children: Vec<(&str, ManagedChild)> = Vec::with_capacity(3);

    // Start API server first (if the app has one)
    if has_api {
        match process::spawn_child_prefixed(
            "api",
            "pnpm",
            &["start:api"],
            &app_dir,
            console::Color::Magenta,
        ) {
            Some(c) => children.push(("api", c)),
            None => return ExitCode::FAILURE,
        }
    }

    // Start WebUI server
    match process::spawn_child_prefixed(
        "server",
        "pnpm",
        &["start:server"],
        &app_dir,
        console::Color::Cyan,
    ) {
        Some(c) => children.push(("server", c)),
        None => {
            kill_all(&mut children);
            return ExitCode::FAILURE;
        }
    }

    // Start client bundler (watch mode)
    match process::spawn_child_prefixed(
        "client",
        "pnpm",
        &["start:client"],
        &app_dir,
        console::Color::Green,
    ) {
        Some(c) => children.push(("client", c)),
        None => {
            kill_all(&mut children);
            return ExitCode::FAILURE;
        }
    }

    eprintln!(
        "\n  {} press {} to stop\n",
        console::style("✨").green(),
        console::style("Ctrl+C").bold(),
    );

    process::wait_for_group(&mut children)
}

fn print_usage() {
    let apps = available_apps().unwrap_or_else(|_| "(unable to list)".into());
    eprintln!(
        "\n  {} cargo xtask dev {}\n",
        console::style("Usage:").dim(),
        console::style("<app>").cyan(),
    );
    eprintln!("  Available apps: {}\n", console::style(apps).bold());
}

/// Read the server port from package.json start:server script.
fn read_port(app_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(app_dir.join("package.json")).ok()?;
    // Look for --port <number> in start:server script
    let server_script = serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .get("scripts")?
        .get("start:server")?
        .as_str()?
        .to_string();
    server_script
        .split_whitespace()
        .zip(server_script.split_whitespace().skip(1))
        .find(|(flag, _)| *flag == "--port")
        .map(|(_, port)| port.to_string())
}

/// Read the API port from package.json start:server script (--api-port).
fn read_api_port(app_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(app_dir.join("package.json")).ok()?;
    let server_script = serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .get("scripts")?
        .get("start:server")?
        .as_str()?
        .to_string();
    server_script
        .split_whitespace()
        .zip(server_script.split_whitespace().skip(1))
        .find(|(flag, _)| *flag == "--api-port")
        .map(|(_, port)| port.to_string())
}

/// Check whether `package.json` in `app_dir` contains a script with the given
/// name. Returns `false` on any I/O or parse error so the caller can skip the
/// script silently.
fn has_script(app_dir: &Path, script: &str) -> bool {
    let pkg_path = app_dir.join("package.json");
    let Ok(content) = fs::read_to_string(&pkg_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(|s| s.get(script))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
}

/// Force-kill and wait on every child spawned so far (used during early
/// startup failures).
fn kill_all(children: &mut [(&str, ManagedChild)]) {
    for (_, child) in children.iter_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn available_apps() -> Result<String, String> {
    let dirs = collect_child_dirs(Path::new("examples/app"))?;
    let names: Vec<String> = dirs.iter().map(|d| display_name(d)).collect();
    Ok(names.join(", "))
}
