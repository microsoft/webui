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
#[cfg(test)]
mod extraction_tests;
mod markdown;
mod regions;
mod serve;
mod state;
mod types;

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use console::style;
use include_dir::{include_dir, Dir, DirEntry};
use webui_dev_server::shutdown::{self, Control, Mode};

use crate::types::{DocsConfig, ShowMode};

static EMBEDDED_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/template");
static EMBEDDED_COMPONENTS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/components");
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

#[derive(Parser)]
#[command(
    name = "webui-press",
    version,
    about = "WebUI documentation site builder"
)]
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

        /// Generate the complete site or only page content (default: all)
        #[arg(long, value_enum)]
        show: Option<ShowMode>,
    },

    /// Build, watch sources, and serve with live reload (dev only)
    Serve(ServeArgs),
}

#[derive(Args)]
struct ServeArgs {
    /// Path to config.json
    #[arg(short, long, default_value = ".webui-press/config.json")]
    config: String,

    /// Path to the template directory (overrides bundled assets)
    #[arg(short, long)]
    template: Option<String>,

    /// Generate the complete site or only page content (default: all)
    #[arg(long, value_enum)]
    show: Option<ShowMode>,

    /// Port to bind
    #[arg(short, long, default_value_t = 3333)]
    port: u16,

    /// Host to bind
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Opt in to supervised shutdown with this grace period in seconds.
    /// A second stop request forces termination sooner.
    #[arg(long, value_name = "SECONDS")]
    shutdown_timeout: Option<NonZeroU64>,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Build {
            config,
            template,
            show,
        } => run_build(&config, template.as_deref(), show).map(|()| 0),
        Commands::Serve(args) => run_serve_command(&args),
    };

    match result {
        Ok(0) => {}
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("{} {e}", style("✘").red().bold());
            process::exit(1);
        }
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

/// Materialize the embedded template + components into a per-version,
/// content-addressed cache directory and return the `template` subdirectory.
///
/// The cache is content-addressed (keyed by an FNV-1a hash of the embedded
/// bytes), so a `.complete` directory for a given hash is always valid and is
/// reused as-is. A fresh extraction is written into a sibling staging directory
/// and published with a single atomic `rename`, so an interrupted run (Ctrl-C,
/// crash) never leaves a half-written cache: the next run sees no `.complete`
/// sentinel and re-extracts.
fn extract_embedded_assets() -> Result<PathBuf> {
    extract_embedded_assets_in(&std::env::temp_dir())
}

