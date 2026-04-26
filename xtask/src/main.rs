// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod build_examples;
mod build_wasm;
mod dev;
mod e2e;
mod e2e_approve;
mod license_headers;
mod process;
mod publish;
mod util;
mod version;

use std::process::ExitCode;
use std::time::Instant;
use util::{
    ensure_cargo_install, ensure_rustup_component, run_command, run_command_quiet, workspace_root,
};

fn main() -> ExitCode {
    let workspace_root = match workspace_root() {
        Ok(path) => path,
        Err(message) => {
            eprintln!("xtask error: {message}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = std::env::set_current_dir(&workspace_root) {
        eprintln!(
            "xtask error: failed to set current directory to workspace root {}: {}",
            workspace_root.display(),
            error
        );
        return ExitCode::FAILURE;
    }

    let args: Vec<String> = std::env::args().collect();
    let task = args.get(1).map(|s| s.as_str());

    match task {
        Some("check") => check(),
        Some("fmt") => run_steps(&[Step::FMT]),
        Some("clippy") => run_steps(&[Step::CLIPPY]),
        Some("deny") => run_steps(&[Step::DENY]),
        Some("test") => run_steps(&[Step::TEST]),
        Some("build") => run_steps(&[Step::BUILD, Step::BUILD_EXAMPLES]),
        Some("build-examples") => run_steps(&[Step::BUILD_EXAMPLES]),
        Some("build-wasm") => run_steps(&[Step::BUILD_WASM]),
        Some("docs") => run_steps(&[Step::DOCS]),
        Some("bench") => {
            let target = args.get(2).map(|s| s.as_str());
            let extra_args: Vec<&str> = args.iter().skip(3).map(String::as_str).collect();
            bench(target, &extra_args)
        }
        Some("run") => {
            let integration = args.get(2).map(|s| s.as_str());
            let app = args.get(3).map(|s| s.as_str());
            build_examples::run_integration_app(integration, app)
        }
        Some("dev") => {
            let app = args.get(2).map(|s| s.as_str());
            dev::run(app)
        }
        Some("e2e") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            e2e::run(&extra)
        }
        Some("e2e-approve") => {
            let run_id = args.get(2).map(|s| s.as_str());
            e2e_approve::run(run_id)
        }
        Some("version") => {
            let ver = args.get(2).map(|s| s.as_str());
            version::run(ver)
        }
        Some("publish-stage") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            publish::run_stage(&extra)
        }
        Some("license-headers") => {
            let fix = args.iter().any(|a| a == "--fix");
            if fix {
                match license_headers::fix() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(msg) => {
                        eprintln!("{msg}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                run_steps(&[Step::LICENSE_HEADERS])
            }
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "Usage: cargo xtask <COMMAND>\n\n\
         Commands:\n  \
           check   Run all checks (fmt, clippy, deny, test, build, bench validate, docs)\n  \
           fmt     Check formatting\n  \
           clippy  Run clippy lints\n  \
           deny    Run cargo-deny license/advisory checks\n  \
           test    Run all tests\n  \
           build   Build the workspace\n  \
           build-examples  Build all example integrations and apps\n  \
           build-wasm  Build WASM playground module\n  \
           docs    Build the documentation site\n  \
           bench <name> [-- <criterion args>]  Run benchmarks for a target crate (parser, handler, protocol, expressions, state, webui, all)\n  \
           dev <app>  Run example app in dev mode (server + client watch concurrently)\n  \
           e2e [--update-snapshots]  Run Playwright E2E tests for all example apps\n  \
           e2e-approve [run-id]  Download CI screenshot baselines and apply locally\n  \
           version <semver>  Update version across all Cargo.toml and package.json files\n  \
           publish-stage [--target <triple|all>] [--profile release] [--native-only|--pack-only]  Stage release artifacts into publish/\n  \
           license-headers [--fix]  Check (or fix) license headers in source files"
    );
    ExitCode::SUCCESS
}

fn bench(target: Option<&str>, extra_args: &[&str]) -> ExitCode {
    let mut args = vec!["bench"];

    match target {
        Some("parser") | Some("webui-parser") | Some("microsoft-webui-parser") => {
            args.extend(["-p", "microsoft-webui-parser"]);
        }
        Some("handler") | Some("webui-handler") | Some("microsoft-webui-handler") => {
            args.extend(["-p", "microsoft-webui-handler"]);
        }
        Some("protocol") | Some("webui-protocol") | Some("microsoft-webui-protocol") => {
            args.extend(["-p", "microsoft-webui-protocol"]);
        }
        Some("expressions") | Some("webui-expressions") | Some("microsoft-webui-expressions") => {
            args.extend(["-p", "microsoft-webui-expressions"]);
        }
        Some("state") | Some("webui-state") | Some("microsoft-webui-state") => {
            args.extend(["-p", "microsoft-webui-state"]);
        }
        Some("contact-book") => {
            args.extend(["-p", "microsoft-webui", "--bench", "contact_book_bench"]);
        }
        Some("all") | None => {
            args.extend(["--workspace"]);
        }
        Some(other) => {
            eprintln!("Unknown bench target '{other}'. Supported targets: parser, handler, protocol, expressions, state, webui, all");
            return ExitCode::FAILURE;
        }
    }

    args.extend(extra_args.iter().copied());

    match run_command("cargo", &args, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("bench failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> ExitCode {
    let total_start = Instant::now();

    // Phase 1: Sequential lint checks (fail-fast)
    eprintln!("\n{} Phase 1 — lint", console::style("▸").cyan().bold());
    if run_steps(&[Step::LICENSE_HEADERS, Step::FMT, Step::CLIPPY]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    // Phase 2: Parallel — deny + test
    if run_parallel(&[Step::DENY, Step::TEST]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    // Phase 3: Parallel — build + build-wasm
    if run_parallel(&[Step::BUILD, Step::BUILD_WASM]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    // Phase 4: Parallel — examples (each independent) + bench + docs
    if run_parallel(&[Step::BUILD_EXAMPLES, Step::BENCH_VALIDATE, Step::DOCS]) != ExitCode::SUCCESS
    {
        return ExitCode::FAILURE;
    }

    let total = total_start.elapsed().as_secs_f64();
    eprintln!(
        "\n{} All checks passed {}\n",
        console::style("✨").green(),
        console::style(format!("({total:.1}s)")).dim(),
    );
    ExitCode::SUCCESS
}

// ── Step runner ─────────────────────────────────────────────────────────

struct Step {
    name: &'static str,
    run: fn() -> Result<(), String>,
}

impl Step {
    const LICENSE_HEADERS: Self = Self {
        name: "license-headers",
        run: license_headers::check,
    };
    const FMT: Self = Self {
        name: "fmt",
        run: || {
            ensure_rustup_component("rustfmt")?;
            run_command_quiet("cargo", &["fmt", "--all", "--check"], None)
        },
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        run: || {
            ensure_rustup_component("clippy")?;
            run_command_quiet(
                "cargo",
                &["clippy", "--workspace", "--", "-D", "warnings"],
                None,
            )
        },
    };
    const DENY: Self = Self {
        name: "deny",
        run: || {
            ensure_cargo_install("cargo-deny", "cargo-deny")?;
            run_command_quiet("cargo", &["deny", "check"], None)
        },
    };
    const TEST: Self = Self {
        name: "test",
        run: || run_command_quiet("cargo", &["test", "--workspace"], None),
    };
    const BUILD: Self = Self {
        name: "build",
        run: || {
            run_command_quiet(
                "cargo",
                &["build", "--workspace", "--exclude", "xtask"],
                None,
            )
        },
    };
    const BUILD_EXAMPLES: Self = Self {
        name: "build (examples)",
        run: build_examples::run_example_builds,
    };
    const BUILD_WASM: Self = Self {
        name: "build (wasm)",
        run: build_wasm::run,
    };
    const DOCS: Self = Self {
        name: "docs",
        run: || {
            // Build the standalone webui-press binary
            run_command_quiet("cargo", &["build", "-p", "microsoft-webui-press"], None)?;
            run_command_quiet("pnpm", &["--filter", "@webui/docs", "build"], None)
        },
    };
    const BENCH_VALIDATE: Self = Self {
        name: "bench (validate)",
        run: || {
            run_command_quiet(
                "cargo",
                &[
                    "bench",
                    "-p",
                    "microsoft-webui",
                    "--bench",
                    "contact_book_bench",
                    "--",
                    "--test",
                ],
                None,
            )
        },
    };
}

fn run_steps(steps: &[Step]) -> ExitCode {
    for step in steps {
        let start = Instant::now();
        match (step.run)() {
            Ok(()) => {
                let elapsed = start.elapsed().as_secs_f64();
                eprintln!(
                    "  {} {} {}",
                    console::style("✔").green(),
                    step.name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
            }
            Err(message) => {
                let elapsed = start.elapsed().as_secs_f64();
                eprintln!(
                    "  {} {} {}",
                    console::style("✘").red().bold(),
                    step.name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
                print_failure_output(&message);
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Run multiple steps in parallel threads. Waits for all to complete,
/// then reports results. Fails if any step failed.
fn run_parallel(steps: &[Step]) -> ExitCode {
    use std::thread;

    let names: Vec<&str> = steps.iter().map(|s| s.name).collect();
    eprintln!(
        "\n{} {}",
        console::style("▸").cyan().bold(),
        names.join(" + "),
    );

    let handles: Vec<_> = steps
        .iter()
        .map(|step| {
            let name = step.name;
            let run = step.run;
            thread::spawn(move || {
                let start = Instant::now();
                let result = run();
                let elapsed = start.elapsed().as_secs_f64();
                (name, result, elapsed)
            })
        })
        .collect();

    let mut all_ok = true;
    let mut failures: Vec<(&str, String)> = Vec::new();

    for handle in handles {
        match handle.join() {
            Ok((name, Ok(()), elapsed)) => {
                eprintln!(
                    "  {} {} {}",
                    console::style("✔").green(),
                    name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
            }
            Ok((name, Err(message), elapsed)) => {
                eprintln!(
                    "  {} {} {}",
                    console::style("✘").red().bold(),
                    name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
                failures.push((name, message));
                all_ok = false;
            }
            Err(_) => {
                eprintln!("  {} (thread panicked)", console::style("✘").red().bold());
                all_ok = false;
            }
        }
    }

    for (name, output) in &failures {
        print_failure_output_with_name(name, output);
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_failure_output(output: &str) {
    let separator = console::style("─".repeat(60)).dim();
    eprintln!("    {separator}");
    for line in output.lines().take(30) {
        eprintln!("    {line}");
    }
    let total = output.lines().count();
    if total > 30 {
        eprintln!(
            "    {} ({} more lines)",
            console::style("...").dim(),
            total - 30,
        );
    }
    eprintln!("    {separator}");
}

fn print_failure_output_with_name(name: &str, output: &str) {
    let separator = console::style("─".repeat(60)).dim();
    eprintln!(
        "\n    {} {} output:",
        console::style("✘").red().bold(),
        name,
    );
    eprintln!("    {separator}");
    for line in output.lines().take(30) {
        eprintln!("    {line}");
    }
    let total = output.lines().count();
    if total > 30 {
        eprintln!(
            "    {} ({} more lines)",
            console::style("...").dim(),
            total - 30,
        );
    }
    eprintln!("    {separator}");
}
