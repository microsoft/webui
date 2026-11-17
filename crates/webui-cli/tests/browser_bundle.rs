// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

async fn changed(path: &Path, previous: Option<&[u8]>) -> Result<Vec<u8>> {
    for _ in 0..200 {
        match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() && previous != Some(bytes.as_slice()) => {
                return Ok(bytes)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("native browser output did not change: {}", path.display())
}

#[tokio::test]
async fn native_bundle_watch_rebuilds_recovers_and_stops_without_node() -> Result<()> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path();
    for directory in ["src", "cfg", "build", "dist"] {
        fs::create_dir(root.join(directory))?;
    }
    let entry = root.join("src/index.ts");
    fs::write(&entry, "export const message = 'initial';")?;
    fs::write(
        root.join("src/first.ts"),
        "export const marker = 'first-choice';",
    )?;
    fs::write(
        root.join("src/second.ts"),
        "export const marker = 'second-choice';",
    )?;
    let first_config = r#"{"compilerOptions":{"paths":{"selected":["../src/first.ts"]}}}"#;
    let second_config = r#"{"compilerOptions":{"paths":{"selected":["../src/second.ts"]}}}"#;
    let inherited_config = r#"{"extends":"../dist/base.json"}"#;
    fs::write(root.join("dist/base.json"), first_config)?;
    fs::write(root.join("dist/config.json"), r#"{"choice":"json-first"}"#)?;
    fs::write(root.join("cfg/tsconfig.json"), inherited_config)?;
    let mut entries = Map::new();
    entries.insert("index".into(), Value::from("src/index.ts"));
    let mut options = Map::new();
    options.insert("project".into(), serde_json::to_value(root)?);
    options.insert("entries".into(), Value::Object(entries));
    options.insert("outDir".into(), Value::from("dist"));
    options.insert("development".into(), Value::Bool(true));
    options.insert("minify".into(), Value::Bool(false));
    options.insert("tsconfig".into(), Value::from("cfg/tsconfig.json"));
    let config = root.join("build/browser.json");
    fs::write(&config, serde_json::to_vec(&options)?)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["bundle", "--config"])
        .arg(&config)
        .args(["--watch", "--control-stdin", "--manifest"])
        .arg(root.join("proof.json"))
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child.stdin.take().context("native bundle control")?;
    let stderr = child.stderr.take().context("native bundle errors")?;
    let (errors, mut error_rx) = tokio::sync::mpsc::channel(16);
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Some(line) = lines.next_line().await? {
            if line.contains("browser build failed") && errors.send(line).await.is_err() {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    });
    let result = async {
        let output = root.join("dist/index.js");
        let first = changed(&output, None).await?;
        let modified = fs::metadata(&output)?.modified()?;
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            fs::metadata(&output)?.modified()?,
            modified,
            "publishing a custom proof must not rebuild without source edits"
        );
        fs::write(&entry, "export const message = 'updated';")?;
        let second = changed(&output, Some(&first)).await?;
        fs::write(&entry, "export const = ;")?;
        tokio::time::timeout(Duration::from_secs(10), error_rx.recv())
            .await?
            .context("native compiler did not report invalid input")?;
        assert_eq!(fs::read(&output)?, second);
        fs::write(&entry, "export const message = 'recovered';")?;
        let recovered = changed(&output, Some(&second)).await?;
        fs::write(&entry, "export { marker } from 'selected';")?;
        let first = changed(&output, Some(&recovered)).await?;
        assert!(String::from_utf8_lossy(&first).contains("first-choice"));
        fs::write(root.join("cfg/tsconfig.json"), second_config)?;
        let second = changed(&output, Some(&first)).await?;
        assert!(String::from_utf8_lossy(&second).contains("second-choice"));
        fs::write(root.join("cfg/tsconfig.json"), inherited_config)?;
        let first = changed(&output, Some(&second)).await?;
        fs::write(root.join("dist/base.json"), second_config)?;
        let inherited = changed(&output, Some(&first)).await?;
        assert!(String::from_utf8_lossy(&inherited).contains("second-choice"));
        fs::write(
            &entry,
            "import data from '../dist/config.json'; export const marker = data.choice;",
        )?;
        let json = changed(&output, Some(&inherited)).await?;
        assert!(String::from_utf8_lossy(&json).contains("json-first"));
        fs::write(root.join("dist/config.json"), r#"{"choice":"json-second"}"#)?;
        let json_updated = changed(&output, Some(&json)).await?;
        assert!(String::from_utf8_lossy(&json_updated).contains("json-second"));
        input.write_all(b"S").await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    drop(input);
    let status = tokio::time::timeout(Duration::from_secs(16), child.wait()).await??;
    reader.await??;
    result?;
    assert!(status.success(), "{status}");
    Ok(())
}

#[test]
fn native_bundle_cannot_replace_its_configuration() -> Result<()> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path();
    fs::write(root.join("index.ts"), "export const message = 'safe';")?;
    let mut entries = Map::new();
    entries.insert("index".into(), Value::from("index.ts"));
    let mut options = Map::new();
    options.insert("project".into(), serde_json::to_value(root)?);
    options.insert("entries".into(), Value::Object(entries));
    options.insert("outDir".into(), Value::from("dist"));
    let config = root.join("browser.json");
    let original = serde_json::to_vec(&options)?;
    fs::write(&config, &original)?;
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["bundle", "--config"])
        .arg(&config)
        .arg("--manifest")
        .arg(&config)
        .env("PATH", "")
        .output()?;
    assert!(!result.status.success());
    assert_eq!(fs::read(config)?, original);
    Ok(())
}
