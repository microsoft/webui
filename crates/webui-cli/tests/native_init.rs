// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::Value;

fn generate(root: &Path, server: &str) -> Result<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format", "json", "init"])
        .arg(root)
        .args(["--server", server, "--no-interactive"])
        .env("PATH", "")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn all_native_hosts_generate_offline_without_javascript_build_scripts() -> Result<()> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    for (server, native) in [
        ("rust", "Cargo.toml"),
        ("go", "go.mod"),
        ("c", "CMakeLists.txt"),
        ("cpp", "CMakeLists.txt"),
        ("dotnet", "App.csproj"),
        ("node", "server/main.mjs"),
        ("bun", "server/main.ts"),
    ] {
        let root = temp.path().join(format!("native-{server}"));
        let report = generate(&root, server)?;
        assert!(report.get("frontend").is_none());
        assert_eq!(report["dependencies"], "skipped");
        assert_eq!(
            report["next_commands"]
                .as_array()
                .context("commands")?
                .last(),
            Some(&Value::from("webui app start"))
        );
        assert!(root.join(native).is_file());
        assert!(root.join("web/index.html").is_file());
        assert!(root.join("web/app-counter/app-counter.ts").is_file());
        assert!(root.join("webui.toml").is_file());
        assert!(!root.join("scripts").exists());
        assert!(!root.join("node_modules").exists());
        if matches!(server, "node" | "bun") {
            let package: Value = serde_json::from_slice(&fs::read(root.join("package.json"))?)?;
            assert!(package.get("scripts").is_none());
            assert!(package.get("devDependencies").is_none());
            assert!(package["dependencies"]
                .get("@microsoft/webui-framework")
                .is_none());
            assert_eq!(
                report["package_manager"],
                if server == "bun" { "bun" } else { "npm" }
            );
            let source = fs::read_to_string(root.join(native))?;
            assert!(source.contains("from './http.mjs'"));
        } else {
            assert!(!root.join("package.json").exists());
            assert_eq!(report["package_manager"], Value::Null);
            assert_eq!(report["next_commands"][0], "webui app check");
        }
        for file in report["files"].as_array().context("files")? {
            let file = file.as_str().context("path")?;
            let content = fs::read_to_string(root.join(file))?;
            for marker in [
                "__WEBUI_VERSION__",
                "__APP_NAME__",
                "__SERVER__",
                "__APP_ROOT_DEFAULT__",
            ] {
                assert!(!content.contains(marker), "{server} {file} {marker}");
            }
        }
    }
    Ok(())
}

#[test]
fn frontend_selector_is_not_advertised_or_accepted() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("removed-frontend");
    let help = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["init", "--help"])
        .output()?;
    assert!(help.status.success());
    assert!(!String::from_utf8(help.stdout)?.contains("--frontend"));
    for value in ["native", "node"] {
        let rejected = Command::new(env!("CARGO_BIN_EXE_webui"))
            .args(["init", "--frontend", value])
            .arg(&root)
            .output()?;
        assert_eq!(rejected.status.code(), Some(2));
        assert!(String::from_utf8(rejected.stderr)?.contains("unexpected argument '--frontend'"));
        assert!(!root.exists());
    }
    Ok(())
}

#[test]
fn interactive_native_build_needs_no_node_and_failed_rebuild_preserves_the_generation() -> Result<()>
{
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path().join("native-counter");
    generate(&root, "rust")?;
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_webui"))
            .args(["app", "build", "--frontend-only", "--project"])
            .arg(&root)
            .env("PATH", "")
            .env_remove("WEBUI_TOOLCHAIN_DIR")
            .output()
    };
    let output = build()?;
    anyhow::ensure!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let protocol = fs::read(root.join("dist/protocol.bin"))?;
    let javascript = fs::read(root.join("dist/public/index.js"))?;
    assert!(!protocol.is_empty());
    assert!(javascript.is_ascii());
    assert!(!root.join("node_modules").exists());
    let component = root.join("web/app-counter/app-counter.ts");
    let original = fs::read(&component)?;
    fs::write(&component, "export class {")?;
    let rejected = build()?;
    assert!(!rejected.status.success());
    assert_eq!(protocol, fs::read(root.join("dist/protocol.bin"))?);
    assert_eq!(javascript, fs::read(root.join("dist/public/index.js"))?);
    fs::write(&component, original)?;
    let rebuilt = build()?;
    anyhow::ensure!(
        rebuilt.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuilt.stderr)
    );
    assert_eq!(
        webui_protocol::WebUIProtocol::from_protobuf(&protocol)?,
        webui_protocol::WebUIProtocol::from_protobuf(&fs::read(root.join("dist/protocol.bin"))?)?
    );
    assert_eq!(javascript, fs::read(root.join("dist/public/index.js"))?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn redirected_generated_modules_never_overwrite_an_external_file() -> Result<()> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path().join("native-lazy");
    generate(&root, "rust")?;
    let home = root.join("web/home-page/home-page.html");
    fs::write(
        &home,
        fs::read_to_string(&home)?.replace(
            "<app-counter></app-counter>",
            "<if condition=\"shown\" lazy><app-counter></app-counter></if>",
        ),
    )?;
    let outside = temp.path().join("outside");
    fs::create_dir(&outside)?;
    let external = outside.join("lazy.webui.js");
    fs::write(&external, "preserve external content")?;
    fs::create_dir(root.join(".webui"))?;
    std::os::unix::fs::symlink(&outside, root.join(".webui/modules"))?;
    let rejected = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["app", "build", "--frontend-only", "--project"])
        .arg(&root)
        .env("PATH", "")
        .output()?;
    assert_eq!(fs::read_to_string(external)?, "preserve external content");
    assert!(!rejected.status.success());
    assert_eq!(fs::read_dir(outside)?.count(), 1);
    Ok(())
}

#[test]
fn native_reference_installation_is_explicit_offline_and_never_uses_npx() -> Result<()> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path().join("offline-reference");
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format", "json", "init"])
        .arg(&root)
        .args(["--ai-skill", "--no-interactive"])
        .env("PATH", "")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["ai_skill"], "installed");
    assert!(root
        .join(".github/skills/webui-reference/SKILL.md")
        .is_file());
    assert!(
        fs::read_to_string(root.join(".github/skills/webui-reference/SKILL.md"))?
            .contains("webui reference --path")
    );
    assert!(!root.join("package.json").exists());
    let reference = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format", "json", "reference", "--path", "--project"])
        .arg(&root)
        .env("PATH", "")
        .output()?;
    assert!(reference.status.success());
    let reference: Value = serde_json::from_slice(&reference.stdout)?;
    assert!(Path::new(reference["path"].as_str().context("reference path")?).is_file());
    Ok(())
}

#[test]
fn rejects_package_manager_selection_for_a_package_free_native_host_before_writing() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("no-packages");
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format", "json", "init"])
        .arg(&root)
        .args(["--package-manager", "npm", "--no-interactive"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(error["code"], "init-invalid-options");
    assert!(!root.exists());
    Ok(())
}
