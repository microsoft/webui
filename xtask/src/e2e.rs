// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! E2E test runner: starts example servers and runs Playwright tests in parallel.
//!
//! Usage: `cargo xtask e2e [--update-snapshots]`
//!
//! Starts all example app servers on their unique ports, waits for them to be
//! ready, then runs Playwright tests in parallel across all configured suites
//! that have Playwright configs. Reports results and cleans up servers on exit.
//!
//! Screenshot baselines are generated on CI (Ubuntu Linux). Locally, visual
//! regression tests may fail due to platform font differences — use
//! `--update-snapshots` to regenerate baselines from your environment.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::process::{self, ManagedChild, ReservedPort};
use crate::util;

/// Maximum time to wait for a server port to become ready.
/// CI environments are slower, but local runs also need slack: when 8+
/// `pnpm start:server` processes spawn concurrently, each runs `cargo run`,
/// and even with prebuilt artifacts cargo briefly checks the workspace
/// graph under a shared filesystem lock — easily exceeding the few seconds
/// of actual server startup.
fn port_timeout() -> Duration {
    if std::env::var_os("CI").is_some() {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(30)
    }
}

/// A Playwright suite with optional long-lived server processes.
struct PlaywrightSuite {
    name: &'static str,
    dir: &'static str,
    ports: &'static [u16],
    scripts: &'static [&'static str],
    build_client: bool,
    /// Optional pnpm script to run before starting servers (e.g. "build:e2e").
    pre_script: Option<&'static str>,
    test_script: &'static str,
    update_snapshots_script: &'static str,
}

