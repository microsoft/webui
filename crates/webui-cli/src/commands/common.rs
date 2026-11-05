// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
pub use webui::CssStrategy;
pub use webui::DomStrategy;
pub use webui::LegalComments;
pub use webui::Plugin;
pub use webui::DEFAULT_ASSET_FILE_NAME_TEMPLATE;

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

    /// Fallback DOM strategy for components without an authored Shadow root
    #[arg(long, value_enum, default_value_t = DomStrategy::Shadow)]
    pub dom: DomStrategy,

    /// Merge component stylesheets into shared bundled chunks
    ///
    /// Composes with `--css`: bundling decides how stylesheets are grouped,
    /// `--css` decides how they reach the page. Stylesheets reached from more
    /// than one CSS tree are split into their own chunk so they are downloaded
    /// and cached once. Not supported with `--css module`, which already inlines
    /// every stylesheet as a data URI.
    #[arg(long)]
    pub css_bundle: bool,

    /// Framework plugin to load
    #[arg(long, value_enum)]
    pub plugin: Option<Plugin>,

    /// Additional component sources (npm packages or local paths, repeatable)
    #[arg(long, value_name = "SOURCE")]
    pub components: Vec<String>,

    /// Bundler projection manifest fragment (repeatable)
    #[arg(long = "projection-manifest", value_name = "PATH")]
    pub projection_manifests: Vec<PathBuf>,

    /// Emitted asset filename template using [name], [hash], [ext]
    #[arg(long, default_value = DEFAULT_ASSET_FILE_NAME_TEMPLATE)]
    pub asset_file_name_template: String,

    /// Public URL or path ending in '/' that maps to the output directory.
    /// Rendered stylesheet links are prefixed with it, and the browser resolves
    /// stylesheet and lazy pack paths against it. Module preload hints keep
    /// their paths. Requests can override it with the `$webui.assetBase` state
    /// member.
    #[arg(long, value_name = "BASE")]
    pub asset_base: Option<String>,

    /// Module that defines a custom element, for components without a usable
    /// `.ts` or `.js` sibling (repeatable). The generated lazy.webui.js imports
    /// SPECIFIER verbatim when a lazy region introduces TAG, overriding a
    /// discovered sibling module.
    #[arg(
        long = "component-module",
        value_name = "TAG=SPECIFIER",
        value_parser = parse_component_module
    )]
    pub component_modules: Vec<(String, String)>,

    /// Legal comment handling: inline preserves legal CSS comments, none strips all comments
    #[arg(long, value_enum, default_value_t = LegalComments::Inline)]
    pub legal_comments: LegalComments,
}

/// Parse and validate one `--component-module TAG=SPECIFIER` value.
fn parse_component_module(value: &str) -> std::result::Result<(String, String), String> {
    let Some((tag, specifier)) = value.split_once('=') else {
        return Err(format!(
            "expected TAG=SPECIFIER, for example settings-panel=@contoso/ui/settings-panel.js, but got '{value}'"
        ));
    };
    webui::validate_component_module(tag, specifier).map_err(|diagnostic| diagnostic.body())?;
    Ok((tag.to_string(), specifier.to_string()))
}

impl AppArgs {
    /// Convert CLI arguments into library `BuildOptions`.
    ///
    /// # Errors
    ///
    /// Returns an error when `--component-module` maps one tag to two modules.
    pub fn to_build_options(&self, app_dir: &std::path::Path) -> Result<webui::BuildOptions> {
        Ok(webui::BuildOptions {
            app_dir: app_dir.to_path_buf(),
            entry: self.entry.clone(),
            css: self.css,
            dom: self.dom,
            css_bundle: self.css_bundle,
            plugin: self.plugin,
            components: self.components.clone(),
            component_asset_roots: Vec::new(),
            metafile: false,
            css_file_name_template: self.asset_file_name_template.clone(),
            asset_base: self.asset_base.clone(),
            component_modules: self.component_module_map()?,
            modules_out: None,
            legal_comments: self.legal_comments,
            theme: None,
            projection_manifests: self
                .projection_manifests
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
        })
    }

    fn component_module_map(&self) -> Result<BTreeMap<String, String>> {
        let mut modules = BTreeMap::new();
        for (tag, specifier) in &self.component_modules {
            if let Some(previous) = modules.insert(tag.clone(), specifier.clone()) {
                if previous != *specifier {
                    anyhow::bail!(
                        "--component-module maps <{tag}> to both {previous} and {specifier}.\nhelp: Pass one --component-module per tag."
                    );
                }
            }
        }
        Ok(modules)
    }
}

