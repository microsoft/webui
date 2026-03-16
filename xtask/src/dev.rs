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
        eprintln!(
            "Usage: cargo xtask dev <app>\n\nAvailable apps: {}",
            available_apps().unwrap_or_else(|_| "(unable to list)".into()),
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
        "{} dev mode for {} (Ctrl+C to stop)",
        console::style("▸").cyan().bold(),
        console::style(app_name).bold(),
    );

    let mut children: Vec<(&str, ManagedChild)> = Vec::with_capacity(3);

    // If the app defines a start:api script, launch it first so the API is
    // available by the time the serve command starts fetching state.
    if has_script(&app_dir, "start:api") {
        match process::spawn_child("api", "pnpm", &["start:api"], &app_dir) {
            Some(c) => children.push(("api", c)),
            None => return ExitCode::FAILURE,
        }
    }

    match process::spawn_child("server", "pnpm", &["start:server"], &app_dir) {
        Some(c) => children.push(("server", c)),
        None => {
            kill_all(&mut children);
            return ExitCode::FAILURE;
        }
    }

    match process::spawn_child("client", "pnpm", &["start:client"], &app_dir) {
        Some(c) => children.push(("client", c)),
        None => {
            kill_all(&mut children);
            return ExitCode::FAILURE;
        }
    }

    process::wait_for_group(&mut children)
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
