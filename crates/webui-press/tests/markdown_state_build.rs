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
fn markdown_json_includes_merge_global_then_file_then_inline() -> TestResult {
    let root = fixture()?;
    fs::create_dir_all(root.path().join("target/state"))?;
    fs::write(
        root.path().join("target/state/local.json"),
        r#"{"sample":{"value":"included","globalOnly":true},"items":["included-item"],"shared":"included-shared"}"#,
    )?;
    fs::write(
        root.path().join("content/component.json"),
        r#"{"sample":{"value":"file-only","page":{"allowed":"nested"}}}"#,
    )?;
    fs::write(root.path().join("content/include.md"), format!(
        "---\nstateFile: ../target/state/local.json\nstate: {{sample: {{value: inline}}}}\n---\n{EXAMPLE}"
    ))?;
    fs::write(
        root.path().join("content/file-only.md"),
        format!("---\nstateFile: ./component.json\n---\n{EXAMPLE}"),
    )?;
    fs::write(root.path().join("content/global.md"), EXAMPLE)?;
    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        let included = fs::read_to_string(root.path().join("dist/include/index.html"))?;
        assert!(included.contains("data-value=\"inline\""), "{included}");
        assert!(
            included.contains("data-global-only=\"\""),
            "inline replacement must be shallow"
        );
        assert!(included.contains("data-item=\"included-item\""));
        assert!(included.contains("data-shared=\"included-shared\""));
        let file_only = fs::read_to_string(root.path().join("dist/file-only/index.html"))?;
        assert!(file_only.contains("data-value=\"file-only\""));
        assert!(file_only.contains("data-item=\"global-item\""));
        let global = fs::read_to_string(root.path().join("dist/global/index.html"))?;
        assert!(global.contains("data-value=\"global\""));
        assert!(!global.contains("included-shared"));
        assert!(!fs::read_to_string(root.path().join("dist/404.html"))?.contains("included-shared"));
    }
    Ok(())
}

