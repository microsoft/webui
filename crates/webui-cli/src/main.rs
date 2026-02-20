use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::Style;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

#[derive(Parser)]
#[command(name = "webui", about = "WebUI build tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a WebUI application from an app folder
    Build {
        /// Path to the app folder (defaults to current directory)
        #[arg(default_value = ".")]
        app: PathBuf,

        /// Output folder for the built protocol and assets
        #[arg(long)]
        out: PathBuf,

        /// Entry HTML file name (defaults to index.html)
        #[arg(long, default_value = "index.html")]
        entry: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { app, out, entry } => build(&app, &out, &entry),
    };

    if let Err(err) = result {
        let red = Style::new().red().bold();
        let dim = Style::new().dim();
        eprintln!("\n  {} {}", red.apply_to("✘"), red.apply_to(&err));
        for cause in err.chain().skip(1) {
            eprintln!("  {} {}", dim.apply_to("caused by:"), cause);
        }
        let err_msg = format!("{:#}", err);
        if err_msg.contains("Failed to read") && err_msg.contains("index.html") {
            eprintln!(
                "\n  {} Try using {} to specify a different entry file",
                dim.apply_to("hint:"),
                Style::new().bold().apply_to("--entry <file>")
            );
        }
        if err_msg.contains("App folder not found") {
            eprintln!(
                "\n  {} Check that the app folder path exists",
                dim.apply_to("hint:")
            );
        }
        eprintln!();
        std::process::exit(1);
    }
}

fn build(app: &Path, out: &Path, entry: &str) -> Result<()> {
    let started = Instant::now();
    let cyan = Style::new().cyan().bold();
    let green = Style::new().green();
    let dim = Style::new().dim();
    let bold = Style::new().bold();

    let app = app
        .canonicalize()
        .with_context(|| format!("App folder not found: {}", app.display()))?;

    // Print header
    eprintln!(
        "\n  {} {}\n",
        cyan.apply_to("⚡"),
        cyan.apply_to("WebUI Build")
    );
    eprintln!(
        "  {} {}",
        dim.apply_to("▸ App      "),
        bold.apply_to(app.display())
    );
    eprintln!("  {} {}", dim.apply_to("▸ Entry    "), bold.apply_to(entry));
    eprintln!(
        "  {} {}\n",
        dim.apply_to("▸ Output   "),
        bold.apply_to(out.display())
    );

    // Create output directory
    fs::create_dir_all(out)
        .with_context(|| format!("Failed to create output dir: {}", out.display()))?;

    // Set up parser and register components from the app directory
    let mut parser = HtmlParser::new();
    parser
        .component_registry_mut()
        .register_from_paths(&[&app])
        .context("Failed to register components")?;

    let component_count = parser.component_registry_mut().len();
    eprintln!(
        "  {} Registered {} component{}",
        green.apply_to("✔"),
        bold.apply_to(component_count),
        if component_count == 1 { "" } else { "s" }
    );

    // Read and parse the entry HTML file
    let entry_path = app.join(entry);
    let html_content = fs::read_to_string(&entry_path)
        .with_context(|| format!("Failed to read {}", entry_path.display()))?;

    parser
        .parse(entry, &html_content)
        .context("Failed to parse HTML")?;

    // Collect component CSS files before consuming parser
    let registry = parser.component_registry_mut();
    let css_files: Vec<(String, String)> = registry
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

    eprintln!(
        "  {} Parsed {} ({} fragment{})",
        green.apply_to("✔"),
        bold.apply_to(entry),
        bold.apply_to(fragment_count),
        if fragment_count == 1 { "" } else { "s" }
    );

    // Write protocol JSON
    let json = protocol
        .to_json_pretty()
        .context("Failed to serialize protocol")?;
    let protocol_path = out.join("protocol.json");
    fs::write(&protocol_path, &json)
        .with_context(|| format!("Failed to write {}", protocol_path.display()))?;
    eprintln!(
        "  {} Wrote {}",
        green.apply_to("✔"),
        bold.apply_to("protocol.json")
    );

    let mut files_written: usize = 1;

    // Copy component CSS files
    for (filename, css_content) in &css_files {
        let css_path = out.join(filename);
        fs::write(&css_path, css_content)
            .with_context(|| format!("Failed to write {}", css_path.display()))?;
        eprintln!(
            "  {} Wrote {}",
            green.apply_to("✔"),
            bold.apply_to(filename)
        );
        files_written += 1;
    }

    let elapsed = started.elapsed();
    eprintln!(
        "\n  {} Build complete ({} file{} written) {}\n",
        green.apply_to("✨"),
        bold.apply_to(files_written),
        if files_written == 1 { "" } else { "s" },
        dim.apply_to(format!("in {elapsed:.0?}")),
    );

    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
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

        let protocol_path = out_dir.path().join("protocol.json");
        assert!(protocol_path.exists());

        let json = fs::read_to_string(&protocol_path).unwrap();
        let protocol = WebUIProtocol::from_json(&json).unwrap();
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

        let json = fs::read_to_string(out_dir.path().join("protocol.json")).unwrap();
        let protocol = WebUIProtocol::from_json(&json).unwrap();

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

        // Protocol should exist
        assert!(out_dir.path().join("protocol.json").exists());
        // Component CSS should be copied
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

        assert!(nested_out.join("protocol.json").exists());
    }

    #[test]
    fn test_build_protocol_is_valid_json() {
        let app_dir = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "index.html").unwrap();

        let json = fs::read_to_string(out_dir.path().join("protocol.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("fragments").is_some());
    }

    #[test]
    fn test_build_hello_world_example() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_dir = manifest_dir.join("../../examples/hello-world");
        let out_dir = TempDir::new().unwrap();

        build(&app_dir, out_dir.path(), "index.html").unwrap();

        let json = fs::read_to_string(out_dir.path().join("protocol.json")).unwrap();
        let protocol = WebUIProtocol::from_json(&json).unwrap();
        let index = &protocol.fragments["index.html"];

        // Should have for loop for people
        assert!(index
            .iter()
            .any(|f| matches!(f, WebUIFragment::For(fl) if fl.collection == "people")));
        // Should have if condition for contact
        assert!(index.iter().any(|f| matches!(f, WebUIFragment::If(_))));
        // Should have raw signal for raw_description
        assert!(index.iter().any(
            |f| matches!(f, WebUIFragment::Signal(s) if s.value == "raw_description" && s.raw)
        ));
    }

    #[test]
    fn test_build_custom_entry_file() {
        let app_dir = create_app_dir(&[("page.html", "<h1>Custom Entry</h1>")]);
        let out_dir = TempDir::new().unwrap();

        build(app_dir.path(), out_dir.path(), "page.html").unwrap();

        let json = fs::read_to_string(out_dir.path().join("protocol.json")).unwrap();
        let protocol = WebUIProtocol::from_json(&json).unwrap();
        assert!(protocol.fragments.contains_key("page.html"));
        assert!(!protocol.fragments.contains_key("index.html"));
    }
}