/// Load and resolve a theme file from a CLI `--theme` value.
pub fn load_theme(theme: &str, search_root: &Path) -> Result<webui::TokenFile> {
    let resolved = webui::resolve_theme_path(theme, search_root)
        .with_context(|| format!("Failed to resolve theme: {theme}"))?;
    webui::load_token_file(&resolved)
        .with_context(|| format!("Failed to load theme file: {}", resolved.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        app: AppArgs,
    }

    #[test]
    fn dom_option_defaults_to_shadow_and_accepts_light() {
        assert!(TestArgs::try_parse_from(["test"]).is_ok());
        assert_eq!(
            TestArgs::try_parse_from(["test"])
                .expect("default arguments")
                .app
                .dom,
            DomStrategy::Shadow
        );
        assert_eq!(
            TestArgs::try_parse_from(["test", "--dom=light"])
                .expect("Light DOM arguments")
                .app
                .dom,
            DomStrategy::Light
        );
        assert!(TestArgs::try_parse_from(["test", "--dom=invalid"]).is_err());
        assert!(TestArgs::command()
            .render_long_help()
            .to_string()
            .contains("--dom"));
    }

    #[test]
    fn asset_base_flag_sets_the_build_default() {
        let args = TestArgs::try_parse_from(["test", "--asset-base", "/static/"])
            .expect("asset base arguments");
        assert_eq!(args.app.asset_base.as_deref(), Some("/static/"));
        assert_eq!(
            TestArgs::try_parse_from(["test"])
                .expect("default arguments")
                .app
                .asset_base,
            None
        );
        assert!(TestArgs::command()
            .render_long_help()
            .to_string()
            .contains("--asset-base <BASE>"));
    }

    #[test]
    fn component_module_flag_maps_tags_to_module_specifiers() {
        let args = TestArgs::try_parse_from([
            "test",
            "--component-module",
            "settings-panel=@contoso/ui/settings-panel.js",
            "--component-module=theme-picker=../vendor/picker.js?raw=1",
        ])
        .expect("component module arguments");
        let options = args
            .app
            .to_build_options(std::path::Path::new("."))
            .expect("build options");
        assert_eq!(
            options.component_modules,
            BTreeMap::from([
                (
                    "settings-panel".to_string(),
                    "@contoso/ui/settings-panel.js".to_string()
                ),
                (
                    "theme-picker".to_string(),
                    "../vendor/picker.js?raw=1".to_string()
                ),
            ])
        );
        assert!(options.modules_out.is_none());
        assert!(TestArgs::command()
            .render_long_help()
            .to_string()
            .contains("--component-module <TAG=SPECIFIER>"));

        for invalid in [
            "settings-panel",
            "SettingsPanel=./panel.js",
            "settings-panel=",
            "settings-panel=./my panel.js",
            "settings-panel=.\\panel.js",
        ] {
            let error = TestArgs::try_parse_from(["test", "--component-module", invalid])
                .err()
                .expect("invalid component module");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{invalid}"
            );
        }

        let same = TestArgs::try_parse_from([
            "test",
            "--component-module",
            "x-card=./a.js",
            "--component-module",
            "x-card=./a.js",
        ])
        .expect("repeated mapping");
        assert!(same.app.to_build_options(Path::new(".")).is_ok());
        let conflicting = TestArgs::try_parse_from([
            "test",
            "--component-module",
            "x-card=./a.js",
            "--component-module",
            "x-card=./b.js",
        ])
        .expect("conflicting mapping parses");
        let error = conflicting
            .app
            .to_build_options(Path::new("."))
            .expect_err("conflicting mappings are rejected");
        assert!(error.to_string().contains("<x-card>"), "{error}");
    }

    #[test]
    fn to_build_options_passes_asset_file_output_settings() {
        let args = AppArgs {
            app: std::path::PathBuf::from("."),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            css_bundle: false,
            plugin: None,
            components: Vec::new(),
            projection_manifests: vec![
                std::path::PathBuf::from("app-projection.json"),
                std::path::PathBuf::from("shared-projection.json"),
            ],
            asset_file_name_template: "[name]-[hash].[ext]".to_string(),
            asset_base: Some("https://cdn.example.com/assets/".to_string()),
            component_modules: Vec::new(),
            legal_comments: LegalComments::None,
        };
        let options = args
            .to_build_options(std::path::Path::new("."))
            .expect("build options");

        assert_eq!(options.dom, DomStrategy::Shadow);
        assert_eq!(options.css_file_name_template, "[name]-[hash].[ext]");
        assert_eq!(
            options.asset_base.as_deref(),
            Some("https://cdn.example.com/assets/")
        );
        assert!(options.component_asset_roots.is_empty());
        assert_eq!(options.legal_comments, LegalComments::None);
        assert_eq!(options.projection_manifests.len(), 2);
        assert!(matches!(
            &options.projection_manifests[0],
            webui::ProjectionManifestSource::Path(path)
                if path == std::path::Path::new("app-projection.json")
        ));
    }

    #[test]
    fn load_theme_resolves_packages_from_app_dir() {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path().join("app");
        let package_dir = app_dir.join("node_modules").join("@scope").join("tokens");
        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::write(
            package_dir.join("tokens.json"),
            r##"{"themes":{"light":{"color-brand":"#123456"}}}"##,
        )
        .unwrap();

        let theme = load_theme("@scope/tokens", &app_dir).unwrap();

        assert_eq!(theme.themes["light"]["color-brand"], "#123456");
    }
}
