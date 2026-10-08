// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const EXAMPLE: &str = concat!(
    "<state-example :sample=\"{{sample}}\" :items=\"{{items}}\"></state-example>\n",
    "<p data-shared=\"{{shared}}\" data-only-a=\"{{onlyA}}\" ",
    "data-title=\"{{page.title}}\" data-site=\"{{site.title}}\">State</p>\n",
);

fn fixture() -> TestResult<TempDir> {
    let root = tempfile::tempdir()?;
    for directory in ["content", "template", "components/state-example"] {
        fs::create_dir_all(root.path().join(directory))?;
    }
    fs::write(
        root.path().join("template/index.html"),
        concat!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\">",
            "<title>{{page.title}}</title>{{{headTags}}}</head>",
            "<body><main>{{{page.content}}}</main></body></html>"
        ),
    )?;
    fs::write(
        root.path()
            .join("components/state-example/state-example.html"),
        concat!(
            "<template><strong data-value=\"{{sample.value}}\">{{sample.value}}</strong>",
            "<span data-global-only=\"{{sample.globalOnly}}\"></span>",
            "<for each=\"item in items\"><span data-item=\"{{item}}\">{{item}}</span></for>",
            "</template>"
        ),
    )?;
    fs::write(
        root.path().join("config.json"),
        r#"{
            "site": {"title": "Canonical docs"},
            "basePath": "/",
            "contentDir": "content",
            "outDir": "dist",
            "publicDir": "public",
            "nav": [],
            "sidebar": [],
            "state": {
                "sample": {"value": "global", "globalOnly": true},
                "items": ["global-item"],
                "shared": "global-shared"
            }
        }"#,
    )?;
    Ok(root)
}

fn build(root: &Path, mode: &str) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_webui-press"))
        .args(["build", "--config", "config.json", "--template"])
        .arg(root.join("template"))
        .args(["--show", mode])
        .current_dir(root)
        .output()?)
}

fn successful_build(root: &Path, mode: &str) -> TestResult {
    let output = build(root, mode)?;
    assert!(
        output.status.success(),
        "{mode}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn cli_build_isolates_markdown_state_in_full_and_content_modes() -> TestResult {
    let root = fixture()?;
    fs::write(
        root.path().join("content/a.md"),
        format!(
            "---\ntitle: Page A\nstate:\n  sample:\n    value: local-a\n  items: [a-one, a-two]\n  onlyA: exclusive-a\n---\n\n{EXAMPLE}"
        ),
    )?;
    fs::write(
        root.path().join("content/b.md"),
        format!(
            "---\r\ntitle: Page B\r\nstate: {{sample: {{value: local-b}}, items: [b-one]}}\r\n---\r\n\r\n{EXAMPLE}"
        ),
    )?;
    fs::write(root.path().join("content/global.md"), EXAMPLE)?;
    fs::write(
        root.path().join("content/empty.md"),
        format!("---\nstate: {{}}\n---\n\n{EXAMPLE}"),
    )?;

    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        for (page, value, item, title) in [
            ("a", "local-a", "a-one", "Page A"),
            ("b", "local-b", "b-one", "Page B"),
            ("global", "global", "global-item", "Canonical docs"),
            ("empty", "global", "global-item", "Canonical docs"),
        ] {
            let html = fs::read_to_string(root.path().join(format!("dist/{page}/index.html")))?;
            assert!(
                html.contains(&format!("data-value=\"{value}\"")),
                "{page}: {html}"
            );
            assert!(
                html.contains(&format!("data-item=\"{item}\"")),
                "{page}: {html}"
            );
            assert!(html.contains("data-shared=\"global-shared\""));
            assert!(html.contains(&format!("data-title=\"{title}\"")));
            assert!(html.contains("data-site=\"Canonical docs\""));
            if page == "a" || page == "b" {
                assert!(
                    html.contains("data-global-only=\"\""),
                    "shallow replacement"
                );
            }
            if page == "a" {
                assert!(html.contains("data-only-a=\"exclusive-a\""));
            } else {
                assert!(!html.contains("exclusive-a"), "state leaked to {page}");
            }
            if page != "b" {
                assert!(!html.contains("local-b"), "state leaked to {page}");
            }
        }
        let not_found = fs::read_to_string(root.path().join("dist/404.html"))?;
        assert!(!not_found.contains("exclusive-a"));
        assert!(!not_found.contains("local-a"));
        assert!(!not_found.contains("local-b"));
    }

    let config_path = root.path().join("config.json");
    let mut config: Value = serde_json::from_str(&fs::read_to_string(&config_path)?)?;
    let global = config
        .as_object_mut()
        .ok_or("config must be an object")?
        .remove("state")
        .ok_or("global state is missing")?;
    fs::write(
        root.path().join("global.json"),
        serde_json::to_string(&global)?,
    )?;
    config["stateFile"] = Value::String("global.json".to_string());
    fs::write(config_path, serde_json::to_string(&config)?)?;
    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        for (page, value) in [("a", "local-a"), ("b", "local-b"), ("global", "global")] {
            let html = fs::read_to_string(root.path().join(format!("dist/{page}/index.html")))?;
            assert!(html.contains(&format!("data-value=\"{value}\"")));
            assert!(html.contains("data-shared=\"global-shared\""));
        }
    }
    Ok(())
}

#[test]
fn cli_rejects_invalid_markdown_state_without_replacing_previous_output() -> TestResult {
    let root = fixture()?;
    for mode in ["all", "content"] {
        fs::write(root.path().join("content/invalid.md"), "# Valid page")?;
        successful_build(root.path(), mode)?;
        let output_path = root.path().join("dist/invalid/index.html");
        let previous_output = fs::read_to_string(&output_path)?;
        for state in ["null", "{site: override}", "{broken: [}"] {
            fs::write(
                root.path().join("content/invalid.md"),
                format!("---\nstate: {state}\n---\n\n# Invalid"),
            )?;
            let output = build(root.path(), mode)?;
            assert!(!output.status.success(), "{mode} accepted {state}");
            let message = String::from_utf8_lossy(&output.stderr);
            assert!(message.contains("invalid.md"), "{message}");
            assert!(
                message.contains("state") || message.contains("YAML"),
                "{message}"
            );
            assert!(message.contains("help:"), "{message}");
            assert_eq!(fs::read_to_string(&output_path)?, previous_output);
        }
    }
    Ok(())
}

#[test]
fn cli_custom_pages_keep_their_state_and_replace_markdown() -> TestResult {
    let root = fixture()?;
    fs::write(
        root.path().join("content/index.md"),
        "---\nstate: invalid-but-overridden\n---\n\nIgnored Markdown",
    )?;
    let path = root.path().join("config.json");
    let mut config: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    config["customPages"] = serde_json::from_str(
        r#"{"/": {
            "html": "<state-example></state-example><p data-custom=\"{{pageData.sample.value}}\" data-title=\"{{site.title}}\"></p>",
            "state": {"sample": {"value": "custom"}, "items": ["custom-item"], "site": "ignored"}
        }}"#,
    )?;
    fs::write(path, serde_json::to_string(&config)?)?;
    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        let html = fs::read_to_string(root.path().join("dist/index.html"))?;
        assert!(html.contains("data-value=\"custom\""));
        assert!(html.contains("data-custom=\"custom\""));
        assert!(html.contains("data-title=\"Canonical docs\""));
        assert!(!html.contains("Ignored Markdown"));
    }
    Ok(())
}
