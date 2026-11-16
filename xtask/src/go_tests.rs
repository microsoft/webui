// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::cargo_artifacts::{self, Kind};
use crate::util::{build_command, run_command_quiet};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run() -> Result<(), String> {
    run_command_quiet("go", &["version"], None).map_err(|error| {
        format!("Go binding tests require Go 1.25+ and a cgo C compiler:\n{error}")
    })?;
    check_formatting()?;
    let output = build_command(
        "cargo",
        &[
            "build",
            "--locked",
            "-p",
            "microsoft-webui-cli",
            "-p",
            "microsoft-webui-ffi",
            "--message-format=json-render-diagnostics",
        ],
    )
    .output()
    .map_err(|error| format!("cannot build Go test native artifacts: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Go native artifact build failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let cli = cargo_artifacts::find(&output.stdout, "webui", Kind::Executable)?;
    let library = cargo_artifacts::find(&output.stdout, "webui_ffi", Kind::Cdylib)?;
    let temporary = tempfile::Builder::new()
        .prefix("webui-go-tests-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let root = if cfg!(windows) {
        temporary.path().to_path_buf()
    } else {
        temporary
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
    };
    let protocol = root.join("protocol.bin");
    let built = Command::new(&cli)
        .args(["build", "go/testdata/src", "--plugin", "webui", "--out"])
        .arg(&root)
        .output()
        .map_err(|error| format!("cannot compile Go fixture: {error}"))?;
    if !built.status.success() {
        return Err(format!(
            "Go fixture build failed:\n{}{}",
            String::from_utf8_lossy(&built.stdout),
            String::from_utf8_lossy(&built.stderr)
        ));
    }
    let include = Path::new("crates/webui-ffi/include")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let library_dir = library.parent().ok_or("FFI artifact has no directory")?;
    for arguments in [
        &["test", "-race", "-count=1", "./..."][..],
        &["vet", "./..."][..],
    ] {
        let mut command = go_command(&include, library_dir, &protocol, arguments)?;
        let output = command
            .output()
            .map_err(|error| format!("cannot run Go binding checks: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "go {} failed:\n{}{}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    let version = crate::version::read_version()?;
    let proxy = root.join("proxy");
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    crate::publish::stage_go_proxy(&workspace, &version, &proxy)?;
    let script = workspace.join("crates/webui-cli/tests/go-module.test.mjs");
    let mut consumer = build_command("node", &["--test"]);
    consumer.arg(script).current_dir(&workspace);
    configure_environment(&mut consumer, &include, library_dir, &protocol)?;
    consumer
        .env("WEBUI_GO_TEST_CLI", &cli)
        .env("WEBUI_GO_TEST_PROXY", &proxy)
        .env("WEBUI_GO_TEST_LIBRARY", &library);
    let output = consumer
        .output()
        .map_err(|error| format!("cannot run generated Go consumer tests: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Generated Go consumer failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn check_formatting() -> Result<(), String> {
    let output = build_command("gofmt", &["-l", "go"])
        .output()
        .map_err(|error| format!("Go formatting requires gofmt on PATH: {error}"))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(format!(
            "Go formatting failed; run gofmt -w go:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn go_command(
    include: &Path,
    library: &Path,
    protocol: &Path,
    arguments: &[&str],
) -> Result<Command, String> {
    let mut command = build_command("go", arguments);
    command.current_dir("go");
    configure_environment(&mut command, include, library, protocol)?;
    Ok(command)
}

fn configure_environment(
    command: &mut Command,
    include: &Path,
    library: &Path,
    protocol: &Path,
) -> Result<(), String> {
    command
        .env("CGO_ENABLED", "1")
        .env("GOTOOLCHAIN", "local")
        .env("GOWORK", "off")
        .env("WEBUI_GO_TEST_PROTOCOL", protocol)
        .env("CGO_CFLAGS", append_flag("CGO_CFLAGS", "-I", include)?)
        .env("CGO_LDFLAGS", append_flag("CGO_LDFLAGS", "-L", library)?);
    let loader = if cfg!(windows) {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let mut paths = vec![library.to_path_buf()];
    if let Some(previous) = std::env::var_os(loader) {
        paths.extend(std::env::split_paths(&previous));
    }
    command.env(
        loader,
        std::env::join_paths(paths).map_err(|error| error.to_string())?,
    );
    Ok(())
}

fn append_flag(variable: &str, prefix: &str, path: &Path) -> Result<OsString, String> {
    let path = path
        .to_str()
        .ok_or("Go compiler paths must be valid UTF-8")?;
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        path.strip_prefix(r"\\?\")
            .unwrap_or(path)
            .replace('\\', "/")
    };
    let flag =
        serde_json::to_string(&format!("{prefix}{path}")).map_err(|error| error.to_string())?;
    let mut value = std::env::var_os(variable).unwrap_or_default();
    if !value.is_empty() {
        value.push(" ");
    }
    value.push(flag);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_flags_normalize_windows_verbatim_paths() -> Result<(), String> {
        let flag = append_flag(
            "WEBUI_UNUSED_TEST_FLAGS",
            "-I",
            Path::new(r"\\?\C:\SDK folder\include"),
        )?;
        assert_eq!(flag, "\"-IC:/SDK folder/include\"");
        let flag = append_flag(
            "WEBUI_UNUSED_TEST_FLAGS",
            "-L",
            Path::new(r"\\?\UNC\server\SDK folder\lib"),
        )?;
        assert_eq!(flag, "\"-L//server/SDK folder/lib\"");
        Ok(())
    }

    #[test]
    fn uses_exact_native_paths_and_a_compiled_fixture() -> Result<(), String> {
        let command = go_command(
            Path::new("SDK with spaces/include"),
            Path::new("SDK with spaces/lib"),
            Path::new("fixture/protocol.bin"),
            &["test", "-race", "./..."],
        )?;
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.iter().any(|(key, value)| {
            *key == "CGO_CFLAGS"
                && value.is_some_and(|value| {
                    value
                        .to_string_lossy()
                        .contains("\"-ISDK with spaces/include\"")
                })
        }));
        assert!(env.iter().any(|(key, value)| {
            *key == "WEBUI_GO_TEST_PROTOCOL"
                && *value == Some(Path::new("fixture/protocol.bin").as_os_str())
        }));
        assert!(env.iter().any(|(key, value)| {
            *key == "GOWORK" && *value == Some(std::ffi::OsStr::new("off"))
        }));
        Ok(())
    }
}