const SUITES: &[PlaywrightSuite] = &[
    PlaywrightSuite {
        name: "hello-world",
        dir: "examples/app/hello-world",
        ports: &[3000],
        scripts: &["start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "calculator",
        dir: "examples/app/calculator",
        ports: &[3002],
        scripts: &["start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "contact-book-manager",
        dir: "examples/app/contact-book-manager",
        ports: &[3003, 3013],
        scripts: &["start:api", "start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "commerce",
        dir: "examples/app/commerce",
        ports: &[3004],
        scripts: &["start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "routes",
        dir: "examples/app/routes",
        ports: &[3018, 3008],
        scripts: &["start:api", "start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "todo-fast",
        dir: "examples/app/todo-fast",
        ports: &[3001],
        scripts: &["start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "component-assets",
        dir: "examples/app/component-assets",
        ports: &[3010],
        scripts: &["start:server"],
        build_client: false,
        pre_script: Some("build"),
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "todo-webui",
        dir: "examples/app/todo-webui",
        ports: &[3006],
        scripts: &["start:server"],
        build_client: true,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "webui-framework",
        dir: "packages/webui-framework",
        ports: &[],
        scripts: &[],
        build_client: false,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "webui-router",
        dir: "packages/webui-router",
        ports: &[39102],
        scripts: &["start:server"],
        build_client: false,
        pre_script: Some("build:e2e"),
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
    PlaywrightSuite {
        name: "webui-press",
        dir: "crates/webui-press",
        ports: &[],
        scripts: &[],
        build_client: false,
        pre_script: None,
        test_script: "test",
        update_snapshots_script: "test:update-snapshots",
    },
];

struct TestResult {
    name: String,
    success: bool,
    output: String,
}

pub fn run(args: &[String]) -> ExitCode {
    let update_snapshots = args.iter().any(|a| a == "--update-snapshots");

    eprintln!("\n{} E2E tests", console::style("▸").cyan().bold());

    // E2E suites serve a static build for the duration of the run and
    // never modify source files. Disable the dev server's filesystem
    // watcher (which `start:server` enables via --watch for dev mode)
    // so spurious filesystem events on CI cannot trigger a livereload
    // and reload the browser mid-test. The CLI honors WEBUI_NO_WATCH
    // and ignores --watch when set; children inherit our env.
    set_env_var("WEBUI_NO_WATCH", "1");

    // Filter to apps that exist on disk
    let suites: Vec<&PlaywrightSuite> = SUITES
        .iter()
        .filter(|suite| Path::new(suite.dir).join("package.json").exists())
        .collect();

    if suites.is_empty() {
        eprintln!("  No test suites with package.json found");
        return ExitCode::FAILURE;
    }

    let reserved_ports = collect_reserved_ports(&suites);
    if let Err(message) = process::ensure_reserved_ports_available("e2e", &reserved_ports) {
        eprintln!("\n  {} {}", console::style("✘").red().bold(), message);
        eprintln!(
            "  {} Stop the process using the occupied port, then rerun cargo xtask e2e.",
            console::style("hint:").dim(),
        );
        eprintln!(
            "  {} Stale dev servers from previous sessions commonly occupy example ports.\n",
            console::style("hint:").dim(),
        );
        return ExitCode::FAILURE;
    }

    // Build all workspace packages so example apps and test fixtures import
    // the current runtime rather than stale dist outputs.
    eprintln!(
        "\n{} Building workspace packages...",
        console::style("▸").cyan().bold(),
    );
    match util::run_command_quiet("pnpm", &["build"], None) {
        Ok(()) => eprintln!("  {}", console::style("✔ all packages").green()),
        Err(msg) => {
            eprintln!(
                "  {} workspace build failed",
                console::style("✘").red().bold(),
            );
            eprintln!("    {msg}");
            return ExitCode::FAILURE;
        }
    }

    // Build native Rust artifacts that example servers and Node test fixtures
    // load at runtime. Without this:
    // - Framework e2e tests load a stale libwebui_node dylib and produce
    //   mismatched SSR output (release profile, loaded by the Node addon).
    // - Example `pnpm start:server` scripts compile webui-cli on the critical
    //   path via `cargo run` and overflow the port-readiness timeout (debug
    //   profile, used by `cargo run` defaults).
    eprintln!(
        "\n{} Building Rust runtime artifacts...",
        console::style("▸").cyan().bold(),
    );
    match util::run_command_quiet(
        "cargo",
        &["build", "--release", "-p", "microsoft-webui-node"],
        None,
    ) {
        Ok(()) => eprintln!("  {}", console::style("✔ webui-node (release)").green()),
        Err(msg) => {
            eprintln!(
                "  {} webui-node release build failed",
                console::style("✘").red().bold(),
            );
            eprintln!("    {msg}");
            return ExitCode::FAILURE;
        }
    }
    match util::run_command_quiet("cargo", &["build", "-p", "microsoft-webui-cli"], None) {
        Ok(()) => eprintln!("  {}", console::style("✔ webui-cli (debug)").green()),
        Err(msg) => {
            eprintln!(
                "  {} webui-cli debug build failed",
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
    for suite in &suites {
        if !suite.build_client {
            continue;
        }
        let dir = PathBuf::from(suite.dir);
        if !dir.join("src").join("index.ts").exists() {
            continue;
        }
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
                eprintln!("  {} {}", console::style("✔").green(), suite.name);
            }
            Err(msg) => {
                eprintln!(
                    "  {} {} — client build failed",
                    console::style("✘").red().bold(),
                    suite.name,
                );
                eprintln!("    {msg}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Run pre-scripts (e.g. build:e2e for router client bundle)
    for suite in &suites {
        if let Some(script) = suite.pre_script {
            let dir = PathBuf::from(suite.dir);
            match util::run_command_quiet("pnpm", &[script], Some(&dir)) {
                Ok(()) => {
                    eprintln!(
                        "  {} {} ({})",
                        console::style("✔").green(),
                        suite.name,
                        script
                    );
                }
                Err(msg) => {
                    eprintln!(
                        "  {} {} — {} failed",
                        console::style("✘").red().bold(),
                        suite.name,
                        script,
                    );
                    eprintln!("    {msg}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    eprintln!(
        "\n{} Starting servers for {} apps...",
        console::style("▸").cyan().bold(),
        suites.len(),
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
    for suite in &suites {
        let dir = PathBuf::from(suite.dir);
        for script in suite.scripts {
            eprintln!(
                "  {} {} → {}",
                console::style("▸").dim(),
                console::style(suite.name).cyan(),
                script,
            );
            match process::spawn_child_quiet(
                &format!("{}/{}", suite.name, script),
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
                        suite.name
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
    let all_ports: Vec<(u16, &str)> = suites
        .iter()
        .flat_map(|suite| suite.ports.iter().map(|p| (*p, suite.name)))
        .collect();
    let timeout = port_timeout();
    for (port, app_name) in &all_ports {
        if ctrlc.load(Ordering::SeqCst) {
            kill_servers(&mut servers);
            return ExitCode::SUCCESS;
        }
        if !wait_for_port(*port, timeout, &ctrlc) {
            eprintln!(
                "  {} Port {} ({}) did not become ready within {}s",
                console::style("✘").red(),
                console::style(port).bold(),
                app_name,
                timeout.as_secs(),
            );
            kill_servers(&mut servers);
            return ExitCode::FAILURE;
        }
        eprintln!(
            "  {} Port {} ({})",
            console::style("✔").green(),
            console::style(port).bold(),
            console::style(app_name).cyan(),
        );
    }

    // Run tests in parallel with progress tracking
    let total = suites.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    eprintln!(
        "\n{} Running Playwright tests ({} suites in parallel)...\n",
        console::style("▸").cyan().bold(),
        total,
    );

    let handles: Vec<_> = suites
        .iter()
        .filter_map(|suite| {
            let name = suite.name.to_string();
            let dir = PathBuf::from(suite.dir);
            let test_script = if update_snapshots {
                // Skip suites that don't have an update-snapshots script.
                let pkg_path = dir.join("package.json");
                let has_update_script = std::fs::read_to_string(&pkg_path)
                    .ok()
                    .is_some_and(|json| json.contains(suite.update_snapshots_script));
                if !has_update_script {
                    let n = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    let progress = console::style(format!("[{n}/{total}]")).dim();
                    eprintln!(
                        "  {} {progress} {} {}",
                        console::style("–").dim(),
                        console::style(&name).bold(),
                        console::style("(skipped, no update-snapshots script)").dim(),
                    );
                    return None;
                }
                suite.update_snapshots_script
            } else {
                suite.test_script
            };
            let done = completed.clone();
            Some(thread::spawn(move || {
                let start = Instant::now();
                let (success, output) = run_test(&name, &dir, test_script);
                let elapsed = start.elapsed().as_secs_f64();
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                let icon = if success {
                    console::style("✔").green().to_string()
                } else {
                    console::style("✘").red().bold().to_string()
                };
                let progress = console::style(format!("[{n}/{total}]")).dim();
                eprintln!(
                    "  {icon} {progress} {} {}",
                    console::style(&name).bold(),
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
                TestResult {
                    name,
                    success,
                    output,
                }
            }))
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
            "\n{} All E2E tests passed ({} suites)\n",
            console::style("✨").green(),
            results.len(),
        );
        ExitCode::SUCCESS
    } else {
        let failed = results.iter().filter(|r| !r.success).count();
        eprintln!(
            "\n{} {} of {} suites failed\n",
            console::style("✘").red().bold(),
            failed,
            results.len(),
        );
        ExitCode::FAILURE
    }
}

fn run_test(name: &str, dir: &Path, script: &str) -> (bool, String) {
    let mut cmd = util::build_command("pnpm", &[script]);
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

fn collect_reserved_ports(suites: &[&PlaywrightSuite]) -> Vec<ReservedPort<'static>> {
    suites
        .iter()
        .flat_map(|suite| {
            suite
                .ports
                .iter()
                .copied()
                .map(|port| ReservedPort::new(suite.name, port))
        })
        .collect()
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

/// Set an environment variable on the current process. Wrapped here
/// because `std::env::set_var` is `unsafe` on newer Rust editions and
/// the workspace denies `unsafe_code`. This call is sound because
/// `xtask e2e` runs single-threaded up to the point we set it.
#[allow(unsafe_code)]
fn set_env_var(key: &str, value: &str) {
    // SAFETY: single-threaded; no other thread is reading or writing
    // process env. Children inherit the new value when spawned later.
    unsafe { std::env::set_var(key, value) }
}
