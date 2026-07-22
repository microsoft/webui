// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use clap::Args;
use expand_tilde::expand_tilde;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use super::common::*;
use crate::utils::error::CliError;
use crate::utils::output;

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    pub app_args: AppArgs,

    /// Output destination. Either a folder (e.g. `./dist`) or a `.bin` file path
    /// (e.g. `./dist/app1.bin`). When a `.bin` file path is given, the protocol
    /// is written with that filename and CSS files are emitted next to it.
    #[arg(long)]
    pub out: PathBuf,

    /// Comma-separated root component tags to emit as static CDN-loadable assets
    #[arg(long, value_delimiter = ',', value_name = "TAGS")]
    pub emit_component_assets: Vec<String>,

    /// Emit a render-state JSON Schema beside the compiled protocol
    #[arg(long)]
    pub emit_schema: bool,

    /// Design token theme to validate against: a JSON file path or npm package name.
    /// Missing unresolved CSS tokens fail the build.
    #[arg(long)]
    pub theme: Option<String>,
}

/// Resolve the `--out` argument into `(output_directory, protocol_filename)`.
///
/// If `out` ends with a `.bin` extension, it is treated as a full file path:
/// the parent becomes the output directory (or `.` if none) and the file name
/// becomes the protocol filename. Otherwise `out` is treated as a directory and
/// the default `protocol.bin` filename is used.
///
/// The filename is kept as an `OsString` so non-UTF8 paths round-trip unchanged.
fn resolve_out(out: &Path) -> (PathBuf, OsString) {
    if out.extension().and_then(|e| e.to_str()) == Some("bin") {
        let name = out
            .file_name()
            .map(OsString::from)
            .unwrap_or_else(|| OsString::from("protocol.bin"));
        let dir = match out.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        (dir, name)
    } else {
        (out.to_path_buf(), OsString::from("protocol.bin"))
    }
}

fn schema_file_name(protocol_name: &OsStr) -> OsString {
    let stem = Path::new(protocol_name)
        .file_stem()
        .filter(|value| !value.is_empty())
        .unwrap_or(protocol_name);
    let mut name = stem.to_os_string();
    name.push(".state.schema.json");
    name
}

fn validate_output_file_names(
    protocol_name: &OsStr,
    schema_name: Option<&OsStr>,
    result: &webui::BuildResult,
) -> Result<()> {
    let mut names = HashSet::with_capacity(
        1 + usize::from(schema_name.is_some())
            + result.css_files.len()
            + result.component_asset_files.len(),
    );
    insert_output_file_name(&mut names, protocol_name)?;
    if let Some(schema_name) = schema_name {
        insert_output_file_name(&mut names, schema_name)?;
    }
    for (name, _) in &result.css_files {
        insert_output_file_name(&mut names, OsStr::new(name))?;
    }
    for file in &result.component_asset_files {
        insert_output_file_name(&mut names, OsStr::new(&file.name))?;
    }
    Ok(())
}

fn insert_output_file_name(names: &mut HashSet<String>, name: &OsStr) -> Result<()> {
    let key = name.to_string_lossy().to_lowercase();
    if names.insert(key) {
        return Ok(());
    }
    anyhow::bail!(
        "output filename collision for '{}'. Choose a different --out filename or adjust --asset-file-name-template.",
        name.to_string_lossy()
    );
}

pub fn execute(args: &BuildArgs) -> Result<()> {
    run(args).inspect_err(|err| {
        output::error(err);
        if let Some(cli_err) = err.chain().find_map(|c| c.downcast_ref::<CliError>()) {
            output::hint(cli_err.hint());
        }
        eprintln!();
    })
}

