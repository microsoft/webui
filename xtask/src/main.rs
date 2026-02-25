mod build_apps;
mod build_wasm;
mod util;

use std::process::ExitCode;
use util::run_command;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let task = args.get(1).map(|s| s.as_str());

    match task {
        Some("check") => check(),
        Some("fmt") => run_steps(&[Step::FMT]),
        Some("clippy") => run_steps(&[Step::CLIPPY]),
        Some("deny") => run_steps(&[Step::DENY]),
        Some("test") => run_steps(&[Step::TEST]),
        Some("build") => run_steps(&[Step::BUILD, Step::BUILD_INTEGRATIONS, Step::BUILD_APPS]),
        Some("build-integrations") => run_steps(&[Step::BUILD_INTEGRATIONS]),
        Some("build-apps") => run_steps(&[Step::BUILD_APPS]),
        Some("build-wasm") => run_steps(&[Step::BUILD_WASM]),
        Some("docs") => run_steps(&[Step::DOCS]),
        Some("run") => {
            let integration = args.get(2).map(|s| s.as_str());
            let app = args.get(3).map(|s| s.as_str());
            build_apps::run_integration_app(integration, app)
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
                     build-integrations  Build all examples/integration targets\n  \
                     build-apps  Build all examples/app templates through webui-cli\n  \
           build-wasm  Build WASM playground module\n  \
           run     Run an integration with an app\n  \
                     usage: cargo xtask run <integration> <app>\n  \
           docs    Build the documentation site"
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
        Step::BUILD_INTEGRATIONS,
        Step::BUILD_APPS,
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
        run: || run_command("cargo", &["build", "--workspace"], None),
    };
    const BUILD_INTEGRATIONS: Self = Self {
        name: "build (integrations)",
        run: build_apps::run_integration_builds,
    };
    const BUILD_APPS: Self = Self {
        name: "build (apps via webui-cli)",
        run: build_apps::run_app_builds,
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
