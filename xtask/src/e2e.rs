// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! E2E test runner: starts example servers and runs Playwright tests in parallel.
//!
//! Usage: `cargo xtask e2e [--no-docker] [--update-snapshots]`
//!
//! By default, tests run inside a Docker container using the official
//! Playwright image for cross-platform screenshot consistency. Use
//! `--no-docker` to run directly on the host (visual regression results may
//! differ across platforms).
//!
//! Starts all example app servers on their unique ports, waits for them to be
//! ready, then runs Playwright tests in parallel across all examples that have
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

/// Docker execution context for a test run.
struct DockerCtx {
    workspace_root: PathBuf,
    image: String,
}

/// E2E configuration parsed from CLI arguments.
struct E2eConfig {
    use_docker: bool,
    update_snapshots: bool,
}

impl E2eConfig {
    fn from_args(args: &[String]) -> Self {
        Self {
            use_docker: !args.iter().any(|a| a == "--no-docker"),
            update_snapshots: args.iter().any(|a| a == "--update-snapshots"),
        }
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let config = E2eConfig::from_args(args);

    let mode_label = if config.use_docker {
        "Docker"
    } else {
        "direct"
    };
    eprintln!(
        "\n{} E2E tests ({})",
        console::style("▸").cyan().bold(),
        console::style(mode_label).bold(),
    );

    // Docker setup: verify Docker availability and pull the Playwright image
    let docker_image = if config.use_docker {
        if let Err(_msg) = util::ensure_docker() {
            return ExitCode::FAILURE;
        }
        eprintln!("  {} Docker", console::style("✔").green());

        let pw_version = match util::playwright_version() {
            Ok(v) => v,
            Err(msg) => {
                eprintln!("  {} {msg}", console::style("✘").red().bold());
                return ExitCode::FAILURE;
            }
        };
        let image = format!("mcr.microsoft.com/playwright:v{pw_version}-noble");
        if let Err(msg) = util::ensure_docker_image(&image) {
            eprintln!("  {} {msg}", console::style("✘").red().bold());
            return ExitCode::FAILURE;
        }
        eprintln!(
            "  {} {}",
            console::style("✔").green(),
            console::style(&image).dim(),
        );
        Some(image)
    } else {
        eprintln!(
            "  {} Running without Docker — visual regression results may vary across platforms",
            console::style("⚠").yellow(),
        );
        None
    };

    let workspace_root = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!(
                "  {} Failed to get current directory: {e}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    // Filter to apps that exist on disk
    let apps: Vec<&ExampleApp> = APPS
        .iter()
        .filter(|app| Path::new(app.dir).join("playwright.config.ts").exists())
        .collect();

    if apps.is_empty() {
        eprintln!("  No example apps with playwright.config.ts found");
        return ExitCode::FAILURE;
    }

    // Build workspace packages (e.g., @microsoft/webui-router needs dist/index.js)
    eprintln!(
        "\n{} Building workspace packages...",
        console::style("▸").cyan().bold(),
    );
    match util::run_command_quiet(
        "pnpm",
        &["--filter", "@microsoft/webui-router", "build"],
        None,
    ) {
        Ok(()) => eprintln!("  {} webui-router", console::style("✔").green()),
        Err(msg) => {
            eprintln!(
                "  {} webui-router build failed",
                console::style("✘").red().bold(),
            );
            eprintln!("    {msg}");
            return ExitCode::FAILURE;
        }
    }

    // Build client JS bundles (esbuild, one-shot, no --watch)
    eprintln!(
        "\n{} Building client bundles...",
        console::style("▸").cyan().bold(),
    );
    for app in &apps {
        let dir = PathBuf::from(app.dir);
        if !dir.join("src").join("index.ts").exists() {
            continue;
        }
        // Use relative paths — cwd is set to the app dir
        match util::run_command_quiet(
            "npx",
            &[
                "esbuild",
                "src/index.ts",
                "--bundle",
                "--outfile=dist/index.js",
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
            let update_snapshots = config.update_snapshots;
            let docker_ctx = docker_image.as_ref().map(|image| DockerCtx {
                workspace_root: workspace_root.clone(),
                image: image.clone(),
            });
            thread::spawn(move || {
                let start = Instant::now();
                let (success, output) = match &docker_ctx {
                    Some(ctx) => run_test_docker(&name, &dir, ctx, update_snapshots),
                    None => run_test(&name, &dir, update_snapshots),
                };
                let elapsed = start.elapsed().as_secs_f64();
                let icon = if success {
                    console::style("✔").green().to_string()
                } else {
                    console::style("✘").red().bold().to_string()
                };
                eprintln!(
                    "  {icon} {} {}",
                    console::style(&name).bold(),
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
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

    // Print failure details
    eprintln!();
    let mut all_passed = true;
    for result in &results {
        if !result.success {
            all_passed = false;
            eprintln!(
                "  {} {} — full output:",
                console::style("✘").red().bold(),
                console::style(&result.name).bold(),
            );
            let separator = console::style("─".repeat(60)).dim();
            eprintln!("    {separator}");
            for line in result.output.lines() {
                eprintln!("    {line}");
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

fn run_test(name: &str, dir: &Path, update_snapshots: bool) -> (bool, String) {
    let test_args: Vec<&str> = if update_snapshots {
        vec!["test", "--", "--update-snapshots"]
    } else {
        vec!["test"]
    };
    let mut cmd = util::build_command("pnpm", &test_args);
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

/// Run Playwright tests inside a Docker container for cross-platform consistency.
///
/// On Linux, the container uses `--network=host` so `127.0.0.1` reaches the
/// host directly. On macOS and Windows (Docker Desktop), the container uses
/// `host.docker.internal` via the `WEBUI_TEST_HOST` env var since
/// `--network=host` does not expose the real host loopback.
fn run_test_docker(
    name: &str,
    dir: &Path,
    ctx: &DockerCtx,
    update_snapshots: bool,
) -> (bool, String) {
    let ws = ctx.workspace_root.to_string_lossy();
    let app_dir = dir.to_string_lossy();

    let volume = format!("{ws}:/workspace");
    let workdir = format!("/workspace/{app_dir}");

    let mut args = vec!["run", "--rm", "--ipc=host"];

    #[cfg(target_os = "linux")]
    args.push("--network=host");

    #[cfg(not(target_os = "linux"))]
    {
        args.extend(["-e", "WEBUI_TEST_HOST=host.docker.internal"]);
    }

    args.extend([
        "-v",
        &volume,
        "-w",
        &workdir,
        &ctx.image,
        "npx",
        "playwright",
        "test",
    ]);
    if update_snapshots {
        args.push("--update-snapshots");
    }

    let mut cmd = util::build_command("docker", &args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("Failed to spawn docker for {name}: {e}")),
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
        Err(e) => (false, format!("Failed to wait for docker ({name}): {e}")),
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