fn extract_embedded_assets_in(tmp: &Path) -> Result<PathBuf> {
    let dir_name = format!(
        "webui-press-{}-{:016x}",
        env!("CARGO_PKG_VERSION"),
        embedded_assets_hash()
    );
    let root = tmp.join(&dir_name);
    let template_dir = root.join("template");

    if is_complete_cache(&root) {
        return Ok(template_dir);
    }

    // Concurrent cold builds must not remove one another's staging directory.
    let cache_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(tmp.join(format!("{dir_name}.lock")))
        .map_err(|e| anyhow::anyhow!("Cannot open embedded asset cache lock: {e}"))?;
    #[cfg(test)]
    extraction_tests::checkpoint("cold")?;
    cache_lock
        .lock()
        .map_err(|e| anyhow::anyhow!("Cannot lock embedded asset cache: {e}"))?;
    if is_complete_cache(&root) {
        return Ok(template_dir);
    }

    // A `root` that isn't complete is a stale or interrupted extraction. Clear
    // it and any leftover staging dir, extract into staging, then publish.
    let staging = tmp.join(format!("{dir_name}.staging"));
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(staging.join("template"))
        .and_then(|()| fs::create_dir_all(staging.join("components")))
        .map_err(|e| anyhow::anyhow!("Cannot create embedded asset directories: {e}"))?;
    #[cfg(test)]
    extraction_tests::checkpoint("staged")?;
    EMBEDDED_TEMPLATE
        .extract(staging.join("template"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded template: {e}"))?;
    EMBEDDED_COMPONENTS
        .extract(staging.join("components"))
        .map_err(|e| anyhow::anyhow!("Cannot extract embedded components: {e}"))?;
    fs::write(staging.join(".complete"), [])
        .map_err(|e| anyhow::anyhow!("Cannot finalize embedded template assets: {e}"))?;

    // Atomic publish: the fully staged tree appears at `root` in one step.
    fs::rename(&staging, &root)
        .map_err(|e| anyhow::anyhow!("Cannot publish embedded template assets: {e}"))?;
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

fn run_build(config_path: &str, template_dir: Option<&str>, show: Option<ShowMode>) -> Result<()> {
    let (mut docs_config, config_dir, template) = load_config(config_path, template_dir)?;
    if let Some(show) = show {
        docs_config.show = show;
    }
    let _stats = build::build_docs(&docs_config, &config_dir, &template)?;
    Ok(())
}

fn run_serve_command(args: &ServeArgs) -> Result<i32> {
    // Gate before load_config can extract embedded assets or acquire cache locks.
    match shutdown::prepare(args.shutdown_timeout)? {
        Mode::Direct => run_serve_blocking(args, None).map(|()| 0),
        Mode::Child(control) => {
            run_serve_blocking(args, Some(control)).map(|()| shutdown::JOINED_EXIT_CODE)
        }
        Mode::Supervisor(code) => Ok(code),
    }
}

fn run_serve_blocking(args: &ServeArgs, control: Option<Control>) -> Result<()> {
    let (docs_config, config_dir, template) = load_config(&args.config, args.template.as_deref())?;
    let config_path_buf = Path::new(&args.config).to_path_buf();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow::anyhow!("Cannot start tokio runtime: {e}"))?;
    rt.block_on(serve::run_serve(
        serve::ServeConfig {
            config: docs_config,
            config_dir,
            template_dir: template,
            config_path: config_path_buf,
            host: args.host.clone(),
            port: args.port,
            show_override: args.show,
        },
        control,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_mode_is_typed_for_both_commands() -> Result<()> {
        for command in ["build", "serve"] {
            for (flag, expected) in [
                ("--show=all", ShowMode::All),
                ("--show=content", ShowMode::Content),
            ] {
                let cli = Cli::try_parse_from(["webui-press", command, flag])?;
                let show = match cli.command {
                    Commands::Build { show, .. } | Commands::Serve(ServeArgs { show, .. }) => show,
                };
                assert_eq!(show, Some(expected));
            }
            let cli = Cli::try_parse_from(["webui-press", command])?;
            let show = match cli.command {
                Commands::Build { show, .. } | Commands::Serve(ServeArgs { show, .. }) => show,
            };
            assert_eq!(show, None);
            assert!(Cli::try_parse_from(["webui-press", command, "--show=invalid"]).is_err());
        }
        Ok(())
    }

    #[test]
    fn shutdown_timeout_is_opt_in_and_positive() -> Result<()> {
        let Commands::Serve(args) = Cli::try_parse_from(["webui-press", "serve"])?.command else {
            panic!("expected serve command");
        };
        assert_eq!(args.shutdown_timeout, None);
        for value in ["1", "30"] {
            let Commands::Serve(args) =
                Cli::try_parse_from(["webui-press", "serve", "--shutdown-timeout", value])?.command
            else {
                panic!("expected serve command");
            };
            assert_eq!(
                args.shutdown_timeout.map(NonZeroU64::get),
                value.parse().ok()
            );
        }
        for value in ["0", "-1", "1.5", "invalid"] {
            assert!(
                Cli::try_parse_from(["webui-press", "serve", "--shutdown-timeout", value,])
                    .is_err()
            );
        }
        assert!(Cli::try_parse_from(["webui-press", "build", "--shutdown-timeout", "1"]).is_err());
        Ok(())
    }

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
            "webui-press-test-incomplete-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
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
