// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;
use tempfile::TempDir;

fn init(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_webui"))
        .arg("--format=json")
        .arg("init")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running webui init")
}

fn temp() -> Result<TempDir> {
    // macOS's system temporary directory can have a symlink ancestor.
    let root = std::env::temp_dir().canonicalize()?;
    Ok(tempfile::tempdir_in(root)?)
}

#[test]
fn all_hosts_generate_offline_with_one_shared_frontend() -> Result<()> {
    let temp = temp()?;
    let mut entry = None;
    for (server, host_file) in [
        ("rust", "server/src/main.rs"),
        ("dotnet", "server/Program.cs"),
        ("c", "server/main.c"),
        ("cpp", "server/main.cpp"),
        ("node", "server/main.mjs"),
        ("bun", "server/main.ts"),
        ("go", "server/main.go"),
    ] {
        let root = temp.path().join(format!("app-{server}"));
        let output = init(&root, &["--server", server, "--no-interactive"])?;
        anyhow::ensure!(
            output.status.success(),
            "init {server}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(report["server"], server);
        assert_eq!(report["dependencies"], "skipped");
        assert_eq!(report["ai_skill"], "skipped");
        let files = report["files"].as_array().context("files array")?;
        assert!(files.iter().any(|file| file == "src/index.html"));
        assert!(
            files.iter().any(|file| file == host_file),
            "{server} host missing"
        );
        assert!(files.iter().any(|file| file == "README.md"));
        if matches!(server, "c" | "cpp") {
            assert!(files.iter().any(|file| file == "server/host.h"));
            assert!(files.iter().any(|file| file == "server/native.cmake"));
        }
        if server == "go" {
            assert!(!files.iter().any(|file| file == "server/webui.go"));
            let source = fs::read_to_string(root.join("server/main.go"))?;
            assert!(source.contains(concat!(
                "github.com/microsoft/webui/go/v",
                env!("CARGO_PKG_VERSION_MAJOR")
            )));
            assert!(!source.contains("import \"C\""));
            let module = fs::read_to_string(root.join("server/go.mod"))?;
            assert!(module.contains(&format!(" v{}", env!("CARGO_PKG_VERSION"))));
        }
        assert!(!root.join("node_modules").exists());
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("package.json"))?)?;
        assert_eq!(manifest["name"], format!("app-{server}"));
        assert_eq!(manifest["webui"]["server"], server);
        assert_eq!(
            manifest["dependencies"]["@microsoft/webui"],
            env!("CARGO_PKG_VERSION")
        );
        let html = fs::read_to_string(root.join("src/index.html"))?;
        if let Some(previous) = &entry {
            assert_eq!(previous, &html);
        }
        entry = Some(html);
        for file in files {
            let path = file.as_str().context("file path")?;
            let text = fs::read_to_string(root.join(path))?;
            assert!(!text.contains("__WEBUI_VERSION__"), "{path}");
            assert!(!text.contains("__APP_NAME__"), "{path}");
            assert!(!text.contains("__RUST_VERSION__"), "{path}");
            assert!(!text.contains("__WEBUI_GO_MODULE__"), "{path}");
            assert!(!text.contains("__SERVER__"), "{path}");
            assert!(!text.contains("workspace:*"), "{path}");
        }
    }
    Ok(())
}

#[test]
fn rejects_conflicts_before_writing_and_preserves_git_metadata() -> Result<()> {
    let temp = temp()?;
    let root = temp.path().join("existing-app");
    fs::create_dir(&root)?;
    fs::write(root.join("keep.txt"), "important")?;
    let output = init(&root, &[])?;
    assert_eq!(output.status.code(), Some(73));
    let error: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(error["code"], "init-directory-not-empty");
    assert_eq!(fs::read_to_string(root.join("keep.txt"))?, "important");
    assert_eq!(fs::read_dir(&root)?.count(), 1);
    fs::remove_file(root.join("keep.txt"))?;
    fs::write(root.join(".git"), "gitdir: preserved")?;
    let output = init(&root, &[])?;
    assert!(output.status.success());
    assert_eq!(fs::read_to_string(root.join(".git"))?, "gitdir: preserved");
    Ok(())
}

