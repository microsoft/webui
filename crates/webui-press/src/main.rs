// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `webui-press` CLI — standalone documentation site builder.
//!
//! ```bash
//! webui-press build                          # defaults: .webui-press/config.json
//! webui-press build --config my-config.json  # custom config
//! webui-press build --template ./my-template # custom template
//! ```

mod build;
mod content;
mod error;
mod markdown;
mod types;

use std::fs;
use std::path::Path;
use std::process;

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::types::DocsConfig;

#[derive(Parser)]
#[command(name = "webui-press", about = "WebUI documentation site builder")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the documentation site
    Build {
        /// Path to config.json
        #[arg(short, long, default_value = ".webui-press/config.json")]
        config: String,

        /// Path to the template directory (overrides built-in)
        #[arg(short, long)]
        template: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { config, template } => run_build(&config, template.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("✘").red().bold(), e);
        process::exit(1);
    }
}

fn run_build(config_path: &str, template_dir: Option<&str>) -> Result<()> {
    let config_str = fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("Cannot read config {}: {}", style(config_path).bold(), e))?;

    let docs_config: DocsConfig = serde_json::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Invalid config JSON: {}", e))?;

    // Directory containing config.json — used to resolve relative paths
    // declared inside the config (e.g. a custom page's `state_file`).
    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    // Resolve template directory: explicit flag > crate-bundled template
    let template = match template_dir {
        Some(t) => Path::new(t).to_path_buf(),
        None => {
            // Look for template/ alongside the binary (crate ships it)
            let exe = std::env::current_exe().unwrap_or_default();
            let exe_dir = exe.parent().unwrap_or(Path::new("."));

            // Workspace dev: crates/webui-press/template/
            let crate_template = exe_dir.ancestors().find_map(|dir| {
                let t = dir.join("crates/webui-press/template");
                if t.join("index.html").exists() {
                    Some(t)
                } else {
                    None
                }
            });

            crate_template.unwrap_or_else(|| exe_dir.join("template"))
        }
    };

    let _stats = build::build_docs(&docs_config, &config_dir, &template)?;

    Ok(())
}
