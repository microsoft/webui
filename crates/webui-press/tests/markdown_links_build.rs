// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::process::Command;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn native_build_maps_markdown_sources_to_existing_page_routes() -> TestResult {
    let root = tempfile::tempdir()?;
    for directory in ["content/guide", "content/button", "content/ai", "template"] {
        fs::create_dir_all(root.path().join(directory))?;
    }
    fs::write(
        root.path().join("template/index.html"),
        "<!DOCTYPE html><html><head><title>{{page.title}}</title></head><body>{{{page.content}}}</body></html>",
    )?;
    let source = concat!(
        "# Home\n\n",
        "[Install](guide/install.md?mode=full#setup)\n\n",
        "[Root](/guide/install.md#setup)\n\n",
        "[Prefixed](/project/guide/install.md#setup)\n\n",
        "[Dotted](./guide/../guide/install.md#setup)\n\n",
        "[Index](guide/index.md)\n\n",
        "[Repeated](button/button.md)\n\n",
        "[Mapped](ai/SKILL.md#rules)\n\n",
        "[Reference][install]\n\n[install]: guide/install.md#setup\n\n",
        "[Missing](guide/missing.md#missing)\n\n",
        "[Hidden](.hidden.md)\n\n",
        "[Custom source](custom/index.md)\n\n",
        "[Route](guide/install/#setup)\n\n",
        "[Asset](guide/data.json?download=1)\n\n",
        "[External](https://example.com/guide/install.md#setup)\n\n",
        "[Protocol relative](//example.com/guide/install.md)\n\n",
        "![Image](guide/install.md)\n",
    );
    fs::write(root.path().join("content/index.md"), source)?;
    fs::write(root.path().join("content/.hidden.md"), "# Hidden")?;
    fs::write(
        root.path().join("content/guide/install.md"),
        "# Install\n\n## Setup\n\n[Sibling](./sibling.md)\n\n[Home](../index.md)\n\n[Self](?view=all#setup)\n",
    )?;
    fs::write(root.path().join("content/guide/sibling.md"), "# Sibling")?;
    fs::write(root.path().join("content/guide/index.md"), "# Guide")?;
    fs::write(root.path().join("content/button/button.md"), "# Button")?;
    fs::write(
        root.path().join("content/ai/SKILL.md"),
        "# Skill\n\n## Rules\n\n[Source sibling](../guide/install.md#setup)",
    )?;

    for base in ["/", "/project/"] {
        fs::write(
            root.path().join("config.json"),
            serde_json::to_string(&serde_json::json!({
                "site": {"title": "Source links"},
                "basePath": base,
                "contentDir": "content",
                "outDir": "dist",
                "nav": [{"text": "Skill", "link": "/agents#rules", "source": "ai/SKILL.md"}],
                "customPages": {"/custom": "<p>Custom page</p>"},
                "sidebar": []
            }))?,
        )?;
        let output = Command::new(env!("CARGO_BIN_EXE_webui-press"))
            .args(["build", "--config", "config.json", "--template"])
            .arg(root.path().join("template"))
            .current_dir(root.path())
            .output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let home = fs::read_to_string(root.path().join("dist/index.html"))?;
        for (label, destination, output_file) in [
            (
                "Install",
                "guide/install?mode=full#setup",
                "guide/install/index.html",
            ),
            ("Root", "guide/install#setup", "guide/install/index.html"),
            ("Dotted", "guide/install#setup", "guide/install/index.html"),
            (
                "Reference",
                "guide/install#setup",
                "guide/install/index.html",
            ),
            ("Index", "guide", "guide/index.html"),
            ("Repeated", "button", "button/index.html"),
            ("Mapped", "agents#rules", "agents/index.html"),
        ] {
            let href = format!("href=\"{base}{destination}\">{label}</a>");
            assert!(home.contains(&href), "missing {href}: {home}");
            assert!(root.path().join("dist").join(output_file).is_file());
        }
        for destination in [
            "guide/missing.md#missing",
            ".hidden.md",
            "custom/index.md",
            "guide/install/#setup",
            "guide/data.json?download=1",
        ] {
            assert!(home.contains(&format!("href=\"{base}{destination}\"")));
        }
        assert!(home.contains("href=\"https://example.com/guide/install.md#setup\""));
        assert!(home.contains("href=\"//example.com/guide/install.md\""));
        assert!(home.contains(&format!("src=\"{base}guide/install.md\"")));
        if base == "/project/" {
            assert!(home.contains("href=\"/project/guide/install#setup\">Prefixed</a>"));
        }
        let install = fs::read_to_string(root.path().join("dist/guide/install/index.html"))?;
        for (label, destination) in [
            ("Sibling", "guide/sibling"),
            ("Home", ""),
            ("Self", "guide/install/?view=all#setup"),
        ] {
            assert!(
                install.contains(&format!("href=\"{base}{destination}\">{label}</a>")),
                "{install}"
            );
        }
        assert!(root.path().join("dist/guide/sibling/index.html").is_file());
        let mapped = fs::read_to_string(root.path().join("dist/agents/index.html"))?;
        assert!(mapped.contains(&format!("href=\"{base}guide/install#setup\"")));
        assert_eq!(
            fs::read_to_string(root.path().join("content/index.md"))?,
            source
        );
    }
    Ok(())
}