#[test]
fn normalizes_names_without_injecting_paths_into_project_files() -> Result<()> {
    let temp = temp()?;
    let root = temp.path().join("My Great App");
    let output = init(&root, &["--package-manager", "pnpm"])?;
    assert!(output.status.success());
    let manifest: Value = serde_json::from_str(&fs::read_to_string(root.join("package.json"))?)?;
    assert_eq!(manifest["name"], "my-great-app");
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["next_commands"][0], "pnpm install");
    let invalid = temp.path().join("app;echo-injected");
    let output = init(&invalid, &[])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(!invalid.exists());
    Ok(())
}

#[test]
fn generation_is_deterministic_and_does_not_prompt_without_tty() -> Result<()> {
    let temp = temp()?;
    let first = temp.path().join("a/same-app");
    let second = temp.path().join("b/same-app");
    let one = init(&first, &[])?;
    let two = init(&second, &[])?;
    assert!(one.status.success() && two.status.success());
    let report: Value = serde_json::from_slice(&one.stdout)?;
    for file in report["files"].as_array().context("file list")? {
        let file = file.as_str().context("file path")?;
        assert_eq!(fs::read(first.join(file))?, fs::read(second.join(file))?);
    }

    Ok(())
}

#[test]
fn published_cargo_source_contains_the_default_rust_starter() -> Result<()> {
    let output = Command::new(env!("CARGO"))
        .args(["package", "--list", "--allow-dirty", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files = String::from_utf8(output.stdout)?;
    for expected in [
        "src/commands/init/templates/rust/server/Cargo.toml.template",
        "src/commands/init/templates/rust/server/src/main.rs",
    ] {
        assert!(files.lines().any(|file| file == expected), "{expected}");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn refuses_symlinks_and_parent_traversal() -> Result<()> {
    let temp = temp()?;
    let target = temp.path().join("real-app");
    fs::create_dir(&target)?;
    let link = temp.path().join("linked-app");
    std::os::unix::fs::symlink(&target, &link)?;
    for path in [link, target.join("../other-app")] {
        let output = init(&path, &[])?;
        assert_eq!(output.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(error["code"], "init-unsafe-destination");
    }
    assert_eq!(fs::read_dir(&target)?.count(), 0);
    Ok(())
}

#[cfg(unix)]
#[test]
fn installers_are_opt_in_and_keep_machine_output_clean() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = temp()?;
    let bin = temp.path().join("bin");
    fs::create_dir(&bin)?;
    for program in ["npm", "npx"] {
        let file = bin.join(program);
        fs::write(&file, "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$@\"\n")?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
    }
    let root = temp.path().join("installed-app");
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format=json", "init"])
        .arg(&root)
        .args(["--install", "--ai-skill", "--no-interactive"])
        .env("PATH", &bin)
        .stdin(Stdio::null())
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["dependencies"], "installed");
    assert_eq!(report["ai_skill"], "installed");
    assert_eq!(report["next_commands"][0], "npm run dev");
    let commands = String::from_utf8(output.stderr)?;
    assert!(commands.contains(&root.display().to_string()));
    assert!(commands.contains("webui-reference"));
    assert!(!commands.contains("--global"));
    assert!(commands.contains("--yes"));

    fs::write(bin.join("npm"), "#!/bin/sh\nexit 23\n")?;
    let failed = temp.path().join("failed-install");
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format=json", "init"])
        .arg(&failed)
        .arg("--install")
        .env("PATH", &bin)
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let records = String::from_utf8(output.stdout)?;
    let records: Vec<Value> = records
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["dependencies"], "failed");
    assert_eq!(records[1]["code"], "init-install-failed");
    assert!(failed.join("package.json").is_file());
    Ok(())
}
