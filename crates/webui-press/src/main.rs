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
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Monotonic counter that, combined with the process id, gives each staged
/// extraction a unique directory name so concurrent runs never collide.
static EXTRACT_NONCE: AtomicU64 = AtomicU64::new(0);

/// Materialize the embedded template + components into a per-version,
/// content-addressed cache directory and return the `template` subdirectory.
///
/// The cache is content-addressed (keyed by an FNV-1a hash of the embedded
/// bytes), so a `.complete` directory for a given hash is always valid and is
/// reused as-is. Extraction is concurrency-safe: assets are written into a
/// unique per-process staging directory and then published with a single
/// atomic `rename`. No process ever deletes or writes into a published cache
/// directory, so simultaneous `webui-press` invocations can never observe a
/// partially extracted cache or race a delete against another's writes.
fn extract_embedded_assets() -> Result<PathBuf> {
    let dir_name = format!(
        "webui-press-{}-{:016x}",
        env!("CARGO_PKG_VERSION"),
        embedded_assets_hash()
    );
    let tmp = std::env::temp_dir();
    let root = tmp.join(&dir_name);
    let template_dir = root.join("template");

    if is_complete_cache(&root) {
        return Ok(template_dir);
    }

    let unique = format!(
        "{}-{:x}",
        std::process::id(),
        EXTRACT_NONCE.fetch_add(1, Ordering::Relaxed)
    );
    let staging = tmp.join(format!("{dir_name}.staging-{unique}"));

    // Start from a clean staging dir, extract both trees, then mark complete.
    let _ = fs::remove_dir_all(&staging);
    EMBEDDED_TEMPLATE
        .extract(staging.join("template"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded template: {e}"))?;
    EMBEDDED_COMPONENTS
        .extract(staging.join("components"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded components: {e}"))?;
    fs::write(staging.join(".complete"), [])
        .map_err(|e| anyhow::anyhow!("Cannot finalize embedded template assets: {e}"))?;

    publish_cache(&staging, &root, &dir_name, &unique)?;
    Ok(template_dir)
}

/// A cache directory is usable only when fully extracted: the `.complete`
/// sentinel, the template entry point, and the sibling `components/` directory
/// (which `build_docs` discovers via `template_dir.parent()/components`) must
/// all be present. Validating `components/` here turns an externally
/// corrupted cache into a clean re-extraction instead of a confusing
/// missing-component build failure later.
fn is_complete_cache(root: &Path) -> bool {
    root.join(".complete").is_file()
        && root.join("template").join("index.html").is_file()
        && root.join("components").is_dir()
}

/// Publish a fully staged cache directory to its final, content-addressed
/// location with an atomic `rename`. If a peer process publishes the same
/// (identical, content-addressed) cache first, its copy is reused and the
/// local staging dir is discarded. A stale/incomplete `root` left by an older
/// build or external corruption is moved aside under a unique name (so
/// concurrent movers never collide) and replaced. The retry count is bounded
/// to guarantee termination.
fn publish_cache(staging: &Path, root: &Path, dir_name: &str, unique: &str) -> Result<()> {
    let tmp = std::env::temp_dir();
    for _ in 0..16 {
        match fs::rename(staging, root) {
            Ok(()) => return Ok(()),
            Err(_) if is_complete_cache(root) => {
                let _ = fs::remove_dir_all(staging);
                return Ok(());
            }
            Err(_) if root.exists() => {
                let stale = tmp.join(format!("{dir_name}.stale-{unique}"));
                if fs::rename(root, &stale).is_ok() {
                    let _ = fs::remove_dir_all(&stale);
                }
                // Retry the publish now that `root` should be free.
            }
            Err(e) => {
                let _ = fs::remove_dir_all(staging);
                return Err(anyhow::anyhow!(
                    "Cannot publish embedded template assets: {e}"
                ));
            }
        }
    }

    if is_complete_cache(root) {
        let _ = fs::remove_dir_all(staging);
        Ok(())
    } else {
        let _ = fs::remove_dir_all(staging);
        Err(anyhow::anyhow!(
            "Cannot publish embedded template assets after repeated attempts"
        ))
    }
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
        let root = template
            .parent()
            .ok_or_else(|| anyhow::anyhow!("template has no parent"))?;

        assert!(template.join("index.html").is_file());
        assert!(root.join("components/code-block/code-block.html").is_file());

        // The published cache must satisfy the completeness contract, and a
        // second call must reuse the same content-addressed directory.
        assert!(is_complete_cache(root));
        assert_eq!(extract_embedded_assets()?, template);
        Ok(())
    }

    #[test]
    fn incomplete_cache_is_not_treated_as_complete() -> Result<()> {
        let base = std::env::temp_dir().join(format!(
            "webui-press-test-incomplete-{}-{:x}",
            std::process::id(),
            EXTRACT_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let outcome: Result<()> = (|| {
            // `.complete` + template present, but no sibling components/ dir.
            fs::create_dir_all(base.join("template"))?;
            fs::write(base.join("template").join("index.html"), b"<html></html>")?;
            fs::write(base.join(".complete"), [])?;
            assert!(!is_complete_cache(&base));

            fs::create_dir_all(base.join("components"))?;
            assert!(is_complete_cache(&base));
            Ok(())
        })();
        let _ = fs::remove_dir_all(&base);
        outcome
    }
}
