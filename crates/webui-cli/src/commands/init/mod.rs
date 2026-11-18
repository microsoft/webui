// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod files;
mod native;
mod onboarding;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::utils::output::{self, OutputFormat};

#[derive(Args)]
pub(crate) struct InitArgs {
    /// New or empty application directory (existing .git metadata is allowed)
    #[arg(default_value = ".")]
    pub(crate) path: PathBuf,

    /// Native rendering host for the generated application
    #[arg(long, value_enum, default_value_t = Server::Rust)]
    pub(crate) server: Server,

    /// Package manager for Node/Bun server runtime dependencies
    #[arg(long, value_enum)]
    pub(crate) package_manager: Option<PackageManager>,

    /// Restore the selected host's dependencies after creating files
    #[arg(long)]
    pub(crate) install: bool,

    /// Install the version-matched offline WebUI reference skill loader
    #[arg(long)]
    pub(crate) ai_skill: bool,

    /// Never prompt (also automatic without a terminal or with --format json)
    #[arg(long)]
    pub(crate) no_interactive: bool,
}

impl InitArgs {
    pub(super) fn manager(&self) -> PackageManager {
        self.package_manager
            .unwrap_or(if self.server == Server::Bun {
                PackageManager::Bun
            } else {
                PackageManager::Npm
            })
    }

    pub(super) fn uses_packages(&self) -> bool {
        matches!(self.server, Server::Node | Server::Bun)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Server {
    #[default]
    Rust,
    Dotnet,
    C,
    Cpp,
    Node,
    Bun,
    Go,
}

impl Server {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Dotnet => "dotnet",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Node => "node",
            Self::Bun => "bun",
            Self::Go => "go",
        }
    }

    fn prerequisites(self) -> &'static str {
        match self {
            Self::Rust => "WebUI native tooling and Rust (Cargo)",
            Self::Go => "WebUI native tooling, Go, a cgo C compiler, and the matching standalone native SDK",
            Self::C | Self::Cpp => "WebUI native tooling, CMake, a C/C++ toolchain, and the matching standalone native SDK",
            Self::Dotnet => "WebUI native tooling, .NET SDK, and an approved matching Microsoft.WebUI package source",
            Self::Node => "WebUI native tooling and Node.js (server runtime)",
            Self::Bun => "WebUI native tooling and Bun with Node-API support (server runtime)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PackageManager {
    #[default]
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InitError {
    #[error("unsafe application destination: {0}")]
    Destination(String),
    #[error("application directory is not empty: {0}")]
    Conflict(String),
    #[error("invalid application name: {0}")]
    Name(String),
    #[error("installation failed: {0}")]
    Install(String),
    #[error("invalid initializer options: {0}")]
    Options(String),
}

impl InitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Destination(_) => "init-unsafe-destination",
            Self::Conflict(_) => "init-directory-not-empty",
            Self::Name(_) => "init-invalid-name",
            Self::Install(_) => "init-install-failed",
            Self::Options(_) => "init-invalid-options",
        }
    }

    pub(crate) fn help(&self) -> &'static str {
        match self {
            Self::Destination(_) => "Choose a non-root directory with no regular-file or symlink path components and no '..' segments.",
            Self::Conflict(_) => "Choose a new or empty directory; existing files are never replaced.",
            Self::Name(_) => "Use ASCII letters, numbers, spaces, '_' or '-'; normalized names (lowercase with '-' separators) must start with a letter, contain at most 64 characters, and not be reserved (con, prn, aux, nul, com1-com9, lpt1-lpt9, node-modules).",
            Self::Install(_) => "The generated app is preserved. Resolve the reported error and rerun the printed command from its directory.",
            Self::Options(_) => "Package-manager selection requires a Node/Bun server. Rust/Go/C/C++/.NET projects use their host toolchains and contain no package.json.",
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Destination(_) | Self::Name(_) | Self::Options(_) => 2,
            Self::Conflict(_) => 73,
            Self::Install(_) => 1,
        }
    }
}

#[derive(Serialize)]
struct InitReport {
    destination: PathBuf,
    server: Server,
    package_manager: Option<PackageManager>,
    files: Vec<String>,
    dependencies: &'static str,
    ai_skill: &'static str,
    prerequisites: &'static str,
    next_commands: Vec<String>,
}

pub(crate) fn execute(args: &InitArgs) -> Result<()> {
    let result = run(args);
    if let Err(error) = &result {
        output::error(error);
    }
    result
}

fn run(args: &InitArgs) -> Result<()> {
    if args.package_manager.is_some() && !args.uses_packages() {
        return Err(InitError::Options(
            "this native host has no JavaScript package dependencies".into(),
        )
        .into());
    }
    let destination = files::destination(&args.path)?;
    let name = files::app_name(&destination)?;
    let generated = files::generate(&destination, &name, args)?;
    let manager = args.manager().name();
    let mut next_commands: Vec<_> = ["check", "dev", "build", "start"]
        .iter()
        .map(|action| format!("webui app {action}"))
        .collect();
    if args.uses_packages() {
        next_commands.insert(0, format!("{manager} install"));
    }
    let mut report = InitReport {
        destination,
        server: args.server,
        package_manager: args.uses_packages().then(|| args.manager()),
        files: generated,
        dependencies: "skipped",
        ai_skill: "skipped",
        prerequisites: args.server.prerequisites(),
        next_commands,
    };
    output::header("WebUI Init");
    output::field("Directory", &report.destination.display());
    output::field("Server", &args.server.name());
    output::field("Requires", &report.prerequisites);
    let result = onboarding::run(args, &mut report);
    if result.is_ok() {
        output::success("Application created");
    }
    if matches!(output::format(), OutputFormat::Json) {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        output::field("Next", &format!("In {}:", report.destination.display()));
        for command in &report.next_commands {
            output::field("Run", command);
        }
    }
    result
}
