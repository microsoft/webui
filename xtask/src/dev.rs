//! Dev mode: run server + client watch concurrently for an example app.
//!
//! Usage: `cargo xtask dev todo-fast`

use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::util::{collect_child_dirs, display_name, Printer};

pub fn run(app: Option<&str>) -> ExitCode {
    let p = Printer::new();

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
        p.cyan.apply_to("▸"),
        p.bold.apply_to(app_name),
    );

    let mut server = match spawn_child("server", "pnpm", &["start:server"], &app_dir, &p) {
        Some(c) => c,
        None => return ExitCode::FAILURE,
    };
    let mut client = match spawn_child("client", "pnpm", &["start:client"], &app_dir, &p) {
        Some(c) => c,
        None => {
            let _ = server.kill();
            let _ = server.wait();
            return ExitCode::FAILURE;
        }
    };

    // Catch Ctrl+C so we can kill children and exit cleanly
    let ctrlc = Arc::new(AtomicBool::new(false));
    let ctrlc_flag = ctrlc.clone();
    ctrlc::set_handler(move || {
        ctrlc_flag.store(true, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    // Poll for Ctrl+C or process exit instead of blocking on .status()
    loop {
        if ctrlc.load(Ordering::SeqCst) {
            let _ = server.kill();
            let _ = client.kill();
            let _ = server.wait();
            let _ = client.wait();
            eprintln!("\n  {} stopped", p.green.apply_to("✔"));
            return ExitCode::SUCCESS;
        }

        let server_done = matches!(server.try_wait(), Ok(Some(_)));
        let client_done = matches!(client.try_wait(), Ok(Some(_)));

        if server_done || client_done {
            if !server_done {
                let _ = server.kill();
            }
            if !client_done {
                let _ = client.kill();
            }
            let s = server.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
            let c = client.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);

            if ctrlc.load(Ordering::SeqCst) {
                eprintln!("\n  {} stopped", p.green.apply_to("✔"));
                return ExitCode::SUCCESS;
            }

            eprintln!(
                "  {} dev processes exited (server={}, client={})",
                p.red.apply_to("✘"),
                s,
                c,
            );
            return ExitCode::FAILURE;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_child(
    label: &str,
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    p: &Printer,
) -> Option<Child> {
    eprintln!(
        "  {} starting {}",
        p.dim.apply_to("→"),
        p.cyan.apply_to(label),
    );

    match Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!("  [{}] failed to start: {}", label, e);
            None
        }
    }
}

fn available_apps() -> Result<String, String> {
    let dirs = collect_child_dirs(Path::new("examples/app"))?;
    let names: Vec<String> = dirs.iter().map(|d| display_name(d)).collect();
    Ok(names.join(", "))
}
