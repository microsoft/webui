// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `webui-press` CLI — standalone documentation site builder.
//!
//! ```bash
//! webui-press build                          # defaults: .webui-press/config.json
//! webui-press build --config my-config.json  # custom config
//! webui-press build --template ./my-template # custom template
//! webui-press serve                          # build + watch + live-reload dev server
//! webui-press serve --port 4000              # custom port
//! ```

mod build;
mod content;
mod error;
mod markdown;
mod serve;
mod types;

use std::fs;
use std::path::{Path, PathBuf};
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

    /// Build, watch sources, and serve with live reload (dev only)
    Serve {
        /// Path to config.json
        #[arg(short, long, default_value = ".webui-press/config.json")]
        config: String,

        /// Path to the template directory (overrides built-in)
        #[arg(short, long)]
        template: Option<String>,

        /// Port to bind
        #[arg(short, long, default_value_t = 3333)]
        port: u16,

        /// Host to bind
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { config, template } => run_build(&config, template.as_deref()),
        Commands::Serve {
            config,
            template,
            port,
            host,
        } => run_serve_blocking(&config, template.as_deref(), &host, port),
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("✘").red().bold(), e);
        process::exit(1);
    }
}

/// Resolve config + template directory + parsed config from CLI args.
/// Shared by `build` and `serve`.
fn load_config(
    config_path: &str,
    template_dir: Option<&str>,
) -> Result<(DocsConfig, PathBuf, PathBuf)> {
    let config_str = fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("Cannot read config {}: {}", style(config_path).bold(), e))?;

    let docs_config: DocsConfig = serde_json::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Invalid config JSON: {}", e))?;

    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let template = match template_dir {
        Some(t) => Path::new(t).to_path_buf(),
        None => {
            let exe = std::env::current_exe().unwrap_or_default();
            let exe_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();

            exe_dir
                .ancestors()
                .find_map(|dir| {
                    let t = dir.join("crates/webui-press/template");
                    if t.join("index.html").exists() {
                        Some(t)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| exe_dir.join("template"))
        }
    };

    Ok((docs_config, config_dir, template))
}

fn run_build(config_path: &str, template_dir: Option<&str>) -> Result<()> {
    let (docs_config, config_dir, template) = load_config(config_path, template_dir)?;
    let _stats = build::build_docs(&docs_config, &config_dir, &template)?;
    Ok(())
}

fn run_serve_blocking(
    config_path: &str,
    template_dir: Option<&str>,
    host: &str,
    port: u16,
) -> Result<()> {
    let (docs_config, config_dir, template) = load_config(config_path, template_dir)?;
    let config_path_buf = Path::new(config_path).to_path_buf();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow::anyhow!("Cannot start tokio runtime: {e}"))?;
    rt.block_on(serve::run_serve(serve::ServeConfig {
        config: docs_config,
        config_dir,
        template_dir: template,
        config_path: config_path_buf,
        host: host.to_string(),
        port,
    }))
}
