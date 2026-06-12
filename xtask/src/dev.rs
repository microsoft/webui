// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Dev mode: run server + client watch concurrently for an example app.
//!
//! Usage: `cargo xtask dev todo-fast`

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::process::{self, ManagedChild, ReservedPort};
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

    let has_api = has_script(&app_dir, "start:api");
    let server_port = read_port(&app_dir);
    let api_port = if has_api {
        read_api_port(&app_dir)
    } else {
        None
    };
    let reserved_ports = collect_reserved_ports(server_port, api_port);

    if let Err(message) = process::ensure_reserved_ports_available(app_name, &reserved_ports) {
        eprintln!("\n  {} {}", console::style("✘").red().bold(), message);
        eprintln!(
            "  {} Stop the process using the occupied port, or update {}",
            console::style("hint:").dim(),
            console::style(app_dir.join("package.json").display()).bold(),
        );
        eprintln!(
            "  {} Stale dev servers from previous sessions can leave ports occupied.\n",
            console::style("hint:").dim(),
        );
        return ExitCode::FAILURE;
    }

    let port = server_port
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());

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
        eprintln!(
            "  {} API        {}",
            console::style("▸").dim(),
            console::style(format!(
                "http://127.0.0.1:{}/",
                api_port
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".into())
            ))
            .dim(),
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

    // Start client bundler (watch mode).
    //
    // esbuild only emits color when its stderr is a TTY and ignores the
    // `FORCE_COLOR`/`CLICOLOR_FORCE` env vars; under our output pipe it would
    // print plain text. Forward esbuild's `--color=true` flag (via pnpm) when
    // we are attached to a terminal so its build errors stay colored, matching
    // the server's behavior. All example `start:client` scripts use esbuild.
    //
    // The flag is appended WITHOUT a `--` separator: pnpm forwards extra args
    // verbatim (it does not consume `--`), so a `--` would reach esbuild and be
    // rejected as an invalid build flag.
    let client_args: &[&str] = if console::colors_enabled_stderr() {
        &["start:client", "--color=true"]
    } else {
        &["start:client"]
    };
    match process::spawn_child_prefixed(
        "client",
        "pnpm",
        client_args,
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
fn read_port(app_dir: &Path) -> Option<u16> {
    read_script_flag_port(app_dir, "start:server", "--port")
}

/// Read the API port from package.json start:server script (--api-port).
fn read_api_port(app_dir: &Path) -> Option<u16> {
    read_script_flag_port(app_dir, "start:server", "--api-port")
}

fn read_script_flag_port(app_dir: &Path, script_name: &str, flag: &str) -> Option<u16> {
    let content = fs::read_to_string(app_dir.join("package.json")).ok()?;
    let script = serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .get("scripts")?
        .get(script_name)?
        .as_str()?
        .to_string();

    script
        .split_whitespace()
        .zip(script.split_whitespace().skip(1))
        .find(|(candidate_flag, _)| *candidate_flag == flag)
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

fn collect_reserved_ports(
    server_port: Option<u16>,
    api_port: Option<u16>,
) -> Vec<ReservedPort<'static>> {
    let mut ports = Vec::with_capacity(2);
    if let Some(port) = server_port {
        ports.push(ReservedPort::new("server", port));
    }
    if let Some(port) = api_port {
        ports.push(ReservedPort::new("api", port));
    }
    ports
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_app_dir(package_json: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("package.json"), package_json).unwrap();
        dir
    }

    #[test]
    fn test_read_ports_from_start_server_script() {
        let app = create_app_dir(
            r#"{
                "scripts": {
                    "start:server": "cargo run -p microsoft-webui-cli -- serve ./src --port 3003 --api-port 3013 --watch",
                    "start:api": "node dist/api.js"
                }
            }"#,
        );

        assert_eq!(read_port(app.path()), Some(3003));
        assert_eq!(read_api_port(app.path()), Some(3013));
    }
}
