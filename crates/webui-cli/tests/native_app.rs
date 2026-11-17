// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};

fn fixture() -> Result<tempfile::TempDir> {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize()?)?;
    let root = temp.path();
    fs::create_dir(root.join("web"))?;
    fs::create_dir(root.join("src"))?;
    fs::write(
        root.join("web/index.html"),
        "<html><head><title>Native</title></head><body><h1>Native fixture</h1></body></html>",
    )?;
    fs::write(
        root.join("Cargo.toml"),
        r#"
[package]
name = "native-host-fixture"
version = "0.0.0"
edition = "2021"
[workspace]
[[bin]]
name = "webui-app"
path = "src/main.rs"
"#,
    )?;
    fs::write(
        root.join("src/main.rs"),
        r#"
use std::io::Read;
fn main() {
    println!("HOST_READY");
    let mut command = [0_u8; 1];
    if std::io::stdin().read_exact(&mut command).is_err() || command != *b"S" {
        std::process::exit(19);
    }
}
"#,
    )?;
    fs::write(
        root.join("webui.toml"),
        format!(
            r#"
schema = 1
version = "{}"
server = "rust"
app = "web"
entry = "index.html"
manifest = "Cargo.toml"
target = "webui-app"
host = "src/main.rs"
"#,
            env!("CARGO_PKG_VERSION")
        ),
    )?;
    Ok(temp)
}

fn cli(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_webui"));
    command
        .args(["app"])
        .current_dir(root)
        .env_remove("WEBUI_TOOLCHAIN_DIR");
    command
}

#[test]
fn html_only_build_needs_no_node_checker_or_host_toolchain_and_rejects_redirected_staging(
) -> Result<()> {
    let temp = fixture()?;
    let output = cli(temp.path())
        .args(["build", "--frontend-only"])
        .env("PATH", "")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join("dist/protocol.bin").is_file());
    assert!(temp.path().join("dist/public/webui-LICENSE.txt").is_file());
    assert!(!temp.path().join(".webui/toolchain").exists());
    assert!(!temp.path().join(".webui/server-executable.json").exists());
    let first = fs::read(temp.path().join("dist/protocol.bin"))?;
    let second = cli(temp.path())
        .args(["build", "--frontend-only"])
        .env("PATH", "")
        .output()?;
    assert!(second.status.success());
    assert_eq!(first, fs::read(temp.path().join("dist/protocol.bin"))?);
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir()?;
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".webui/build"))?;
        let rejected = cli(temp.path())
            .args(["build", "--frontend-only"])
            .env("PATH", "")
            .output()?;
        assert!(!rejected.status.success());
        assert_eq!(first, fs::read(temp.path().join("dist/protocol.bin"))?);
        assert_eq!(fs::read_dir(outside.path())?.count(), 0);
    }
    Ok(())
}

#[test]
fn supervised_start_keeps_control_open_until_an_explicit_stop() -> Result<()> {
    let temp = fixture()?;
    let built = cli(temp.path()).arg("build").output()?;
    anyhow::ensure!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut child = cli(temp.path())
        .args(["start", "--control-stdin"])
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut controller = Some(child.stdin.take().context("master controller")?);
    let stdout = child.stdout.take().context("supervised output")?;
    let (ready, ready_rx) = mpsc::sync_channel(1);
    let output = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if matches!(line.as_deref(), Ok("HOST_READY")) {
                let _ = ready.send(());
            }
        }
    });
    let (exited, exit_rx) = mpsc::sync_channel(1);
    let waiter = std::thread::spawn(move || {
        let _ = exited.send(child.wait());
    });
    let result = (|| -> Result<()> {
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .context("host did not become ready")?;
        assert!(
            matches!(
                exit_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "waiting must not close the native host's control pipe"
        );
        let concurrent = cli(temp.path())
            .args(["build", "--frontend-only"])
            .env("PATH", "")
            .output()?;
        assert!(
            !concurrent.status.success(),
            "a running server must retain exclusive generation ownership"
        );
        controller
            .as_mut()
            .context("open controller")?
            .write_all(b"S")?;
        drop(controller.take());
        let status = exit_rx
            .recv_timeout(Duration::from_secs(16))
            .context("native supervisor did not stop")??;
        assert!(status.success(), "{status}");
        Ok(())
    })();
    if result.is_err() {
        drop(controller.take());
    }
    waiter
        .join()
        .map_err(|_| anyhow::anyhow!("exit waiter panicked"))?;
    output
        .join()
        .map_err(|_| anyhow::anyhow!("output reader panicked"))?;
    result
}
