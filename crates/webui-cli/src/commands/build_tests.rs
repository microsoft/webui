use super::*;
use std::path::Path;
use tempfile::TempDir;
use webui_protocol::web_ui_fragment::Fragment;

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
        app: app_dir.path().to_path_buf(),
        out: out_dir.path().to_path_buf(),
        entry: "index.html".to_string(),
        css: CssMode::Inline,
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
    let app_dir = manifest_dir.join("../../examples/app/hello-world/templates");
    let out_dir = TempDir::new().unwrap();

    build(&app_dir, out_dir.path(), "index.html").unwrap();

    let bytes = fs::read(out_dir.path().join("protocol.bin")).unwrap();
    let protocol = WebUIProtocol::from_protobuf(&bytes).unwrap();
    let index = &protocol.fragments["index.html"].fragments;

    assert!(index.iter().any(
        |f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.collection == "people")
    ));
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
