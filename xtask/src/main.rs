mod build_examples;
mod build_wasm;
mod dev;
mod process;
mod util;
mod version;

use std::process::ExitCode;
use util::{ensure_cargo_install, ensure_rustup_component, run_command, workspace_root};

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
        Some("version") => {
            let ver = args.get(2).map(|s| s.as_str());
            version::run(ver)
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
           version <semver>  Update version across all Cargo.toml and package.json files"
    );
    ExitCode::SUCCESS
}

fn bench(target: Option<&str>, extra_args: &[&str]) -> ExitCode {
    let mut args = vec!["bench"];

    match target {
        Some("parser") | Some("webui-parser") => {
            args.extend(["-p", "webui-parser"]);
        }
        Some("handler") | Some("webui-handler") => {
            args.extend(["-p", "webui-handler"]);
        }
        Some("protocol") | Some("webui-protocol") => {
            args.extend(["-p", "webui-protocol"]);
        }
        Some("expressions") | Some("webui-expressions") => {
            args.extend(["-p", "webui-expressions"]);
        }
        Some("state") | Some("webui-state") => {
            args.extend(["-p", "webui-state"]);
        }
        Some("webui") | Some("contact-book") => {
            args.extend(["-p", "webui", "--bench", "contact_book_bench"]);
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
    run_steps(&[
        Step::FMT,
        Step::CLIPPY,
        Step::DENY,
        Step::TEST,
        Step::BUILD,
        Step::BUILD_EXAMPLES,
        Step::BUILD_WASM,
        Step::BENCH_VALIDATE,
        Step::DOCS,
    ])
}

// ── Step runner ─────────────────────────────────────────────────────────

struct Step {
    name: &'static str,
    run: fn() -> Result<(), String>,
}

impl Step {
    const FMT: Self = Self {
        name: "fmt",
        run: || {
            ensure_rustup_component("rustfmt")?;
            run_command("cargo", &["fmt", "--all", "--check"], None)
        },
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        run: || {
            ensure_rustup_component("clippy")?;
            run_command(
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
            run_command("cargo", &["deny", "check"], None)
        },
    };
    const TEST: Self = Self {
        name: "test",
        run: || run_command("cargo", &["test", "--workspace"], None),
    };
    const BUILD: Self = Self {
        name: "build",
        run: || {
            // Exclude xtask from the workspace build: it is already compiled
            // (it is the running process) and on Windows the OS locks the
            // running executable, causing "Access is denied" (os error 5).
            run_command(
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
        run: || run_command("pnpm", &["--filter", "@webui/docs", "build"], None),
    };
    const BENCH_VALIDATE: Self = Self {
        name: "bench (validate)",
        run: || {
            run_command(
                "cargo",
                &[
                    "bench",
                    "-p",
                    "webui",
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
        eprintln!("\n{} {}", console::style("▸").cyan().bold(), step.name);
        match (step.run)() {
            Ok(()) => eprintln!("  {} {}", console::style("✔").green(), step.name),
            Err(message) => {
                eprintln!("  {} {}", console::style("✘").red().bold(), step.name);
                eprintln!(
                    "  {} {} — {}",
                    console::style("✘").red().bold(),
                    step.name,
                    message
                );
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!("\n{} All checks passed\n", console::style("✨").green());
    ExitCode::SUCCESS
}
