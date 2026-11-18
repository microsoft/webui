// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::cargo_artifacts::{self, Kind};
use crate::util::{build_command, run_command_quiet};
use std::ffi::OsString;
use std::fs;
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
    check_external_consumer(&workspace, &root, (&cli, library_dir), &protocol, &proxy)?;
    check_generated_consumer(&root, &cli, &library, &protocol, &proxy)
}

fn check_generated_consumer(
    root: &Path,
    cli: &Path,
    library: &Path,
    protocol: &Path,
    proxy: &Path,
) -> Result<(), String> {
    let mut command = build_command(
        "node",
        &["--test", "crates/webui-cli/tests/go-module.test.mjs"],
    );
    configure_environment(
        &mut command,
        &root.join("SDK with spaces/include"),
        &root.join("SDK with spaces/lib"),
        protocol,
    )?;
    command
        .env("WEBUI_GO_TEST_CLI", cli)
        .env("WEBUI_GO_TEST_LIBRARY", library)
        .env("WEBUI_GO_TEST_PROXY", proxy)
        .env("WEBUI_SDK_DIR", root.join("SDK with spaces"))
        .env("GONOSUMDB", "*")
        .env("GONOPROXY", "")
        .env("GOPRIVATE", "")
        .env("GOFLAGS", "");
    checked(
        &mut command,
        "test initializer-generated Go module consumer",
    )
}

fn checked(command: &mut Command, label: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot run {label}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn check_external_consumer(
    workspace: &Path,
    root: &Path,
    native: (&Path, &Path),
    protocol: &Path,
    proxy: &Path,
) -> Result<(), String> {
    let (cli, library_dir) = native;
    let version = crate::version::workspace_package_version(workspace)?;
    let module_version = crate::release_version::ReleaseVersion::parse(&version)
        .and_then(crate::release_version::ReleaseVersion::go_version)
        .ok_or_else(|| format!("cannot map native SDK version {version} to a Go module version"))?;
    let staging = root.join("release");
    let header = staging.join("crates/webui-ffi/include");
    fs::create_dir_all(&header).map_err(|error| error.to_string())?;
    for (source, destination) in [
        (workspace.join("Cargo.toml"), staging.join("Cargo.toml")),
        (workspace.join("LICENSE"), staging.join("LICENSE")),
        (
            workspace.join("crates/webui-ffi/include/webui_ffi.h"),
            header.join("webui_ffi.h"),
        ),
    ] {
        fs::copy(source, destination).map_err(|error| error.to_string())?;
    }
    let archive = crate::publish::stage_sdk_archive(
        &staging,
        &crate::publish::detect_host_triple(),
        library_dir,
    )?;
    let sdk = root.join("SDK with spaces");
    fs::create_dir_all(&sdk).map_err(|error| error.to_string())?;
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(&sdk),
        "extract standalone SDK",
    )?;
    checked(
        Command::new(cli)
            .args(["--format", "json", "sdk", "check"])
            .env("WEBUI_SDK_DIR", &sdk)
            .env("PATH", ""),
        "validate genuine standalone SDK without Node or npm",
    )?;
    let app = root.join("consumer");
    fs::create_dir_all(&app).map_err(|error| error.to_string())?;
    fs::write(
        app.join("go.mod"),
        format!(
            "module consumer\n\ngo 1.25\n\nrequire github.com/microsoft/webui/go/v3 v{module_version}\n"
        ),
    )
    .map_err(|error| error.to_string())?;
    fs::write(app.join("main.go"), include_str!("go_consumer.go"))
        .map_err(|error| error.to_string())?;
    let executable = app.join(if cfg!(windows) {
        "consumer.exe"
    } else {
        "consumer"
    });
    let mut build = Command::new(cli);
    build
        .args(["sdk", "exec", "--", "go", "build", "-mod=mod", "-o"])
        .arg(&executable)
        .arg(".")
        .current_dir(&app)
        .env("WEBUI_SDK_DIR", &sdk)
        .env("GOTOOLCHAIN", "local")
        .env("GOWORK", "off");
    build
        .env("GOPROXY", crate::publish::go_proxy_url(proxy)?)
        .env("GOSUMDB", "off")
        .env("GONOSUMDB", "*")
        .env("GONOPROXY", "")
        .env("GOPRIVATE", "")
        .env("GOFLAGS", "")
        .env("GOMODCACHE", root.join("module-cache"));
    let result = checked(&mut build, "build external Go module consumer").and_then(|()| {
        let sum = fs::read_to_string(app.join("go.sum")).map_err(|error| error.to_string())?;
        if !sum.contains(&format!(
            "github.com/microsoft/webui/go/v3 v{module_version} h1:"
        )) {
            return Err("external consumer did not checksum the staged module".to_owned());
        }
        let mut consumer = Command::new(executable);
        configure_environment(
            &mut consumer,
            &sdk.join("include"),
            &sdk.join("lib"),
            protocol,
        )?;
        consumer.env("PATH", sdk.join("lib")).current_dir(&app);
        checked(
            &mut consumer,
            "run external Go HTTP consumer without Node or npm",
        )
    });
    let cleanup = checked(
        build_command("go", &["clean", "-modcache"]).env("GOMODCACHE", root.join("module-cache")),
        "clean isolated Go module cache",
    );
    result.and(cleanup)
}

