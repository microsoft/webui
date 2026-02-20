use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

use crate::output::Printer;

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
    eprintln!();

    // Create output directory
    fs::create_dir_all(&args.out)
        .with_context(|| format!("Failed to create output dir: {}", args.out.display()))?;

    // Set up parser and register components from the app directory
    let mut parser = HtmlParser::new();
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
    let fragment_count: usize = fragment_records.values().map(|v| v.len()).sum();
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

    // Copy component CSS files
    for (filename, css_content) in &css_files {
        let css_path = args.out.join(filename);
        fs::write(&css_path, css_content)
            .with_context(|| format!("Failed to write {}", css_path.display()))?;
        printer.success(&format!("Wrote {}", printer.bold.apply_to(filename)));
        files_written += 1;
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
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;
    use webui_protocol::WebUIFragment;

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

        let index = &protocol.fragments["index.html"];
        assert!(index.iter().any(|f| matches!(f, WebUIFragment::For(_))));
        assert!(index.iter().any(|f| matches!(f, WebUIFragment::If(_))));
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
    fn test_build_missing_index_html() {
        let app_dir = create_app_dir(&[("other.html", "<h1>Not index</h1>")]);
        let out_dir = TempDir::new().unwrap();

        let result = build(app_dir.path(), out_dir.path(), "index.html");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to read"));
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
        let app_dir = manifest_dir.join("../../examples/hello-world");
        let out_dir = TempDir::new().unwrap();

        build(&app_dir, out_dir.path(), "index.html").unwrap();

        let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
        let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
        let index = &protocol.fragments["index.html"];

        assert!(index
            .iter()
            .any(|f| matches!(f, WebUIFragment::For(fl) if fl.collection == "people")));
        assert!(index.iter().any(|f| matches!(f, WebUIFragment::If(_))));
        assert!(index.iter().any(
            |f| matches!(f, WebUIFragment::Signal(s) if s.value == "raw_description" && s.raw)
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
}
