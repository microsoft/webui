// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::Value;

struct Fixture {
    root: tempfile::TempDir,
    cli: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let cli = root.path().join("crates/webui-cli");
        fs::create_dir_all(&cli)?;
        let fixture = Self { root, cli };
        for (path, contents) in [
            (
                "Cargo.toml",
                "[package]\nname = \"resource-fixture\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\
                 [build-dependencies]\nserde_json = \"1\"\n",
            ),
            ("src/lib.rs", ""),
            ("src/commands/init/templates/starter.txt", ""),
            ("build.rs", include_str!("../build.rs")),
            (
                "resources/build_support.rs",
                include_str!("../resources/build_support.rs"),
            ),
        ] {
            write(&fixture.cli.join(path), contents)?;
        }
        Ok(fixture)
    }

    fn build(&self) -> Result<(PathBuf, bool)> {
        let output = Command::new(env!("CARGO"))
            .args([
                "build",
                "--offline",
                "--message-format=json",
                "--manifest-path",
            ])
            .arg(self.cli.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(self.root.path().join("target"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "fixture build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let messages: Vec<Value> = String::from_utf8(output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let artifact = messages
            .iter()
            .find(|message| {
                message["reason"] == "compiler-artifact"
                    && message["target"]["name"] == "resource_fixture"
            })
            .context("fixture Cargo artifact")?;
        let fresh = artifact["fresh"]
            .as_bool()
            .context("fixture build freshness")?;
        let out = messages
            .iter()
            .find(|message| {
                message["reason"] == "build-script-executed"
                    && message["package_id"] == artifact["package_id"]
            })
            .and_then(|message| message["out_dir"].as_str())
            .context("fixture build script's output directory")?;
        Ok((Path::new(out).join("native-assets"), fresh))
    }

    fn verify_invalidation(&self, additions: &[(PathBuf, &str)]) -> Result<()> {
        let (assets, fresh) = self.build()?;
        assert!(!fresh);
        write(&self.root.path().join("unrelated.txt"), "not a resource")?;
        assert!(self.build()?.1, "an unrelated edit invalidated the build");
        for (source, target) in additions {
            write(source, "new resource")?;
            assert!(!self.build()?.1, "adding {source:?} did not rerun Cargo");
            assert_eq!(fs::read_to_string(assets.join(target))?, "new resource");
            fs::remove_file(source)?;
            assert!(!self.build()?.1, "removing {source:?} did not rerun Cargo");
            assert!(!assets.join(target).exists(), "stale staged {target}");
        }
        Ok(())
    }
}

fn write(path: &Path, contents: &str) -> Result<()> {
    fs::create_dir_all(path.parent().context("resource parent")?)?;
    fs::write(path, contents)?;
    Ok(())
}

const FRAMEWORK: &str = r#"{"name":"@microsoft/webui-framework","version":"1.0.0","exports":{}}"#;

#[test]
fn cargo_restages_checkout_tree_additions_and_removals() -> Result<()> {
    let fixture = Fixture::new()?;
    let root = fixture.root.path();
    for (path, contents) in [
        ("packages/webui-framework/package.json", FRAMEWORK),
        ("packages/webui-framework/src/index.ts", ""),
        ("docs/ai.md", ""),
        ("ai/SKILL.md", ""),
        ("LICENSE", ""),
    ] {
        write(&root.join(path), contents)?;
    }
    fixture.verify_invalidation(&[
        (
            root.join("packages/webui-framework/src/browser/new.ts"),
            "framework/src/browser/new.ts",
        ),
        (
            root.join("docs/reference/new.md"),
            "reference/docs/reference/new.md",
        ),
    ])
}

#[test]
fn cargo_restages_packaged_asset_additions_and_removals() -> Result<()> {
    let fixture = Fixture::new()?;
    let assets = fixture.cli.join("assets");
    write(&assets.join("framework/package.json"), FRAMEWORK)?;
    fixture.verify_invalidation(&[(assets.join("reference/new.md"), "reference/new.md")])
}
