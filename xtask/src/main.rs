mod build_examples;
mod build_wasm;
mod dev;
mod util;

use std::process::ExitCode;
use util::{run_command, workspace_root};

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
        Some("run") => {
            let integration = args.get(2).map(|s| s.as_str());
            let app = args.get(3).map(|s| s.as_str());
            build_examples::run_integration_app(integration, app)
        }
        Some("dev") => {
            let app = args.get(2).map(|s| s.as_str());
            dev::run(app)
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "Usage: cargo xtask <COMMAND>\n\n\
         Commands:\n  \
           check   Run all checks (fmt, clippy, deny, test, build, docs)\n  \
           fmt     Check formatting\n  \
           clippy  Run clippy lints\n  \
           deny    Run cargo-deny license/advisory checks\n  \
           test    Run all tests\n  \
           build   Build the workspace\n  \
           build-examples  Build all example integrations and apps\n  \
           build-wasm  Build WASM playground module\n  \
           docs    Build the documentation site\n  \
           dev <app>  Run example app in dev mode (server + client watch concurrently)"
    );
    ExitCode::FAILURE
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
        run: || run_command("cargo", &["fmt", "--all", "--check"], None),
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        run: || {
            run_command(
                "cargo",
                &["clippy", "--workspace", "--", "-D", "warnings"],
                None,
            )
        },
    };
    const DENY: Self = Self {
        name: "deny",
        run: || run_command("cargo", &["deny", "check"], None),
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
}

fn run_steps(steps: &[Step]) -> ExitCode {
    for step in steps {
        eprintln!("\n▸ {}", step.name);
        match (step.run)() {
            Ok(()) => eprintln!("  ✔ {}", step.name),
            Err(message) => {
                eprintln!("  ✘ {}", step.name);
                eprintln!("  ✘ {} — {}", step.name, message);
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!("\n✨ All checks passed\n");
    ExitCode::SUCCESS
}
