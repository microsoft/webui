// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use clap::Args;
use expand_tilde::expand_tilde;
use std::path::PathBuf;

use super::common::*;
use crate::utils::output;

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    pub app_args: AppArgs,

    /// Output folder for the built protocol and assets
    #[arg(long)]
    pub out: PathBuf,
}

pub fn execute(args: &BuildArgs) -> Result<()> {
    run(args).map_err(|err| {
        output::error(&err);

        let err_msg = format!("{:#}", err);
        if err_msg.contains("App folder not found") {
            output::hint("Check that the app folder path exists");
        } else if err_msg.contains("Failed to read") && args.app_args.entry == "index.html" {
            output::hint("Try using --entry <file> to specify a different entry file");
        }
        eprintln!();
        err
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
        .with_context(|| format!("App folder not found: {}", args.app_args.app.display()))?;

    output::header("WebUI Build");
    output::field("App", &app.display());
    output::field("Entry", &args.app_args.entry);
    output::field("Output", &out.display());
    output::field("CSS", &format!("{:?}", args.app_args.css));
    if let Some(ref plugin_name) = args.app_args.plugin {
        output::field("Plugin", plugin_name);
    }
    if !args.app_args.components.is_empty() {
        output::field("Components", &args.app_args.components.join(", "));
    }
    eprintln!();

    let build_options = args.app_args.to_build_options(&app);
    let stats = webui::build_to_disk(build_options, &out).with_context(|| "Build failed")?;

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

    let files_written = 1 + stats.css_file_count;
    output::success(&format!("Wrote {}", console::style("protocol.bin").bold()));

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
            plugin: None,
            components: Vec::new(),
        },
        out: out.to_path_buf(),
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
                plugin: None,
                components: Vec::new(),
            },
            out: out_dir.path().to_path_buf(),
        })
        .unwrap();

        assert!(out_dir.path().join("protocol.bin").exists());
        // Inline mode should NOT write external CSS files
        assert!(!out_dir.path().join("my-card.css").exists());
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
                plugin: None,
                components: vec![ext_path],
            },
            out: out_dir.path().to_path_buf(),
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
                plugin: None,
                components: vec!["test-widget".to_string()],
            },
            out: out_dir.path().to_path_buf(),
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
                plugin: None,
                components: vec!["@myui".to_string()],
            },
            out: out_dir.path().to_path_buf(),
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