fn check_formatting() -> Result<(), String> {
    let output = build_command(
        "gofmt",
        &[
            "-l",
            "go",
            "xtask/src/go_consumer.go",
            "xtask/src/publish_go_packer.go",
            "crates/webui-cli/src/commands/init/templates/go/server",
        ],
    )
    .output()
    .map_err(|error| format!("Go formatting requires gofmt on PATH: {error}"))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(format!(
            "Go formatting failed; run gofmt -w go xtask/src/go_consumer.go xtask/src/publish_go_packer.go crates/webui-cli/src/commands/init/templates/go/server:\n{}{}",
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
        .env("CGO_CFLAGS", prepend_flag("CGO_CFLAGS", "-I", include)?)
        .env("CGO_LDFLAGS", prepend_flag("CGO_LDFLAGS", "-L", library)?);
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

fn prepend_flag(variable: &str, prefix: &str, path: &Path) -> Result<OsString, String> {
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
    let mut value = OsString::from(flag);
    if let Some(previous) = std::env::var_os(variable).filter(|previous| !previous.is_empty()) {
        value.push(" ");
        value.push(previous);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_proxy_urls_preserve_spaces_and_windows_paths() -> Result<(), String> {
        for (path, expected) in [
            ("/tmp/Go proxy#1%", "file:///tmp/Go%20proxy%231%25"),
            ("/proxy,comma|pipe", "file:///proxy%2Ccomma%7Cpipe"),
            (r"\\?\C:\Go proxy", "file:///C:/Go%20proxy"),
            (r"\\?\UNC\server\Go proxy", "file://server/Go%20proxy"),
        ] {
            assert_eq!(crate::publish::go_proxy_url(Path::new(path))?, expected);
        }
        assert!(crate::publish::go_proxy_url(Path::new("relative/path")).is_err());
        Ok(())
    }

    #[test]
    fn selected_sdk_flags_precede_inherited_flags() -> Result<(), String> {
        const CHILD: &str = "WEBUI_TEST_INHERITED_CGO_FLAGS";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
                .args([
                    "--exact",
                    "go_tests::tests::selected_sdk_flags_precede_inherited_flags",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("CGO_CFLAGS", "\"-Istale SDK/include\" -DKEEP=1")
                .env("CGO_LDFLAGS", "\"-Lstale SDK/lib\" -lm")
                .output()
                .map_err(|error| error.to_string())?;
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(());
        }
        let mut command = Command::new("go");
        configure_environment(
            &mut command,
            Path::new("selected SDK/include"),
            Path::new("selected SDK/lib"),
            Path::new("fixture/protocol.bin"),
        )?;
        for (variable, expected) in [
            (
                "CGO_CFLAGS",
                "\"-Iselected SDK/include\" \"-Istale SDK/include\" -DKEEP=1",
            ),
            (
                "CGO_LDFLAGS",
                "\"-Lselected SDK/lib\" \"-Lstale SDK/lib\" -lm",
            ),
        ] {
            let actual = command
                .get_envs()
                .find(|(key, _)| *key == variable)
                .and_then(|(_, value)| value);
            assert_eq!(actual, Some(std::ffi::OsStr::new(expected)));
        }
        Ok(())
    }

    #[test]
    fn compiler_flags_normalize_windows_verbatim_paths() -> Result<(), String> {
        let flag = prepend_flag(
            "WEBUI_UNUSED_TEST_FLAGS",
            "-I",
            Path::new(r"\\?\C:\SDK folder\include"),
        )?;
        assert_eq!(flag, "\"-IC:/SDK folder/include\"");
        let flag = prepend_flag(
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
