// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value};
use webui_docs::{build_docs, DocsConfig};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn builds_layout_scoped_regions_for_pages_and_404() -> TestResult {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("workspace root is missing")?;
    ensure_projection_package(workspace)?;

    let root = manifest_dir
        .join("target")
        .join(format!("regions-build-{}", std::process::id()));
    fs::remove_dir_all(&root).ok();
    let template_dir = root.join("template");
    let components_dir = root.join("components");
    let content_dir = root.join("content");
    let scripts_dir = root.join("scripts");
    let public_dir = root.join("public");
    let out_dir = root.join("dist");
    for path in [
        &template_dir,
        &components_dir,
        &content_dir,
        &scripts_dir,
        &public_dir,
    ] {
        fs::create_dir_all(path)?;
    }

    fs::write(
        template_dir.join("index.html"),
        concat!(
            "<!DOCTYPE html><html><head><base href=\"{{site.base}}\">",
            "{{{headTags}}}</head><body data-layout=\"{{page.layout}}\">",
            "<webui-press-region name=\"home.panel\" layout=\"home\">",
            "<home-region label=\"{{regions.home.panel.label}}\"></home-region>",
            "</webui-press-region>",
            "<webui-press-region name=\"doc.panel\" layout=\"doc\">",
            "<replaced-region></replaced-region>",
            "</webui-press-region>",
            "<main>{{{page.content}}}</main></body></html>"
        ),
    )?;
    write_component(&components_dir, "home-region", "<strong>{{label}}</strong>")?;
    write_component(&components_dir, "doc-region", "<strong>{{label}}</strong>")?;
    write_component(
        &components_dir,
        "replaced-region",
        "<strong>must not render</strong>",
    )?;
    fs::write(
        content_dir.join("index.md"),
        "---\nlayout: home\ntitle: Home\n---\n\n# Home",
    )?;
    fs::write(content_dir.join("guide.md"), "# Guide")?;
    fs::write(
        scripts_dir.join("home.ts"),
        "globalThis.__homeRegionScript = true;",
    )?;
    fs::write(
        scripts_dir.join("doc.ts"),
        "globalThis.__docRegionScript = true;",
    )?;

    let config: DocsConfig = serde_json::from_value(object([
        ("site", object([("title", string("Regions"))])),
        ("basePath", string("/fixture/")),
        ("contentDir", path_value(&content_dir)),
        ("outDir", path_value(&out_dir)),
        ("publicDir", path_value(&public_dir)),
        ("nav", Value::Array(Vec::new())),
        ("sidebar", Value::Array(Vec::new())),
        (
            "regions",
            object([
                (
                    "home.panel",
                    object([
                        ("state", object([("label", string("Home fallback"))])),
                        ("scriptFile", string("scripts/home.ts")),
                    ]),
                ),
                (
                    "doc.panel",
                    object([
                        (
                            "html",
                            string(
                                "<doc-region label=\"{{regions.doc.panel.label}}\"></doc-region>",
                            ),
                        ),
                        ("state", object([("label", string("Doc override"))])),
                        ("scriptFile", string("scripts/doc.ts")),
                    ]),
                ),
            ]),
        ),
    ]))?;

    build_docs(&config, &root, &template_dir)?;

    let home = fs::read_to_string(out_dir.join("index.html"))?;
    let doc = fs::read_to_string(out_dir.join("guide/index.html"))?;
    let not_found = fs::read_to_string(out_dir.join("404.html"))?;
    assert_page(&home, "home-region", "Home fallback", "replaced-region");
    assert_page(&doc, "doc-region", "Doc override", "replaced-region");
    assert_page(&not_found, "doc-region", "Doc override", "replaced-region");

    let home_script = read_page_script(&home, &out_dir)?;
    let doc_script = read_page_script(&doc, &out_dir)?;
    let not_found_script = read_page_script(&not_found, &out_dir)?;
    assert!(home_script.contains("__homeRegionScript"));
    assert!(!home_script.contains("__docRegionScript"));
    assert!(doc_script.contains("__docRegionScript"));
    assert!(!doc_script.contains("__homeRegionScript"));
    assert!(not_found_script.contains("__docRegionScript"));

    fs::remove_dir_all(root).ok();
    Ok(())
}

fn ensure_projection_package(workspace: &Path) -> TestResult {
    let entry = workspace.join("packages/webui/dist/projection/index.js");
    if entry.exists() {
        return Ok(());
    }
    let program = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };
    let status = Command::new(program)
        .args(["--filter", "@microsoft/webui", "build"])
        .current_dir(workspace)
        .status()?;
    if !status.success() {
        return Err("failed to build @microsoft/webui projection package".into());
    }
    Ok(())
}

fn write_component(root: &Path, name: &str, html: &str) -> TestResult {
    let dir = root.join(name);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{name}.html")), html)?;
    Ok(())
}

fn assert_page(html: &str, component: &str, text: &str, absent: &str) {
    assert!(html.contains(&format!("<{component}")));
    assert!(html.contains(text));
    assert!(!html.contains(&format!("<{absent}")));
}

fn read_page_script(html: &str, out_dir: &Path) -> TestResult<String> {
    let marker = "<script type=\"module\" src=\"";
    let start = html.find(marker).ok_or("module script is missing")? + marker.len();
    let end = html[start..]
        .find('"')
        .ok_or("module script is malformed")?
        + start;
    let relative = html[start..end]
        .split('?')
        .next()
        .ok_or("module script path is missing")?
        .strip_prefix("/fixture/")
        .ok_or("module script has the wrong base path")?;
    Ok(fs::read_to_string(out_dir.join(relative))?)
}

fn path_value(path: &Path) -> Value {
    Value::String(path.to_string_lossy().into_owned())
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::with_capacity(N);
    for (key, value) in entries {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}
