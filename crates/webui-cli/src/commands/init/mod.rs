// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod files;
mod onboarding;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::utils::output::{self, OutputFormat};

#[derive(Args)]
pub(crate) struct InitArgs {
    /// New or empty application directory (existing .git metadata is allowed)
    #[arg(default_value = ".")]
    pub(crate) path: PathBuf,

    /// Native rendering host for the generated application
    #[arg(long, value_enum, default_value_t = Server::Rust)]
    pub(crate) server: Server,

    /// Package manager for build tools, independent of the rendering host
    #[arg(long, value_enum, default_value_t = PackageManager::Npm)]
    pub(crate) package_manager: PackageManager,

    /// Install generated package dependencies after creating files
    #[arg(long)]
    pub(crate) install: bool,

    /// Run the existing project-local WebUI reference skill installer
    #[arg(long)]
    pub(crate) ai_skill: bool,

    /// Never prompt (also automatic without a terminal or with --format json)
    #[arg(long)]
    pub(crate) no_interactive: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Server {
    #[default]
    Rust,
    Node,
    Bun,
}

impl Server {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Bun => "bun",
        }
    }

    fn prerequisites(self) -> &'static str {
        match self {
            Self::Rust => "Node.js and Rust (Cargo)",
            Self::Node => "Node.js",
            Self::Bun => "Node.js (build tools) and Bun (server)",
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
}

impl InitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Destination(_) => "init-unsafe-destination",
            Self::Conflict(_) => "init-directory-not-empty",
            Self::Name(_) => "init-invalid-name",
            Self::Install(_) => "init-install-failed",
        }
    }

    pub(crate) fn help(&self) -> &'static str {
        match self {
            Self::Destination(_) => "Choose a directory with no symlink ancestors or '..' segments.",
            Self::Conflict(_) => "Choose a new or empty directory; existing files are never replaced.",
            Self::Name(_) => "Use a directory name with ASCII letters, numbers, spaces, '_' or '-'.",
            Self::Install(_) => "The generated app is preserved. Resolve the reported error and rerun the printed command from its directory.",
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Destination(_) | Self::Name(_) => 2,
            Self::Conflict(_) => 73,
            Self::Install(_) => 1,
        }
    }
}

#[derive(Serialize)]
struct InitReport {
    destination: PathBuf,
    server: Server,
    package_manager: PackageManager,
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
    let destination = files::destination(&args.path)?;
    let name = files::app_name(&destination)?;
    let generated = files::generate(&destination, &name, args)?;
    let manager = args.package_manager.name();
    let mut report = InitReport {
        destination,
        server: args.server,
        package_manager: args.package_manager,
        files: generated,
        dependencies: "skipped",
        ai_skill: "skipped",
        prerequisites: args.server.prerequisites(),
        next_commands: vec![
            format!("{manager} install"),
            format!("{manager} run dev"),
            format!("{manager} run build"),
            format!("{manager} run start"),
        ],
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
