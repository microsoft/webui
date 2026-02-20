use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let task = std::env::args().nth(1);

    match task.as_deref() {
        Some("check") => check(),
        Some("fmt") => run_steps(&[Step::FMT]),
        Some("clippy") => run_steps(&[Step::CLIPPY]),
        Some("deny") => run_steps(&[Step::DENY]),
        Some("test") => run_steps(&[Step::TEST]),
        Some("build") => run_steps(&[Step::BUILD]),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "Usage: cargo xtask <COMMAND>\n\n\
         Commands:\n  \
           check   Run all checks (fmt, clippy, deny, test, build)\n  \
           fmt     Check formatting\n  \
           clippy  Run clippy lints\n  \
           deny    Run cargo-deny license/advisory checks\n  \
           test    Run all tests\n  \
           build   Build the workspace"
    );
    ExitCode::FAILURE
}

fn check() -> ExitCode {
    run_steps(&[Step::FMT, Step::CLIPPY, Step::DENY, Step::TEST, Step::BUILD])
}

struct Step {
    name: &'static str,
    cmd: &'static str,
    args: &'static [&'static str],
}

impl Step {
    const FMT: Self = Self {
        name: "fmt",
        cmd: "cargo",
        args: &["fmt", "--all", "--check"],
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        cmd: "cargo",
        args: &["clippy", "--workspace", "--", "-D", "warnings"],
    };
    const DENY: Self = Self {
        name: "deny",
        cmd: "cargo",
        args: &["deny", "check"],
    };
    const TEST: Self = Self {
        name: "test",
        cmd: "cargo",
        args: &["test", "--workspace"],
    };
    const BUILD: Self = Self {
        name: "build",
        cmd: "cargo",
        args: &["build", "--workspace"],
    };
}

fn run_steps(steps: &[Step]) -> ExitCode {
    for step in steps {
        eprintln!("\n▸ {}", step.name);
        let status = Command::new(step.cmd).args(step.args).status();

        match status {
            Ok(s) if s.success() => eprintln!("  ✔ {}", step.name),
            Ok(s) => {
                eprintln!("  ✘ {}", step.name);
                return ExitCode::from(s.code().unwrap_or(1) as u8);
            }
            Err(e) => {
                eprintln!("  ✘ {} — {}", step.name, e);
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!("\n✨ All checks passed\n");
    ExitCode::SUCCESS
}
