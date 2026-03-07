use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use std::fs;
use std::path::{Path, PathBuf};
use webui_parser::plugin::FastParserPlugin;
use webui_parser::{CssStrategy, HtmlParser};
use webui_protocol::WebUIProtocol;

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

/// Shared CLI arguments used by both `build` and `start` commands.
#[derive(Args, Clone)]
pub struct AppArgs {
    /// Path to the app folder (defaults to current directory)
    #[arg(default_value = ".")]
    pub app: PathBuf,

    /// Entry HTML file name (defaults to index.html)
    #[arg(long, default_value = "index.html")]
    pub entry: String,

    /// CSS delivery strategy for component stylesheets
    #[arg(long, value_enum, default_value_t = CssMode::External)]
    pub css: CssMode,

    /// Parser plugin to load (e.g., "fast" for FAST-HTML hydration support)
    #[arg(long)]
    pub plugin: Option<String>,

    /// Additional component sources (npm packages or local paths, repeatable)
    #[arg(long, value_name = "SOURCE")]
    pub components: Vec<String>,
}

/// Result of building the protocol from app templates.
pub struct BuildOutput {
    /// The compiled WebUI protocol
    pub protocol: WebUIProtocol,
    /// Component CSS files: (filename, content) — only components used in the protocol
    pub css_files: Vec<(String, String)>,
    /// Total fragment count
    pub fragment_count: usize,
    /// Number of registered components
    pub component_count: usize,
    /// Number of unique CSS tokens discovered
    pub token_count: usize,
}

/// Build the protocol from an app directory.
///
/// This is the shared core: sets up parser, registers components (app + external),
/// parses entry HTML, collects CSS for used components, and returns the protocol.
pub fn build_protocol(app_dir: &Path, args: &AppArgs) -> Result<BuildOutput> {
    // Set up parser with plugin
    let mut parser = match args.plugin.as_deref() {
        Some("fast") => HtmlParser::with_plugin(Box::new(FastParserPlugin::new())),
        Some(unknown) => anyhow::bail!("Unknown plugin: {unknown}"),
        None => HtmlParser::new(),
    };
    parser.set_css_strategy(args.css.into());

    // Register app directory components
    parser
        .component_registry_mut()
        .register_from_paths(&[app_dir])
        .context("Failed to register components")?;

    // Discover and register --components sources
    for source in &args.components {
        let result = webui_discovery::discover_source(source, app_dir)
            .with_context(|| format!("Failed to discover components from {source}"))?;
        for comp in &result.components {
            parser
                .component_registry_mut()
                .register_component(
                    &comp.tag_name,
                    &comp.html_content,
                    comp.css_content.as_deref(),
                )
                .with_context(|| {
                    format!(
                        "Failed to register component '{}' from {}",
                        comp.tag_name, comp.source
                    )
                })?;
        }
    }

    let component_count = parser.component_registry_mut().len();

    // Parse entry HTML
    let entry_path = app_dir.join(&args.entry);
    let html_content = fs::read_to_string(&entry_path)
        .with_context(|| format!("Failed to read {}", entry_path.display()))?;
    parser
        .parse(&args.entry, &html_content)
        .context("Failed to parse HTML")?;

    // Snapshot CSS for components that have it before consuming the parser
    let css_snapshot: Vec<(String, String)> = parser
        .component_registry_mut()
        .get_all()
        .filter_map(|c| {
            c.css_content
                .as_ref()
                .map(|css| (c.tag_name.clone(), css.clone()))
        })
        .collect();

    // Collect CSS tokens before consuming the parser
    let tokens = parser.take_tokens();
    let token_count = tokens.len();

    // Build protocol (consumes parser)
    let fragment_records = parser.into_fragment_records();
    let fragment_count: usize = fragment_records.values().map(|v| v.fragments.len()).sum();

    // Filter CSS to only protocol-referenced components
    let css_files: Vec<(String, String)> = css_snapshot
        .into_iter()
        .filter(|(tag, _)| fragment_records.contains_key(tag))
        .map(|(tag, css)| (format!("{tag}.css"), css))
        .collect();

    let protocol = WebUIProtocol::with_tokens(fragment_records, tokens);

    Ok(BuildOutput {
        protocol,
        css_files,
        fragment_count,
        component_count,
        token_count,
    })
}
