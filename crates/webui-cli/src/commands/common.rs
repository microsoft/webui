// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use clap::Args;
use std::path::PathBuf;
pub use webui::CssStrategy;
pub use webui::DomStrategy;
pub use webui::Plugin;

/// Shared CLI arguments used by both `build` and `serve` commands.
#[derive(Args, Clone)]
pub struct AppArgs {
    /// Path to the app folder (defaults to current directory)
    #[arg(default_value = ".")]
    pub app: PathBuf,

    /// Entry HTML file name (defaults to index.html)
    #[arg(long, default_value = "index.html")]
    pub entry: String,

    /// CSS delivery strategy for component stylesheets
    #[arg(long, value_enum, default_value_t = CssStrategy::Link)]
    pub css: CssStrategy,

    /// DOM strategy for component rendering (shadow or light)
    #[arg(long, value_enum, default_value_t = DomStrategy::Shadow)]
    pub dom: DomStrategy,

    /// Framework plugin to load
    #[arg(long, value_enum)]
    pub plugin: Option<Plugin>,

    /// Additional component sources (npm packages or local paths, repeatable)
    #[arg(long, value_name = "SOURCE")]
    pub components: Vec<String>,
}

impl AppArgs {
    /// Convert CLI arguments into library `BuildOptions`.
    pub fn to_build_options(&self, app_dir: &std::path::Path) -> webui::BuildOptions {
        webui::BuildOptions {
            app_dir: app_dir.to_path_buf(),
            entry: self.entry.clone(),
            css: self.css,
            dom: self.dom,
            plugin: self.plugin,
            components: self.components.clone(),
        }
    }
}
