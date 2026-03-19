// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! E2E test runner: starts example servers and runs Playwright tests in parallel.
//!
//! Usage: `cargo xtask e2e`
//!
//! Starts all example app servers on their unique ports, waits for them to be
//! ready, then runs `pnpm test` in parallel across all examples that have
//! Playwright configs. Reports results and cleans up servers on exit.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::process::{self, ManagedChild};
use crate::util;

/// Maximum time to wait for all server ports to become ready.
const PORT_TIMEOUT: Duration = Duration::from_secs(180);

/// An example app with its server configuration.
struct ExampleApp {
    name: &'static str,
    dir: &'static str,
    ports: &'static [u16],
    scripts: &'static [&'static str],
}

const APPS: &[ExampleApp] = &[
    ExampleApp {
        name: "hello-world",
        dir: "examples/app/hello-world",
        ports: &[3000],
        scripts: &["start:server"],
    },
    ExampleApp {
        name: "calculator",
        dir: "examples/app/calculator",
        ports: &[3002],
        scripts: &["start:server"],
    },
    ExampleApp {
        name: "contact-book-manager",
        dir: "examples/app/contact-book-manager",
        ports: &[3003, 3013],
        scripts: &["start:api", "start:server"],
    },
    ExampleApp {
        name: "commerce",
        dir: "examples/app/commerce",
        ports: &[3004],
        scripts: &["start:server"],
    },
    ExampleApp {
        name: "routes",
        dir: "examples/app/routes",
        ports: &[3005, 3015],
        scripts: &["start:api", "start:server"],
    },
];

struct TestResult {
    name: String,
    success: bool,
    output: String,
}

