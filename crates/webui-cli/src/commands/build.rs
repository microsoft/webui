use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use webui_parser::{CssStrategy, HtmlParser};
use webui_protocol::WebUIProtocol;

use crate::output::Printer;

/// CSS delivery strategy for component stylesheets.
#[derive(ValueEnum, Clone, Copy, Debug, Default)]
pub enum CssMode {
    /// Emit <link> tags referencing external .css files (default)
    #[default]
    External,
    /// Embed CSS inline in <style> tags within shadow DOM templates
    Inline,
}

impl From<CssMode> for CssStrategy {
    fn from(mode: CssMode) -> Self {
        match mode {
            CssMode::External => CssStrategy::External,
            CssMode::Inline => CssStrategy::Inline,
        }
    }
}

#[derive(Args)]
pub struct BuildArgs {
    /// Path to the app folder (defaults to current directory)
    #[arg(default_value = ".")]
    pub app: PathBuf,

    /// Output folder for the built protocol and assets
    #[arg(long)]
    pub out: PathBuf,

    /// Entry HTML file name (defaults to index.html)
    #[arg(long, default_value = "index.html")]
    pub entry: String,

    /// CSS delivery strategy for component stylesheets
    #[arg(long, value_enum, default_value_t = CssMode::External)]
    pub css: CssMode,
}

pub fn execute(args: &BuildArgs) -> Result<()> {
    run(args).map_err(|err| {
        let printer = Printer::new();
        printer.error(&err);

        let err_msg = format!("{:#}", err);
        if err_msg.contains("App folder not found") {
            printer.hint("Check that the app folder path exists");
        } else if err_msg.contains("Failed to read") && args.entry == "index.html" {
            printer.hint("Try using --entry <file> to specify a different entry file");
        }
        eprintln!();
        err
    })
}

fn run(args: &BuildArgs) -> Result<()> {
    let started = Instant::now();
    let printer = Printer::new();

    let app = args
        .app
        .canonicalize()
        .with_context(|| format!("App folder not found: {}", args.app.display()))?;

    printer.header("WebUI Build");
    printer.field("App", &app.display());
    printer.field("Entry", &args.entry);
    printer.field("Output", &args.out.display());
    printer.field("CSS", &format!("{:?}", args.css));
    eprintln!();

    // Create output directory
    fs::create_dir_all(&args.out)
        .with_context(|| format!("Failed to create output dir: {}", args.out.display()))?;

    // Set up parser and register components from the app directory
    let mut parser = HtmlParser::new();
    parser.set_css_strategy(args.css.into());
    parser
        .component_registry_mut()
        .register_from_paths(&[&app])
        .context("Failed to register components")?;

    let component_count = parser.component_registry_mut().len();
    printer.success(&format!(
        "Registered {} component{}",
        printer.bold.apply_to(component_count),
        if component_count == 1 { "" } else { "s" }
    ));

    // Read and parse the entry HTML file
    let entry_path = app.join(&args.entry);
    let html_content = fs::read_to_string(&entry_path)
        .with_context(|| format!("Failed to read {}", entry_path.display()))?;

    parser
        .parse(&args.entry, &html_content)
        .context("Failed to parse HTML")?;

    // Collect component CSS files before consuming parser
    let css_files: Vec<(String, String)> = parser
        .component_registry_mut()
        .get_all()
        .filter_map(|c| {
            c.css_content
                .as_ref()
                .map(|css| (format!("{}.css", c.tag_name), css.clone()))
        })
        .collect();

    // Build the protocol
    let fragment_records = parser.into_fragment_records();
    let fragment_count: usize = fragment_records.values().map(|v| v.fragments.len()).sum();
    let protocol = WebUIProtocol {
        fragments: fragment_records,
    };

    printer.success(&format!(
        "Parsed {} ({} fragment{})",
        printer.bold.apply_to(&args.entry),
        printer.bold.apply_to(fragment_count),
        if fragment_count == 1 { "" } else { "s" }
    ));

    // Write protocol as optimized protobuf binary
    let bytes = protocol
        .to_protobuf()
        .context("Failed to serialize protocol")?;
    let protocol_path = args.out.join("protocol.bin");
    fs::write(&protocol_path, &bytes)
        .with_context(|| format!("Failed to write {}", protocol_path.display()))?;
    printer.success(&format!("Wrote {}", printer.bold.apply_to("protocol.bin")));

    let mut files_written: usize = 1;

    // Copy component CSS files (only in external mode)
    if matches!(args.css, CssMode::External) {
        for (filename, css_content) in &css_files {
            let css_path = args.out.join(filename);
            fs::write(&css_path, css_content)
                .with_context(|| format!("Failed to write {}", css_path.display()))?;
            printer.success(&format!("Wrote {}", printer.bold.apply_to(filename)));
            files_written += 1;
        }
    }

    let elapsed = started.elapsed();
    printer.finish(&format!(
        "Build complete ({} file{} written) {}",
        printer.bold.apply_to(files_written),
        if files_written == 1 { "" } else { "s" },
        printer.dim.apply_to(format!("in {elapsed:.0?}")),
    ));

    Ok(())
}

/// Standalone build function for testing without CLI args.
#[cfg(test)]
pub fn build(app: &std::path::Path, out: &std::path::Path, entry: &str) -> Result<()> {
    run(&BuildArgs {
        app: app.to_path_buf(),
        out: out.to_path_buf(),
        entry: entry.to_string(),
        css: CssMode::External,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "build_tests.rs"]
mod build_tests;
