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
mod bundler;
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
use include_dir::{include_dir, Dir, DirEntry};

use crate::types::DocsConfig;

static EMBEDDED_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/template");
static EMBEDDED_COMPONENTS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/components");
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

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

        /// Path to the template directory (overrides bundled assets)
        #[arg(short, long)]
        template: Option<String>,
    },

    /// Build, watch sources, and serve with live reload (dev only)
    Serve {
        /// Path to config.json
        #[arg(short, long, default_value = ".webui-press/config.json")]
        config: String,

        /// Path to the template directory (overrides bundled assets)
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

/// Load the config and materialize the embedded template assets.
/// Shared by `build` and `serve`.
fn load_config(
    config_path: &str,
    template_dir: Option<&str>,
) -> Result<(DocsConfig, PathBuf, PathBuf)> {
    let config_str = fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("Cannot read config {}: {}", style(config_path).bold(), e))?;

    let docs_config: DocsConfig = serde_json::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Invalid config JSON: {e}"))?;

    let config_dir = Path::new(config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let template = match template_dir {
        Some(template_dir) => Path::new(template_dir).to_path_buf(),
        None => extract_embedded_assets()?,
    };

    Ok((docs_config, config_dir, template))
}

fn extract_embedded_assets() -> Result<PathBuf> {
    let hash = embedded_assets_hash();
    let root = std::env::temp_dir().join(format!(
        "webui-press-{}-{hash:016x}",
        env!("CARGO_PKG_VERSION"),
    ));

    let template_dir = root.join("template");
    if root.join(".complete").is_file() && template_dir.join("index.html").is_file() {
        return Ok(template_dir);
    }

    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|e| anyhow::anyhow!("Cannot refresh embedded template assets: {e}"))?;
    }
    EMBEDDED_TEMPLATE
        .extract(root.join("template"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded template: {e}"))?;
    EMBEDDED_COMPONENTS
        .extract(root.join("components"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded components: {e}"))?;
    fs::write(root.join(".complete"), [])
        .map_err(|e| anyhow::anyhow!("Cannot finalize embedded template assets: {e}"))?;
    Ok(template_dir)
}

fn embedded_assets_hash() -> u64 {
    let mut hash = FNV_OFFSET;
    hash = hash_dir(hash, &EMBEDDED_TEMPLATE);
    hash_dir(hash, &EMBEDDED_COMPONENTS)
}

fn hash_dir(mut hash: u64, dir: &Dir<'_>) -> u64 {
    for entry in dir.entries() {
        hash = hash_bytes(hash, entry.path().to_string_lossy().as_bytes());
        match entry {
            DirEntry::Dir(dir) => hash = hash_dir(hash, dir),
            DirEntry::File(file) => hash = hash_bytes(hash, file.contents()),
        }
    }
    hash
}

fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_extract_template_and_components() -> Result<()> {
        let template = extract_embedded_assets()?;
        assert!(template.join("index.html").is_file());
        assert!(template
            .parent()
            .ok_or_else(|| anyhow::anyhow!("template has no parent"))?
            .join("components/code-block/code-block.html")
            .is_file());
        Ok(())
    }
}