pub fn run() -> ExitCode {
    eprintln!("\n{} E2E tests", console::style("▸").cyan().bold(),);

    // Filter to apps that exist on disk
    let apps: Vec<&ExampleApp> = APPS
        .iter()
        .filter(|app| Path::new(app.dir).join("playwright.config.ts").exists())
        .collect();

    if apps.is_empty() {
        eprintln!("  No example apps with playwright.config.ts found");
        return ExitCode::FAILURE;
    }

    // Build client JS bundles (esbuild, one-shot, no --watch)
    eprintln!(
        "\n{} Building client bundles...",
        console::style("▸").cyan().bold(),
    );
    for app in &apps {
        let dir = PathBuf::from(app.dir);
        let index_ts = dir.join("src").join("index.ts");
        if !index_ts.exists() {
            continue;
        }
        let out = dir.join("dist").join("index.js");
        let out_str = out.to_string_lossy();
        let src_str = index_ts.to_string_lossy();
        match util::run_command_quiet(
            "npx",
            &[
                "esbuild",
                &src_str,
                "--bundle",
                "--outfile",
                &out_str,
                "--format=esm",
                "--sourcemap",
            ],
            Some(&dir),
        ) {
            Ok(()) => {
                eprintln!("  {} {}", console::style("✔").green(), app.name);
            }
            Err(msg) => {
                eprintln!(
                    "  {} {} — client build failed",
                    console::style("✘").red().bold(),
                    app.name,
                );
                eprintln!("    {msg}");
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!(
        "\n{} Starting servers for {} apps...",
        console::style("▸").cyan().bold(),
        apps.len(),
    );

    // Ctrl+C handler
    let ctrlc = Arc::new(AtomicBool::new(false));
    let flag = ctrlc.clone();
    if ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
    })
    .is_err()
    {
        eprintln!("  Warning: could not set Ctrl+C handler");
    }

    // Start all servers
    let mut servers: Vec<ManagedChild> = Vec::new();
    for app in &apps {
        let dir = PathBuf::from(app.dir);
        for script in app.scripts {
            eprintln!(
                "  {} {} → {}",
                console::style("▸").dim(),
                console::style(app.name).cyan(),
                script,
            );
            match process::spawn_child_quiet(
                &format!("{}/{}", app.name, script),
                "pnpm",
                &[script],
                &dir,
            ) {
                Some(child) => servers.push(child),
                None => {
                    eprintln!(
                        "  {} Failed to start {} for {}",
                        console::style("✘").red(),
                        script,
                        app.name
                    );
                    kill_servers(&mut servers);
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    // Wait for all ports
    eprintln!(
        "\n{} Waiting for ports...",
        console::style("▸").cyan().bold(),
    );
    let all_ports: Vec<u16> = apps.iter().flat_map(|a| a.ports.iter().copied()).collect();
    for port in &all_ports {
        if ctrlc.load(Ordering::SeqCst) {
            kill_servers(&mut servers);
            return ExitCode::SUCCESS;
        }
        if !wait_for_port(*port, PORT_TIMEOUT, &ctrlc) {
            eprintln!(
                "  {} Port {} did not become ready within {}s",
                console::style("✘").red(),
                port,
                PORT_TIMEOUT.as_secs(),
            );
            kill_servers(&mut servers);
            return ExitCode::FAILURE;
        }
        eprintln!("  {} Port {} ready", console::style("✔").green(), port);
    }

    // Run tests in parallel
    eprintln!(
        "\n{} Running Playwright tests...\n",
        console::style("▸").cyan().bold(),
    );

    let handles: Vec<_> = apps
        .iter()
        .map(|app| {
            let name = app.name.to_string();
            let dir = PathBuf::from(app.dir);
            thread::spawn(move || {
                let (success, output) = run_test(&name, &dir);
                TestResult {
                    name,
                    success,
                    output,
                }
            })
        })
        .collect();

    let mut results: Vec<TestResult> = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.join() {
            Ok(result) => results.push(result),
            Err(_) => {
                results.push(TestResult {
                    name: "(thread panicked)".into(),
                    success: false,
                    output: "Test thread panicked unexpectedly".into(),
                });
            }
        }
    }

    // Print results
    eprintln!();
    let mut all_passed = true;
    for result in &results {
        if result.success {
            eprintln!(
                "  {} {}",
                console::style("✔").green(),
                console::style(&result.name).bold(),
            );
        } else {
            all_passed = false;
            eprintln!(
                "  {} {}",
                console::style("✘").red().bold(),
                console::style(&result.name).bold(),
            );
            let separator = console::style("─".repeat(60)).dim();
            eprintln!("    {separator}");
            for line in result.output.lines().take(40) {
                eprintln!("    {line}");
            }
            let total = result.output.lines().count();
            if total > 40 {
                eprintln!(
                    "    {} ({} more lines)",
                    console::style("...").dim(),
                    total - 40,
                );
            }
            eprintln!("    {separator}");
        }
    }

    // Cleanup servers
    kill_servers(&mut servers);

    if all_passed {
        eprintln!(
            "\n{} All E2E tests passed ({} apps)\n",
            console::style("✨").green(),
            results.len(),
        );
        ExitCode::SUCCESS
    } else {
        let failed = results.iter().filter(|r| !r.success).count();
        eprintln!(
            "\n{} {} of {} apps failed\n",
            console::style("✘").red().bold(),
            failed,
            results.len(),
        );
        ExitCode::FAILURE
    }
}

fn run_test(name: &str, dir: &Path) -> (bool, String) {
    let mut cmd = util::build_command("pnpm", &["test"]);
    cmd.current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("Failed to spawn pnpm test: {e}")),
    };

    match child.wait_with_output() {
        Ok(output) => {
            let mut combined = String::new();
            if let Ok(s) = String::from_utf8(output.stdout) {
                combined.push_str(&s);
            }
            if let Ok(s) = String::from_utf8(output.stderr) {
                combined.push_str(&s);
            }
            (output.status.success(), combined)
        }
        Err(e) => (false, format!("Failed to wait for {name}: {e}")),
    }
}

fn wait_for_port(port: u16, timeout: Duration, ctrlc: &AtomicBool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if ctrlc.load(Ordering::SeqCst) {
            return false;
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

fn kill_servers(servers: &mut [ManagedChild]) {
    for server in servers.iter_mut() {
        process::terminate_gracefully(server);
    }
}
