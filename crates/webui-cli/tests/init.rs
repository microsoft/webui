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

#[cfg(windows)]
#[test]
fn initializes_drive_and_verbatim_destinations_without_inspecting_bare_prefixes() -> Result<()> {
    let temp = temp()?;
    let canonical = temp.path().canonicalize()?;
    let ordinary = canonical.to_str().context("temporary directory is UTF-8")?;
    let ordinary = if let Some(unc) = ordinary.strip_prefix(r"\\?\UNC\") {
        std::path::PathBuf::from(format!(r"\\{unc}"))
    } else {
        std::path::PathBuf::from(
            ordinary
                .strip_prefix(r"\\?\")
                .context("canonical temporary directory has a verbatim prefix")?,
        )
    };
    for (parent, name) in [(ordinary, "drive-app"), (canonical, "verbatim-app")] {
        let root = parent.join("new-parent").join(name);
        let output = init(&root, &["--no-interactive"])?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(root.join("server/Cargo.toml").is_file());
        assert!(root.join("src/index.html").is_file());
        let conflict = init(&root, &[])?;
        assert_eq!(conflict.status.code(), Some(73));
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_cmd_installers_keep_the_project_cwd_and_report_failures() -> Result<()> {
    let temp = temp()?;
    let bin = temp.path().join("bin");
    fs::create_dir(&bin)?;
    for program in ["npm", "npx"] {
        fs::write(
            bin.join(format!("{program}.cmd")),
            format!("@echo off\r\necho %*\r\n>{program}-cwd.txt echo installed\r\n"),
        )?;
    }
    let system = std::env::var_os("SystemRoot").context("Windows system directory")?;
    let path = std::env::join_paths([bin.clone(), Path::new(&system).join("System32")])?;
    let run = |root: &Path, args: &[&str]| -> Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_webui"))
            .args(["--format=json", "init"])
            .arg(root)
            .args(args)
            .env("PATH", &path)
            .stdin(Stdio::null())
            .output()
            .context("running webui init through Windows cmd.exe")
    };
    let skipped = temp.path().join("skipped-app");
    let output = run(&skipped, &["--no-interactive"])?;
    assert!(output.status.success());
    assert!(!skipped.join("npm-cwd.txt").exists());
    assert!(!skipped.join("npx-cwd.txt").exists());

    let root = temp.path().join("installed-app");
    let output = run(&root, &["--install", "--ai-skill", "--no-interactive"])?;
    anyhow::ensure!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["dependencies"], "installed");
    assert_eq!(report["ai_skill"], "installed");
    assert!(root.join("npm-cwd.txt").is_file());
    assert!(root.join("npx-cwd.txt").is_file());
    assert!(String::from_utf8(output.stderr)?.contains("webui-reference"));

    fs::write(bin.join("npm.cmd"), "@echo off\r\nexit /b 23\r\n")?;
    let failed = temp.path().join("failed-app");
    let output = run(&failed, &["--install", "--no-interactive"])?;
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
        assert_eq!(
            files.iter().any(|file| file == "scripts/sdk.mjs"),
            matches!(server, "c" | "cpp" | "go")
        );
        if matches!(server, "c" | "cpp") {
            assert!(files.iter().any(|file| file == "server/host.h"));
            assert!(files.iter().any(|file| file == "server/native.cmake"));
        }
        if server == "go" {
            assert!(!files.iter().any(|file| file == "server/webui.go"));
            let source = fs::read_to_string(root.join("server/main.go"))?;
            let module_path = concat!(
                "github.com/microsoft/webui/go/v",
                env!("CARGO_PKG_VERSION_MAJOR")
            );
            assert!(source.contains(module_path));
            assert!(!source.contains("import \"C\""));
            let module = fs::read_to_string(root.join("server/go.mod"))?;
            let native = env!("CARGO_PKG_VERSION");
            let version = if let Some((base, number)) = native.split_once("-hotfix.") {
                let (prefix, patch) = base.rsplit_once('.').context("release patch")?;
                let patch = patch
                    .parse::<u64>()?
                    .checked_add(1)
                    .context("Go hotfix patch overflow")?;
                format!("{prefix}.{patch}-0.hotfix.{number}")
            } else {
                native.to_owned()
            };
            assert!(module.contains(&format!("require {module_path} v{version}")));
            assert!(!module.contains("replace"));
        }
        if server == "rust" {
            assert!(files.iter().any(|file| file == "server/Cargo.toml"));
            assert!(!root.join("server/Cargo.toml.template").exists());
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
            assert!(!text.contains("__WEBUI_GO_VERSION__"), "{path}");
            assert!(!text.contains("__SERVER__"), "{path}");
            assert!(!text.contains("workspace:*"), "{path}");
        }
    }
    Ok(())
}

#[test]
fn go_named_application_has_a_valid_nonreserved_module_path() -> Result<()> {
    let temp = temp()?;
    let root = temp.path().join("go");
    let output = init(&root, &["--server", "go", "--no-interactive"])?;
    assert!(output.status.success());
    let module = fs::read_to_string(root.join("server/go.mod"))?;
    assert!(module.starts_with("module example.com/go\n"));
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
fn rejected_names_explain_normalization_limits() -> Result<()> {
    let temp = temp()?;
    for name in ["123".to_owned(), "node_modules".to_owned(), "a".repeat(65)] {
        let root = temp.path().join(name);
        let output = init(&root, &["--no-interactive"])?;
        assert_eq!(output.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(error["code"], "init-invalid-name");
        let help = error["help"].as_str().context("name correction help")?;
        for restriction in [
            "ASCII",
            "normalize",
            "letter",
            "64",
            "con",
            "prn",
            "aux",
            "nul",
            "com1",
            "com9",
            "lpt1",
            "lpt9",
            "node-modules",
        ] {
            assert!(help.contains(restriction), "{help}");
        }
        assert!(!root.exists());
    }
    Ok(())
}

#[test]
fn rejected_file_components_and_roots_explain_destination_rules() -> Result<()> {
    let temp = temp()?;
    let file = temp.path().join("regular-file");
    fs::write(&file, "preserved")?;
    let filesystem_root = temp.path().ancestors().last().context("filesystem root")?;
    for root in [file.clone(), file.join("app"), filesystem_root.to_owned()] {
        let output = init(&root, &["--no-interactive"])?;
        assert_eq!(output.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(error["code"], "init-unsafe-destination");
        let help = error["help"]
            .as_str()
            .context("destination correction help")?;
        for restriction in ["root", "file", "symlink", "'..'"] {
            assert!(help.contains(restriction), "{help}");
        }
    }
    assert_eq!(fs::read_to_string(file)?, "preserved");
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
    for program in ["npm", "npx", "yarn"] {
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

    let yarn_root = temp.path().join("yarn-app");
    let output = Command::new(env!("CARGO_BIN_EXE_webui"))
        .args(["--format=json", "init"])
        .arg(&yarn_root)
        .args(["--package-manager", "yarn", "--install", "--no-interactive"])
        .env("PATH", &bin)
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["package_manager"], "yarn");
    assert_eq!(report["dependencies"], "installed");
    assert_eq!(report["next_commands"][0], "yarn run dev");
    assert_eq!(
        fs::read_to_string(yarn_root.join(".yarnrc.yml"))?,
        "nodeLinker: node-modules\n"
    );
    assert!(String::from_utf8(output.stderr)?.contains(&yarn_root.display().to_string()));

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