fn run(args: &BuildArgs) -> Result<()> {
    let app_input = expand_tilde(&args.app_args.app)
        .with_context(|| format!("Failed to expand app path: {}", args.app_args.app.display()))?
        .into_owned();
    let out = expand_tilde(&args.out)
        .with_context(|| format!("Failed to expand output path: {}", args.out.display()))?
        .into_owned();

    let app = app_input
        .canonicalize()
        .map_err(|_| CliError::AppFolderNotFound {
            path: args.app_args.app.display().to_string(),
        })?;

    let entry_path = app.join(&args.app_args.entry);
    if !entry_path.is_file() {
        return Err(CliError::EntryReadFailed {
            path: entry_path.display().to_string(),
        }
        .into());
    }

    let (out_dir, protocol_name) = resolve_out(&out);
    let protocol_path = out_dir.join(&protocol_name);
    let schema_name = args.emit_schema.then(|| schema_file_name(&protocol_name));
    let schema_path = schema_name.as_ref().map(|name| out_dir.join(name));

    output::header("WebUI Build");
    output::field("App", &app.display());
    output::field("Entry", &args.app_args.entry);
    output::field("Output", &protocol_path.display());
    output::field("CSS", &args.app_args.css);
    if let Some(ref plugin_name) = args.app_args.plugin {
        output::field("Plugin", plugin_name);
    }
    if !args.app_args.components.is_empty() {
        output::field("Components", &args.app_args.components.join(", "));
    }
    if !args.emit_component_assets.is_empty() {
        output::field("Component assets", &args.emit_component_assets.join(", "));
    }
    if let Some(schema_path) = &schema_path {
        output::field("Schema", &schema_path.display());
    }
    if let Some(ref theme) = args.theme {
        output::field("Theme", theme);
    }
    eprintln!();

    let mut build_options = args.app_args.to_build_options(&app);
    build_options.component_asset_roots = args.emit_component_assets.clone();
    build_options.theme = args
        .theme
        .as_deref()
        .map(|theme| load_theme(theme, &app))
        .transpose()?;
    let result = webui::build(build_options).with_context(|| "Build failed")?;
    validate_output_file_names(&protocol_name, schema_name.as_deref(), &result)?;
    let schema_output = schema_path
        .as_ref()
        .map(|path| -> Result<(PathBuf, Vec<u8>)> {
            let schema = super::state_schema::generate_schema(
                &result.protocol,
                &args.app_args.entry,
                super::state_schema::DEFAULT_SCHEMA_TITLE,
            )?;
            let output = super::state_schema::schema_to_pretty_json(&schema)
                .with_context(|| "Failed to serialize render-state schema")?;
            Ok((path.clone(), output.into_bytes()))
        })
        .transpose()?;

    fs::create_dir_all(&out_dir)
        .with_context(|| format!("Failed to create {}", out_dir.display()))?;
    fs::write(&protocol_path, &result.protocol_bytes)
        .with_context(|| format!("Failed to write {}", protocol_path.display()))?;
    for (name, content) in &result.css_files {
        fs::write(out_dir.join(name), content)
            .with_context(|| format!("Failed to write {name} to {}", out_dir.display()))?;
    }
    for file in &result.component_asset_files {
        fs::write(out_dir.join(&file.name), &file.content).with_context(|| {
            format!(
                "Failed to write component asset {} to {}",
                file.name,
                out_dir.display()
            )
        })?;
    }
    if let Some((schema_path, schema_bytes)) = &schema_output {
        fs::write(schema_path, schema_bytes)
            .with_context(|| format!("Failed to write {}", schema_path.display()))?;
    }
    let stats = result.stats;

    output::success(&format!(
        "Registered {} component{}",
        console::style(stats.component_count).bold(),
        if stats.component_count == 1 { "" } else { "s" }
    ));

    output::success(&format!(
        "Parsed {} ({} fragment{})",
        console::style(&args.app_args.entry).bold(),
        console::style(stats.fragment_count).bold(),
        if stats.fragment_count == 1 { "" } else { "s" }
    ));

    if stats.token_count > 0 {
        output::success(&format!(
            "Discovered {} CSS token{}",
            console::style(stats.token_count).bold(),
            if stats.token_count == 1 { "" } else { "s" }
        ));
    }

    if !result.component_asset_files.is_empty() {
        output::success(&format!(
            "Emitted {} component asset{}",
            console::style(result.component_asset_files.len()).bold(),
            if result.component_asset_files.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }

    let files_written = 1
        + stats.css_file_count
        + result.component_asset_files.len()
        + usize::from(schema_output.is_some());
    output::success(&format!(
        "Wrote {}",
        console::style(Path::new(&protocol_name).display()).bold()
    ));
    if let Some(schema_name) = schema_name {
        output::success(&format!(
            "Wrote {}",
            console::style(Path::new(&schema_name).display()).bold()
        ));
    }

    for advisory in &result.warnings {
        output::warning_diagnostic(advisory);
    }

    output::finish(&format!(
        "Build complete ({} file{} written) {}",
        console::style(files_written).bold(),
        if files_written == 1 { "" } else { "s" },
        console::style(format!("in {:.0?}", stats.duration)).dim(),
    ));

    Ok(())
}

/// Standalone build function for testing without CLI args.
#[cfg(test)]
pub fn build(app: &std::path::Path, out: &std::path::Path, entry: &str) -> Result<()> {
    run(&BuildArgs {
        app_args: AppArgs {
            app: app.to_path_buf(),
            entry: entry.to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
            projection_manifests: Vec::new(),
            asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
            css_public_base: None,
            legal_comments: LegalComments::Inline,
        },
        out: out.to_path_buf(),
        emit_component_assets: Vec::new(),
        emit_schema: false,
        theme: None,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;
    use webui_protocol::web_ui_fragment::Fragment;
    use webui_protocol::WebUIProtocol;

    fn create_app_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    #[test]
    fn test_build_simple_html() {
        let app_dir = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let protocol_path = out_dir.path().join("protocol.bin");
        assert!(protocol_path.exists());

        let bytes = fs::read(&protocol_path).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert!(protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_with_directives() {
        let html = r#"<h1>Hello</h1>
<for each="item in items">
    <p>{{item.name}}</p>
</for>
<if condition="show">
    <p>Visible</p>
</if>"#;
        let app_dir = create_app_dir(&[("index.html", html)]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();

        let index = &protocol.fragments["index.html"].fragments;
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(_)))));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
    }

    #[test]
    fn test_build_with_component_css() {
        let app_dir = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        assert!(out_dir.path().join("protocol.bin").exists());
        let css_path = out_dir.path().join("my-card.css");
        assert!(css_path.exists());
        let css = fs::read_to_string(&css_path).unwrap();
        assert!(css.contains("color: red"));
    }

    #[test]
    fn test_build_with_inline_css_skips_css_files() {
        let app_dir = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Style,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        assert!(out_dir.path().join("protocol.bin").exists());
        // Inline mode should NOT write external CSS files
        assert!(!out_dir.path().join("my-card.css").exists());
    }

    #[test]
    fn test_build_emits_static_component_assets() {
        let app_dir = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", r#"<div w-ref="{slot}"></div>"#),
            (
                "mail-thread.html",
                r#"<if condition="hasMessages"><mail-message title="{{title}}"></mail-message></if>"#,
            ),
            ("mail-thread.ts", "export {};"),
            ("mail-message.html", "<p>{{title}}</p>"),
            ("mail-message.ts", "export {};"),
        ]);
        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: Some(Plugin::WebUI),
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: vec!["mail-thread".to_string()],
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        let asset_path = out_dir.path().join("mail-thread.webui.js");
        assert!(asset_path.exists());

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let index_fragments = &protocol.fragments["index.html"].fragments;
        assert!(
            !index_fragments.iter().any(|fragment| matches!(
                fragment.fragment.as_ref(),
                Some(Fragment::Component(component)) if component.fragment_id == "mail-thread"
            )),
            "mail-thread must not be reachable from the SSR entry fragment"
        );

        let asset = fs::read_to_string(asset_path).unwrap();
        assert!(asset.contains(r#""type":"webui-component-asset""#));
        assert!(asset.contains(r#""version":1"#));
        assert!(!asset.contains(r#""plugin""#));
        assert!(!asset.contains(r#""inventory""#));
        assert!(asset.contains(r#""components":["mail-message","mail-thread"]"#));
        assert!(asset.contains(r#""templates":{"mail-message":"#));
        assert!(asset.contains(r#""mail-thread":"#));
        assert!(asset.contains(r#""templateFunctions":{"mail-thread":"#));
        assert!(asset.contains("export default asset;"));
    }

    #[test]
    fn test_build_rejects_duplicate_component_assets_before_writing() {
        let app_dir = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            ("mail-thread.html", "<p>Mail</p>"),
        ]);
        let out_dir = TempDir::new().unwrap();

        let result = run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: Some(Plugin::WebUI),
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: vec!["mail-thread".to_string(), "mail-thread".to_string()],
            emit_schema: false,
            theme: None,
        });

        assert!(result.is_err());
        assert!(!out_dir.path().join("protocol.bin").exists());
    }

    #[test]
    fn test_build_emits_fast_component_assets() {
        let app_dir = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            ("fast-card.html", "<p>{{title}}</p>"),
        ]);
        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: Some(Plugin::FastV3),
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: vec!["fast-card".to_string()],
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        let asset_path = out_dir.path().join("fast-card.webui.js");
        assert!(asset_path.exists());
        let asset = fs::read_to_string(asset_path).unwrap();
        assert!(asset.contains(r#""type":"webui-component-asset""#));
        assert!(asset.contains(r#""version":1"#));
        assert!(!asset.contains(r#""plugin""#));
        assert!(!asset.contains(r#""templateFunctionModule""#));
        assert!(!asset.contains(r#""templateFunctions""#));
        assert!(asset.contains("<f-template"));
    }

    #[test]
    fn test_build_emits_hashed_component_asset_filename() {
        let app_dir = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            ("mail-thread.html", "<p>{{title}}</p>"),
            ("mail-thread.ts", "export {};"),
        ]);
        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: Some(Plugin::WebUI),
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: "[name]-[hash].[ext]".to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: vec!["mail-thread".to_string()],
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        let asset_names: Vec<String> = fs::read_dir(out_dir.path())
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".webui.js") {
                    Some(name.into_owned())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(asset_names.len(), 1);
        assert!(asset_names[0].starts_with("mail-thread-"));
        assert!(asset_names[0].ends_with(".webui.js"));
        assert_ne!(asset_names[0], "mail-thread-.webui.js");
    }

    #[test]
    fn test_build_missing_index_html() {
        let app_dir = create_app_dir(&[("other.html", "<h1>Not index</h1>")]);
        let out_dir = TempDir::new().unwrap();

        let result = build(app_dir.path(), out_dir.path(), "index.html");
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Failed to read"));
    }

    #[test]
    fn test_build_missing_app_folder() {
        let out_dir = TempDir::new().unwrap();
        let result = build(Path::new("/nonexistent/path"), out_dir.path(), "index.html");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_creates_output_dir() {
        let app_dir = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out_dir = TempDir::new().unwrap();
        let nested_out = out_dir.path().join("nested").join("output");

        build(app_dir.path(), &nested_out, "index.html").unwrap();

        assert!(nested_out.join("protocol.bin").exists());
    }

    #[test]
    fn test_build_protocol_is_valid_protobuf() {
        let app_dir = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert!(protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_hello_world_example() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_dir = manifest_dir.join("../../examples/app/hello-world/src");
        let out_dir = TempDir::new().unwrap();

        build(&app_dir, out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let index = &protocol.fragments["index.html"].fragments;

        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.collection == "people")));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
        assert!(index.iter().any(
            |f| matches!(f.fragment.as_ref(), Some(Fragment::Signal(s)) if s.value == "raw_description" && s.raw)
        ));
    }

    #[test]
    fn test_build_custom_entry_file() {
        let app_dir = create_app_dir(&[("page.html", "<h1>Custom Entry</h1>")]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "page.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert!(protocol.fragments.contains_key("page.html"));
        assert!(!protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_with_components_local_path() {
        // App directory with an entry that uses an external component
        let app_dir = create_app_dir(&[("index.html", "<ext-card>Hello</ext-card>")]);

        // External component directory (separate from app)
        let ext_dir = TempDir::new().unwrap();
        fs::write(
            ext_dir.path().join("ext-card.html"),
            "<div class=\"card\"><slot></slot></div>",
        )
        .unwrap();
        fs::write(
            ext_dir.path().join("ext-card.css"),
            ".card { border: 1px solid #ccc; }",
        )
        .unwrap();

        let out_dir = TempDir::new().unwrap();
        let ext_path = ext_dir.path().to_string_lossy().to_string();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: vec![ext_path],
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        // protocol.bin should exist
        assert!(out_dir.path().join("protocol.bin").exists());
        // External component CSS should be emitted
        let css_path = out_dir.path().join("ext-card.css");
        assert!(css_path.exists());
        let css = fs::read_to_string(&css_path).unwrap();
        assert!(css.contains("border"));
    }

    #[test]
    fn test_build_with_components_npm_package() {
        // Create a mock project with node_modules alongside app dir
        let project_dir = TempDir::new().unwrap();
        let nm = project_dir.path().join("node_modules");
        let pkg_dir = nm.join("test-widget");
        fs::create_dir_all(&pkg_dir).unwrap();

        // Create the npm package files
        fs::write(
            pkg_dir.join("template-webui.html"),
            "<button><slot></slot></button>",
        )
        .unwrap();
        fs::write(pkg_dir.join("styles.css"), ".btn { padding: 4px; }").unwrap();

        let manifest = serde_json::json!({
            "schemaVersion": "1.0.0",
            "modules": [{
                "kind": "javascript-module",
                "declarations": [{
                    "kind": "class",
                    "tagName": "test-widget"
                }]
            }]
        });
        fs::write(
            pkg_dir.join("custom-elements.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let pkg_json = serde_json::json!({
            "name": "test-widget",
            "version": "1.0.0",
            "customElements": "./custom-elements.json",
            "exports": {
                "./template-webui.html": "./template-webui.html",
                "./styles.css": "./styles.css"
            }
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string(&pkg_json).unwrap(),
        )
        .unwrap();

        // Create app directory under project_dir (node_modules found via upward walk)
        let app_dir = project_dir.path().join("src");
        fs::create_dir_all(&app_dir).unwrap();
        fs::write(
            app_dir.join("index.html"),
            "<test-widget>Click me</test-widget>",
        )
        .unwrap();

        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir,
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: vec!["test-widget".to_string()],
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        assert!(out_dir.path().join("protocol.bin").exists());
        let css_path = out_dir.path().join("test-widget.css");
        assert!(css_path.exists());
        let css = fs::read_to_string(&css_path).unwrap();
        assert!(css.contains("padding"));
    }

    #[test]
    fn test_build_with_components_npm_scoped() {
        // Create a mock scoped npm package
        let project_dir = TempDir::new().unwrap();
        let nm = project_dir.path().join("node_modules");
        let scope_dir = nm.join("@myui");
        fs::create_dir_all(&scope_dir).unwrap();

        // Create two sub-packages under the scope
        for (sub, tag, html) in &[
            ("btn", "myui-btn", "<button><slot></slot></button>"),
            ("txt", "myui-txt", "<span><slot></slot></span>"),
        ] {
            let pkg_dir = scope_dir.join(sub);
            fs::create_dir_all(&pkg_dir).unwrap();

            fs::write(pkg_dir.join("template-webui.html"), html).unwrap();

            let manifest = serde_json::json!({
                "schemaVersion": "1.0.0",
                "modules": [{
                    "kind": "javascript-module",
                    "declarations": [{ "kind": "class", "tagName": tag }]
                }]
            });
            fs::write(
                pkg_dir.join("custom-elements.json"),
                serde_json::to_string(&manifest).unwrap(),
            )
            .unwrap();

            let pkg_json = serde_json::json!({
                "name": format!("@myui/{sub}"),
                "version": "1.0.0",
                "customElements": "./custom-elements.json",
                "exports": {
                    "./template-webui.html": "./template-webui.html"
                }
            });
            fs::write(
                pkg_dir.join("package.json"),
                serde_json::to_string(&pkg_json).unwrap(),
            )
            .unwrap();
        }

        // App under project_dir
        let app_dir = project_dir.path().join("src");
        fs::create_dir_all(&app_dir).unwrap();
        fs::write(
            app_dir.join("index.html"),
            "<myui-btn>Go</myui-btn><myui-txt>Hi</myui-txt>",
        )
        .unwrap();

        let out_dir = TempDir::new().unwrap();

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir,
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: vec!["@myui".to_string()],
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        assert!(out_dir.path().join("protocol.bin").exists());
    }

    #[test]
    fn test_build_protocol_includes_tokens_from_components() {
        let app_dir = create_app_dir(&[
            ("index.html", "<my-btn></my-btn>"),
            ("my-btn.html", "<button><slot></slot></button>"),
            (
                "my-btn.css",
                ".btn { color: var(--text-color); padding: var(--spacing-m); }",
            ),
        ]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();

        assert_eq!(protocol.tokens, vec!["spacing-m", "text-color"]);
    }

    #[test]
    fn test_build_theme_missing_token_fails() {
        let app_dir = create_app_dir(&[
            ("index.html", "<my-btn></my-btn>"),
            ("my-btn.html", "<button><slot></slot></button>"),
            (
                "my-btn.css",
                ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c))); }",
            ),
            ("theme.json", r#"{"themes":{"light":{"token-b":"green"}}}"#),
        ]);
        let out_dir = TempDir::new().unwrap();
        let result = run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: out_dir.path().to_path_buf(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: Some(
                app_dir
                    .path()
                    .join("theme.json")
                    .to_string_lossy()
                    .to_string(),
            ),
        });

        let err = result.expect_err("missing theme token must fail");
        let message = format!("{err:#}");
        assert!(message.contains("missing-theme-token"), "msg: {message}");
        assert!(message.contains("--token-c"), "msg: {message}");
        assert!(!out_dir.path().join("protocol.bin").exists());
    }

    #[test]
    fn test_build_custom_protocol_name() {
        let app_dir = create_app_dir(&[
            ("index.html", "<my-card>Hi</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out_dir = TempDir::new().unwrap();
        let custom_path = out_dir.path().join("app1.bin");

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: custom_path.clone(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        // Protocol is written under the requested filename, not protocol.bin.
        assert!(custom_path.exists());
        assert!(!out_dir.path().join("protocol.bin").exists());

        // The bytes are a valid protocol.
        let bytes = fs::read(&custom_path).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        assert!(protocol.fragments.contains_key("index.html"));

        // CSS files are emitted next to the renamed protocol.
        assert!(out_dir.path().join("my-card.css").exists());
        assert!(!out_dir.path().join("app1.state.schema.json").exists());
    }

    #[test]
    fn test_build_emits_schema_beside_custom_protocol() {
        let app_dir = create_app_dir(&[(
            "index.html",
            "<h1>{{title}}</h1><for each=\"item in items\">{{item.name}}</for>",
        )]);
        let out_dir = TempDir::new().unwrap();
        let protocol_path = out_dir.path().join("catalog.bin");

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: protocol_path.clone(),
            emit_component_assets: Vec::new(),
            emit_schema: true,
            theme: None,
        })
        .unwrap();

        let schema_path = out_dir.path().join("catalog.state.schema.json");
        assert!(protocol_path.exists());
        assert!(schema_path.exists());
        let schema: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(schema_path).unwrap()).unwrap();
        assert_eq!(schema["title"], "WebUIState");
        assert_eq!(schema["properties"]["items"]["type"], "array");
        assert!(schema["properties"]["title"]["type"].is_array());
    }

    #[test]
    fn test_build_custom_protocol_name_creates_parent_dir() {
        let app_dir = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out_dir = TempDir::new().unwrap();
        let nested = out_dir.path().join("nested").join("app2.bin");

        run(&BuildArgs {
            app_args: AppArgs {
                app: app_dir.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                projection_manifests: Vec::new(),
                asset_file_name_template: DEFAULT_ASSET_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            out: nested.clone(),
            emit_component_assets: Vec::new(),
            emit_schema: false,
            theme: None,
        })
        .unwrap();

        assert!(nested.exists());
        assert!(!nested.parent().unwrap().join("protocol.bin").exists());
    }

    #[test]
    fn test_resolve_out_directory() {
        let (dir, name) = resolve_out(Path::new("./dist"));
        assert_eq!(dir, PathBuf::from("./dist"));
        assert_eq!(name, "protocol.bin");
    }

    #[test]
    fn test_schema_file_name_tracks_protocol_stem() {
        assert_eq!(
            schema_file_name(std::ffi::OsStr::new("protocol.bin")),
            "protocol.state.schema.json"
        );
        assert_eq!(
            schema_file_name(std::ffi::OsStr::new("app1.bin")),
            "app1.state.schema.json"
        );
    }

    #[test]
    fn test_schema_collision_check_is_case_insensitive() {
        let app_dir = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let mut result = webui::build(webui::BuildOptions {
            app_dir: app_dir.path().to_path_buf(),
            ..webui::BuildOptions::default()
        })
        .unwrap();
        result.css_files.push((
            "catalog.state.schema.json".to_string(),
            "collision".to_string(),
        ));

        let error = validate_output_file_names(
            std::ffi::OsStr::new("Catalog.bin"),
            Some(std::ffi::OsStr::new("Catalog.state.schema.json")),
            &result,
        )
        .unwrap_err();

        assert!(error.to_string().contains("filename collision"));
    }

    #[test]
    fn test_resolve_out_bin_file_with_parent() {
        let (dir, name) = resolve_out(Path::new("./dist/app1.bin"));
        assert_eq!(dir, PathBuf::from("./dist"));
        assert_eq!(name, "app1.bin");
    }

    #[test]
    fn test_resolve_out_bin_file_no_parent() {
        let (dir, name) = resolve_out(Path::new("app1.bin"));
        assert_eq!(dir, PathBuf::from("."));
        assert_eq!(name, "app1.bin");
    }

    #[test]
    fn test_build_protocol_excludes_entry_defined_tokens() {
        let html = r#"<style>
            :root { --text-color: #333; --spacing-m: 12px; }
            body { color: var(--text-color); }
        </style>
        <my-btn></my-btn>"#;
        let app_dir = create_app_dir(&[
            ("index.html", html),
            ("my-btn.html", "<button><slot></slot></button>"),
            (
                "my-btn.css",
                ".btn { color: var(--text-color); margin: var(--spacing-m); }",
            ),
        ]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();

        // Both tokens are defined in entry :root — should not be in protocol
        assert!(
            protocol.tokens.is_empty(),
            "Entry-defined tokens should be excluded: {:?}",
            protocol.tokens
        );
    }
}
