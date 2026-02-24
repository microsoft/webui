use std::process::{Command, ExitCode};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn main() -> ExitCode {
    let task = std::env::args().nth(1);

    match task.as_deref() {
        Some("check") => check(),
        Some("fmt") => run_steps(&[Step::FMT]),
        Some("clippy") => run_steps(&[Step::CLIPPY]),
        Some("deny") => run_steps(&[Step::DENY]),
        Some("test") => run_steps(&[Step::TEST]),
        Some("build") => run_steps(&[Step::BUILD, Step::BUILD_INTEGRATIONS, Step::BUILD_APPS]),
        Some("build-integrations") => run_steps(&[Step::BUILD_INTEGRATIONS]),
        Some("build-apps") => run_steps(&[Step::BUILD_APPS]),
        Some("docs") => run_steps(&[Step::DOCS]),
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

struct Step {
    name: &'static str,
    run: fn() -> Result<(), String>,
}

struct BuildCommand {
    cmd: &'static str,
    args: &'static [&'static str],
    cwd: Option<&'static str>,
}

struct IntegrationBuild {
    name: &'static str,
    commands: &'static [BuildCommand],
}

const INTEGRATION_HYPER_COMMANDS: &[BuildCommand] = &[BuildCommand {
    cmd: "cargo",
    args: &["build"],
    cwd: Some("examples/integration/hyper"),
}];

const INTEGRATION_TINY_HTTP_COMMANDS: &[BuildCommand] = &[BuildCommand {
    cmd: "cargo",
    args: &["build"],
    cwd: Some("examples/integration/tiny_http"),
}];

const INTEGRATION_BUILDS: &[IntegrationBuild] = &[
    IntegrationBuild {
        name: "hyper",
        commands: INTEGRATION_HYPER_COMMANDS,
    },
    IntegrationBuild {
        name: "tiny_http",
        commands: INTEGRATION_TINY_HTTP_COMMANDS,
    },
];

impl Step {
    const FMT: Self = Self {
        name: "fmt",
        run: run_fmt,
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        run: run_clippy,
    };
    const DENY: Self = Self {
        name: "deny",
        run: run_deny,
    };
    const TEST: Self = Self {
        name: "test",
        run: run_test,
    };
    const BUILD: Self = Self {
        name: "build",
        run: run_workspace_build,
    };
    const BUILD_INTEGRATIONS: Self = Self {
        name: "build (integrations)",
        run: run_integration_builds,
    };
    const BUILD_APPS: Self = Self {
        name: "build (apps via webui-cli)",
        run: run_app_builds,
    };
    const DOCS: Self = Self {
        name: "docs",
        run: run_docs,
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

fn run_fmt() -> Result<(), String> {
    run_command("cargo", &["fmt", "--all", "--check"], None)
}

fn run_clippy() -> Result<(), String> {
    run_command(
        "cargo",
        &["clippy", "--workspace", "--", "-D", "warnings"],
        None,
    )
}

fn run_deny() -> Result<(), String> {
    run_command("cargo", &["deny", "check"], None)
}

fn run_test() -> Result<(), String> {
    run_command("cargo", &["test", "--workspace"], None)
}

fn run_workspace_build() -> Result<(), String> {
    run_command("cargo", &["build", "--workspace"], None)
}

fn run_docs() -> Result<(), String> {
    run_command("pnpm", &["--filter", "@webui/docs", "build"], None)
}

fn run_integration_builds() -> Result<(), String> {
    if INTEGRATION_BUILDS.is_empty() {
        eprintln!("  • no integration build entries configured");
        return Ok(());
    }

    for integration in INTEGRATION_BUILDS {
        eprintln!("  • integration: {}", integration.name);
        for command in integration.commands {
            let cwd = command.cwd.map(Path::new);
            run_command(command.cmd, command.args, cwd).map_err(|message| {
                format!(
                    "integration '{}' command failed: {}",
                    integration.name, message
                )
            })?;
        }
    }

    Ok(())
}

fn run_app_builds() -> Result<(), String> {
    let apps_root = Path::new("examples/app");
    let app_dirs = collect_child_dirs(apps_root)?;

    if app_dirs.is_empty() {
        eprintln!("  • no example apps found under examples/app");
        return Ok(());
    }

    for app_dir in app_dirs {
        let app_name = display_name(&app_dir);
        let templates_dir = app_dir.join("templates");
        if !templates_dir.is_dir() {
            return Err(format!(
                "app '{}' is missing templates directory at {}",
                app_name,
                templates_dir.display()
            ));
        }

        let output_dir = PathBuf::from("target")
            .join("xtask")
            .join("app-builds")
            .join(app_name.as_str());

        eprintln!("  • app: {}", app_name);
        run_command(
            "cargo",
            &[
                "run",
                "-p",
                "webui-cli",
                "--",
                "build",
                templates_dir.to_string_lossy().as_ref(),
                "--out",
                output_dir.to_string_lossy().as_ref(),
            ],
            None,
        )
        .map_err(|message| format!("app '{}' build failed: {}", app_name, message))?;
    }

    Ok(())
}

fn run_command(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("exit code {}", status.code().unwrap_or(1))),
        Err(error) => Err(error.to_string()),
    }
}

fn collect_child_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| error.to_string())?;
    let mut dirs = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }

    dirs.sort();
    Ok(dirs)
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| path.display().to_string())
}
