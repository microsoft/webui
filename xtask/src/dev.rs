//! Dev mode: run server + client watch concurrently for an example app.
//!
//! Usage: `cargo xtask dev todo-fast`

use std::path::Path;
use std::process::ExitCode;

use crate::process;
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

    let mut server = match process::spawn_child("server", "pnpm", &["start:server"], &app_dir) {
        Some(c) => c,
        None => return ExitCode::FAILURE,
    };
    let mut client = match process::spawn_child("client", "pnpm", &["start:client"], &app_dir) {
        Some(c) => c,
        None => {
            let _ = server.kill();
            let _ = server.wait();
            return ExitCode::FAILURE;
        }
    };

    process::wait_for_pair(&mut server, &mut client)
}

fn available_apps() -> Result<String, String> {
    let dirs = collect_child_dirs(Path::new("examples/app"))?;
    let names: Vec<String> = dirs.iter().map(|d| display_name(d)).collect();
    Ok(names.join(", "))
}
