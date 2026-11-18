// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::process::Command;

use serde_json::{Map, Value};
use tempfile::TempDir;
use webui_docs::build::{build_docs_with_cache, BuildCache};
use webui_docs::DocsConfig;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture() -> TestResult<(TempDir, DocsConfig)> {
    let fixture = tempfile::tempdir()?;
    let root = fixture.path();
    fs::create_dir_all(root.join("content"))?;
    fs::create_dir_all(root.join("template"))?;
    fs::write(
        root.join("template/index.html"),
        "<!doctype html><html><body>{{{page.content}}}</body></html>",
    )?;
    fs::write(
        root.join("content/index.md"),
        "# Native advisory\n\n<script type=\"module\" bundle src=\"entry.ts\"></script>",
    )?;
    fs::write(
        root.join("entry.ts"),
        concat!(
            "import { value } from './shared.ts';\n",
            "globalThis.staticValue = value;\n",
            "globalThis.dynamicValue = () => import('./shared.ts');\n",
        ),
    )?;
    fs::write(root.join("shared.ts"), "export const value = 42;")?;
    let config = Value::Object(Map::from_iter([
        (
            "site".to_string(),
            Value::Object(Map::from_iter([(
                "title".to_string(),
                Value::String("Native warnings".to_string()),
            )])),
        ),
        ("basePath".to_string(), Value::String("/".to_string())),
        (
            "contentDir".to_string(),
            Value::String(root.join("content").to_string_lossy().into_owned()),
        ),
        (
            "outDir".to_string(),
            Value::String(root.join("dist").to_string_lossy().into_owned()),
        ),
        ("nav".to_string(), Value::Array(Vec::new())),
        ("sidebar".to_string(), Value::Array(Vec::new())),
    ]));
    fs::write(root.join("config.json"), serde_json::to_vec(&config)?)?;
    Ok((fixture, serde_json::from_value(config)?))
}

#[test]
fn native_advisories_remain_typed_and_plain_across_quiet_rebuilds() -> TestResult {
    let (fixture, config) = fixture()?;
    let root = fixture.path();
    let mut cache = BuildCache::new();
    for rebuild in [false, true] {
        if rebuild {
            cache.set_dev_rebuild();
        }
        let stats = build_docs_with_cache(&config, root, &root.join("template"), &mut cache)?;
        assert_eq!(stats.warnings.len(), 1);
        assert_eq!(stats.warnings[0].code, "INEFFECTIVE_DYNAMIC_IMPORT");
        assert!(stats.warnings[0].message.contains("shared.ts"));
        assert!(!stats.warnings[0].message.contains('\x1b'));
        let output = fs::read_to_string(root.join("dist/assets/page-0.js"))?;
        assert!(output.contains("staticValue"));
        assert!(output.contains("dynamicValue"));
    }

    fs::write(
        root.join("entry.ts"),
        "import { value } from './shared.ts'; globalThis.staticValue = value;",
    )?;
    let clean = build_docs_with_cache(&config, root, &root.join("template"), &mut cache)?;
    assert!(clean.warnings.is_empty(), "advisories belong to one build");
    Ok(())
}

#[test]
fn cli_presents_nonfatal_native_advisories_without_losing_outputs() -> TestResult {
    let (fixture, _) = fixture()?;
    let root = fixture.path();
    let output = Command::new(env!("CARGO_BIN_EXE_webui-press"))
        .args(["build", "--config"])
        .arg(root.join("config.json"))
        .arg("--template")
        .arg(root.join("template"))
        .current_dir(root)
        .env("CLICOLOR_FORCE", "0")
        .output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty());
    assert_eq!(stderr.matches("build warning:").count(), 1, "{stderr}");
    assert!(stderr.contains("INEFFECTIVE_DYNAMIC_IMPORT"), "{stderr}");
    assert!(stderr.contains("shared.ts"), "{stderr}");
    assert!(!stderr.contains('\x1b'));
    assert!(root.join("dist/index.html").is_file());
    assert!(root.join("dist/assets/page-0.js").is_file());
    Ok(())
}