#[test]
fn invalid_markdown_include_preserves_output_and_reports_source_and_hint() -> TestResult {
    let root = fixture()?;
    fs::write(root.path().join("content/include.md"), "# Valid")?;
    successful_build(root.path(), "content")?;
    let output_path = root.path().join("dist/include/index.html");
    let previous = fs::read_to_string(&output_path)?;
    for (frontmatter, json) in [
        ("stateFile: null", "{}"),
        ("stateFile: []", "{}"),
        ("stateFile: {component: ./data.json}", "{}"),
        ("stateFile: 42", "{}"),
        ("stateFile: ./data.json", "[]"),
        ("stateFile: ./data.json", r#"{"site":"reserved"}"#),
        ("stateFile: ./data.json", "null"),
        ("stateFile: ./data.json", "{"),
        ("stateFile: ./missing.json", "{}"),
    ] {
        fs::write(root.path().join("content/data.json"), json)?;
        fs::write(
            root.path().join("content/include.md"),
            format!("---\n{frontmatter}\n---\n# Invalid"),
        )?;
        let output = build(root.path(), "content")?;
        assert!(!output.status.success(), "{frontmatter}: {json}");
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(
            message.contains("include.md")
                && message.contains("state")
                && message.contains("help:"),
            "{message}"
        );
        assert_eq!(fs::read_to_string(&output_path)?, previous);
    }
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
fn cli_build_preserves_yaml_frontmatter_values_and_aliases() -> TestResult {
    let root = fixture()?;
    fs::write(
        root.path().join("content/yaml.md"),
        concat!(
            "---\n",
            "title: &title YAML compatibility\n",
            "description: >-\n  Folded\n  description\n",
            "defaults: &defaults {value: anchored, items: [one, two]}\n",
            "state:\n",
            "  sample: *defaults\n",
            "  items: [*title, !!str 42, yes, off, 'true', 'null', '007']\n",
            "  enabled: true\n",
            "  disabled: false\n",
            "  optional: null\n",
            "  count: 0x2a\n",
            "  ratio: 1.25\n",
            "  large: 18446744073709551615\n",
            "  negative: -9223372036854775808\n",
            "  literalMerge: {<<: *defaults}\n",
            "  text: |-\n    First line\n    Second line\n",
            "---\n\n",
            "<state-example :sample=\"{{sample}}\" :items=\"{{items}}\"></state-example>\n",
            "<p data-enabled=\"{{enabled}}\" data-disabled=\"{{disabled}}\" ",
            "data-optional=\"{{optional}}\" data-count=\"{{count}}\" data-ratio=\"{{ratio}}\" ",
            "data-large=\"{{large}}\" data-negative=\"{{negative}}\" ",
            "data-merge=\"{{literalMerge.value}}\" ",
            "data-title=\"{{page.title}}\" data-description=\"{{page.description}}\">{{text}}</p>\n",
        ),
    )?;
    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        let html = fs::read_to_string(root.path().join("dist/yaml/index.html"))?;
        for expected in [
            "data-value=\"anchored\"",
            "data-item=\"YAML compatibility\"",
            "data-item=\"42\"",
            "data-item=\"yes\"",
            "data-item=\"off\"",
            "data-item=\"true\"",
            "data-item=\"null\"",
            "data-item=\"007\"",
            "data-enabled=\"true\"",
            "data-disabled=\"false\"",
            "data-optional=\"\"",
            "data-count=\"42\"",
            "data-ratio=\"1.25\"",
            "data-large=\"18446744073709551615\"",
            "data-negative=\"-9223372036854775808\"",
            "data-merge=\"\"",
            "data-title=\"YAML compatibility\"",
            "data-description=\"Folded description\"",
            "First line\nSecond line",
        ] {
            assert!(
                html.contains(expected),
                "{mode}: missing {expected}: {html}"
            );
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
        for state in [
            "null",
            "{site: override}",
            "{broken: [}",
            "{nested: {1: value}}",
            "{nested: {true: value}}",
            "{nested: {value: .inf}}",
            "{nested: {value: -.inf}}",
            "{nested: [1, .nan]}",
            "{nested: !example value}",
            "{nested: !example [one, two]}",
            "!example {value: tagged}",
            "{nested: {!example key: value}}",
            "{nested: {value: first, value: second}}",
            "{value: first}\nstate: {value: second}",
            "{}\ntitle: First\ntitle: Second",
            "{value: *missing}",
            "{example: {id: 18446744073709551617}}",
            "{example: {id: -9223372036854775809}}",
            "{example: !str value}",
            "{example: !int 42}",
            "{example: !time value}",
        ] {
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
fn cli_rejects_duplicate_metadata_keys_without_replacing_previous_output() -> TestResult {
    let root = fixture()?;
    for mode in ["all", "content"] {
        fs::write(root.path().join("content/metadata.md"), "# Valid page")?;
        successful_build(root.path(), mode)?;
        let output_path = root.path().join("dist/metadata/index.html");
        let previous_output = fs::read_to_string(&output_path)?;
        for metadata in [
            "42: first\n42: second",
            "true: first\ntrue: second",
            "ignored: {42: first, 42: second}",
            "ignored: {true: first, true: second}",
        ] {
            fs::write(
                root.path().join("content/metadata.md"),
                format!("---\n{metadata}\nstate: {{count: 2}}\n---\n# Invalid"),
            )?;
            let output = build(root.path(), mode)?;
            assert!(!output.status.success(), "{mode} accepted {metadata}");
            let message = String::from_utf8_lossy(&output.stderr);
            assert!(message.contains("metadata.md"), "{message}");
            assert!(
                message.contains("duplicate") && message.contains("help:"),
                "{message}"
            );
            assert_eq!(fs::read_to_string(&output_path)?, previous_output);
        }
        fs::write(
            root.path().join("content/metadata.md"),
            concat!(
                "---\n42: number\n'42': string\ntrue: boolean\n'true': text\n",
                "ignored: {42: number, '42': string, true: boolean, 'true': text}\n",
                "state: {count: 2}\n---\n# Typed keys\n"
            ),
        )?;
        successful_build(root.path(), mode)?;
    }
    Ok(())
}

#[test]
fn cli_preserves_frontmatter_scalar_classification() -> TestResult {
    let root = fixture()?;
    fs::write(
        root.path().join("content/scalars.md"),
        concat!(
            "---\ntitle: 00123\ndescription: 1_000\n",
            "state:\n  values:\n    leading: 00123\n    underscored: 1_000\n",
            "    00123: leading-key\n    1_000: underscored-key\n",
            "    maximum: 18446744073709551615\n    minimum: -9223372036854775808\n",
            "    explicit: !!str 42\n---\n",
            "<p data-title=\"{{page.title}}\" data-description=\"{{page.description}}\" ",
            "data-leading=\"{{values.leading}}\" data-underscored=\"{{values.underscored}}\" ",
            "data-max=\"{{values.maximum}}\" data-min=\"{{values.minimum}}\" ",
            "data-explicit=\"{{values.explicit}}\">Scalars</p>\n"
        ),
    )?;
    for mode in ["all", "content"] {
        successful_build(root.path(), mode)?;
        let html = fs::read_to_string(root.path().join("dist/scalars/index.html"))?;
        for expected in [
            "data-title=\"00123\"",
            "data-description=\"1_000\"",
            "data-leading=\"00123\"",
            "data-underscored=\"1_000\"",
            "data-max=\"18446744073709551615\"",
            "data-min=\"-9223372036854775808\"",
            "data-explicit=\"42\"",
        ] {
            assert!(
                html.contains(expected),
                "{mode}: missing {expected}: {html}"
            );
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
