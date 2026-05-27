// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use clap::Args;
use std::path::PathBuf;
pub use webui::CssStrategy;
pub use webui::DomStrategy;
pub use webui::Plugin;
pub use webui::DEFAULT_CSS_FILE_NAME_TEMPLATE;

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

    /// Link-mode CSS filename template using [name], [hash], [ext]
    #[arg(long, default_value = DEFAULT_CSS_FILE_NAME_TEMPLATE)]
    pub css_file_name_template: String,

    /// Optional base URL/path prefix for Link-mode css hrefs
    #[arg(long)]
    pub css_public_base: Option<String>,
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
            css_file_name_template: self.css_file_name_template.clone(),
            css_public_base: self.css_public_base.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_build_options_passes_css_file_output_settings() {
        let args = AppArgs {
            app: std::path::PathBuf::from("."),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
            css_file_name_template: "[name]-[hash].[ext]".to_string(),
            css_public_base: Some("https://cdn.example.com/assets".to_string()),
        };
        let options = args.to_build_options(std::path::Path::new("."));

        assert_eq!(options.css_file_name_template, "[name]-[hash].[ext]");
        assert_eq!(
            options.css_public_base.as_deref(),
            Some("https://cdn.example.com/assets")
        );
    }
}
