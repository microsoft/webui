// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Local macOS Windows artifact builds through cargo-xwin.

use crate::util::which_exists;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const CARGO_XWIN_VERSION: &str = "0.23.0";
const PROFILE: &str = "release";
const XWIN_CACHE_DIR: &str = "target/xwin-cache";
const NATIVE_PACKAGES: &[&str] = &[
    "microsoft-webui-cli",
    "microsoft-webui-ffi",
    "microsoft-webui-node",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WindowsTarget {
    selector: &'static str,
    triple: &'static str,
    npm_package: &'static str,
    nuget_rid: &'static str,
    native_binary: &'static str,
}

const WINDOWS_TARGETS: &[WindowsTarget] = &[
    WindowsTarget {
        selector: "x64",
        triple: "x86_64-pc-windows-msvc",
        npm_package: "webui-win32-x64",
        nuget_rid: "win-x64",
        native_binary: "webui-win32-x64.exe",
    },
    WindowsTarget {
        selector: "arm64",
        triple: "aarch64-pc-windows-msvc",
        npm_package: "webui-win32-arm64",
        nuget_rid: "win-arm64",
        native_binary: "webui-win32-arm64.exe",
    },
];

#[derive(Debug, Eq, PartialEq)]
enum Request {
    Build(Vec<&'static WindowsTarget>),
    Help,
}

/// Build and stage Windows artifacts locally with cargo-xwin.
pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("  {} {message}", console::style("✘").red().bold());
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let request = parse_args(args)?;
    let Request::Build(targets) = request else {
        print_usage();
        return Ok(());
    };

    let root = std::env::current_dir().map_err(|e| format!("failed to read current dir: {e}"))?;
    ensure_macos_host()?;
    ensure_cargo_xwin()?;
    ensure_llvm_tools()?;
    ensure_rustup_targets(&targets)?;

    let cache_dir = root.join(XWIN_CACHE_DIR);
    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("failed to create {}: {e}", cache_dir.display()))?;

    eprintln!(
        "\n{} Windows local build",
        console::style("▸").cyan().bold(),
    );
    eprintln!(
        "  {} cargo-xwin {}",
        console::style("·").dim(),
        console::style(CARGO_XWIN_VERSION).bold(),
    );
    eprintln!(
        "  {} xwin cache {}",
        console::style("·").dim(),
        console::style(cache_dir.display()).bold(),
    );

    for target in &targets {
        build_target(&root, target, &cache_dir)?;
    }

    crate::publish::stage_native_targets(
        &root,
        targets.iter().map(|target| target.triple),
        PROFILE,
    )?;
    validate_staged_artifacts(&root, &targets)?;

    eprintln!(
        "\n{} Windows artifacts staged for {} target(s)\n",
        console::style("✨").green(),
        console::style(targets.len()).bold(),
    );
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Request, String> {
    let mut selected = Vec::new();
    let mut saw_target = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(Request::Help),
            "--target" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --target".to_string());
                }
                saw_target = true;
                select_targets(args[i].as_str(), &mut selected)?;
            }
            unknown => {
                return Err(format!(
                    "unknown argument '{unknown}'\nRun: cargo xtask build-windows-local --help"
                ));
            }
        }
        i += 1;
    }

    if !saw_target {
        selected.extend(WINDOWS_TARGETS.iter());
    }

    Ok(Request::Build(selected))
}

fn select_targets(value: &str, selected: &mut Vec<&'static WindowsTarget>) -> Result<(), String> {
    if value == "all" {
        selected.clear();
        selected.extend(WINDOWS_TARGETS.iter());
        return Ok(());
    }

    let Some(target) = find_target(value) else {
        return Err(format!(
            "unknown Windows target '{value}'. Supported: all, x64, arm64, x86_64-pc-windows-msvc, aarch64-pc-windows-msvc"
        ));
    };

    if !selected
        .iter()
        .any(|existing| existing.triple == target.triple)
    {
        selected.push(target);
    }
    Ok(())
}

fn find_target(value: &str) -> Option<&'static WindowsTarget> {
    WINDOWS_TARGETS
        .iter()
        .find(|target| target.selector == value || target.triple == value)
}

fn print_usage() {
    eprintln!(
        "Usage: cargo xtask build-windows-local [--target all|x64|arm64|<triple>]\n\n\
         Builds and stages Windows MSVC artifacts locally on macOS using cargo-xwin.\n\
         Defaults to both x86_64-pc-windows-msvc and aarch64-pc-windows-msvc."
    );
}

fn ensure_macos_host() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return Ok(());
    }

    Err("build-windows-local is intended for local macOS use; CI and release workflows are unchanged".to_string())
}

fn ensure_cargo_xwin() -> Result<(), String> {
    match installed_cargo_xwin_version()? {
        Some(found) if found == CARGO_XWIN_VERSION => Ok(()),
        Some(found) => Err(format!(
            "cargo-xwin {CARGO_XWIN_VERSION} is required, found {found}.\n  help: install the pinned version with: cargo install cargo-xwin --version {CARGO_XWIN_VERSION}"
        )),
        None => Err(format!(
            "cargo-xwin {CARGO_XWIN_VERSION} is required but was not found on PATH.\n  help: install it with: cargo install cargo-xwin --version {CARGO_XWIN_VERSION}"
        )),
    }
}

fn installed_cargo_xwin_version() -> Result<Option<String>, String> {
    let output = match Command::new("cargo-xwin").arg("--version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to run cargo-xwin --version: {error}")),
    };

    if !output.status.success() {
        return Err(format!(
            "cargo-xwin --version failed with {}",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(cargo_xwin_version(&stdout).map(str::to_string))
}

fn cargo_xwin_version(output: &str) -> Option<&str> {
    let mut parts = output.split_whitespace();
    if parts.next() == Some("cargo-xwin") {
        return parts.next();
    }
    None
}

fn ensure_rustup_targets(targets: &[&WindowsTarget]) -> Result<(), String> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|e| format!("failed to run rustup: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "rustup target list --installed failed with {}",
            output.status
        ));
    }

    let installed = String::from_utf8_lossy(&output.stdout);
    let mut missing = Vec::new();
    for target in targets {
        if !installed.lines().any(|line| line == target.triple) {
            missing.push(target.triple);
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(missing_targets_message(&missing))
}

fn missing_targets_message(missing: &[&str]) -> String {
    let mut message = String::from("missing Rust Windows target(s):");
    for target in missing {
        let _ = write!(&mut message, "\n  {target}");
    }
    message.push_str("\n  help: install them with: rustup target add");
    for target in missing {
        let _ = write!(&mut message, " {target}");
    }
    message
}

fn ensure_llvm_tools() -> Result<(), String> {
    let has_clang = which_exists("clang") || which_exists("clang-cl");
    let has_lld = which_exists("lld-link") || which_exists("ld.lld") || which_exists("lld");

    if has_clang && has_lld {
        return Ok(());
    }

    Err("clang/LLVM tools are required for cargo-xwin. On macOS: brew install llvm lld, then ensure both Homebrew bin directories are on PATH".to_string())
}

fn build_target(root: &Path, target: &WindowsTarget, cache_dir: &Path) -> Result<(), String> {
    eprintln!(
        "\n{} Building {}",
        console::style("▸").cyan().bold(),
        console::style(target.triple).bold(),
    );

    let args = cargo_xwin_build_args(target);
    let mut command = Command::new("cargo-xwin");
    command.args(&args);
    command.current_dir(root);
    command.env("CARGO_INCREMENTAL", "0");
    command.env("XWIN_CACHE_DIR", cache_dir);

    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "cargo xwin build failed for {} with {status}",
            target.triple
        )),
        Err(error) => Err(format!(
            "failed to spawn cargo xwin build for {}: {error}",
            target.triple
        )),
    }
}

fn cargo_xwin_build_args(target: &WindowsTarget) -> Vec<String> {
    let mut args = Vec::with_capacity(4 + (NATIVE_PACKAGES.len() * 2));
    args.push("build".to_string());
    args.push("--release".to_string());
    args.push("--target".to_string());
    args.push(target.triple.to_string());
    for package in NATIVE_PACKAGES {
        args.push("-p".to_string());
        args.push((*package).to_string());
    }
    args
}

fn validate_staged_artifacts(root: &Path, targets: &[&WindowsTarget]) -> Result<(), String> {
    let mut missing = Vec::new();

    for target in targets {
        for path in expected_artifacts(root, target) {
            if !path.is_file() {
                missing.push(path);
            }
        }
    }

    if missing.is_empty() {
        eprintln!(
            "\n  {} staged artifact validation passed",
            console::style("✔").green(),
        );
        return Ok(());
    }

    let mut message = String::from("missing staged Windows artifacts:");
    for path in missing {
        let _ = write!(&mut message, "\n  {}", path.display());
    }
    Err(message)
}

fn expected_artifacts(root: &Path, target: &WindowsTarget) -> Vec<PathBuf> {
    Vec::from([
        root.join("publish")
            .join("native")
            .join(target.native_binary),
        root.join("packages")
            .join(target.npm_package)
            .join("bin")
            .join("webui.exe"),
        root.join("packages")
            .join(target.npm_package)
            .join("webui.node"),
        root.join("dotnet")
            .join("runtimes")
            .join(target.nuget_rid)
            .join("native")
            .join("webui_ffi.dll"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    fn parse_ok(args: &[&str]) -> Request {
        match parse_args(&strings(args)) {
            Ok(request) => request,
            Err(message) => panic!("parse failed: {message}"),
        }
    }

    fn triples(request: &Request) -> Vec<&'static str> {
        match request {
            Request::Build(targets) => targets.iter().map(|target| target.triple).collect(),
            Request::Help => Vec::new(),
        }
    }

    #[test]
    fn parse_args_defaults_to_both_targets() {
        let request = parse_ok(&[]);

        assert_eq!(
            triples(&request),
            vec!["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]
        );
    }

    #[test]
    fn parse_args_accepts_selector() {
        let request = parse_ok(&["--target", "arm64"]);

        assert_eq!(triples(&request), vec!["aarch64-pc-windows-msvc"]);
    }

    #[test]
    fn parse_args_accepts_triple() {
        let request = parse_ok(&["--target", "x86_64-pc-windows-msvc"]);

        assert_eq!(triples(&request), vec!["x86_64-pc-windows-msvc"]);
    }

    #[test]
    fn parse_args_deduplicates_repeated_targets() {
        let request = parse_ok(&["--target", "x64", "--target", "x86_64-pc-windows-msvc"]);

        assert_eq!(triples(&request), vec!["x86_64-pc-windows-msvc"]);
    }

    #[test]
    fn parse_args_rejects_unknown_target() {
        let error = match parse_args(&strings(&["--target", "windows"])) {
            Ok(_) => panic!("unknown target should fail"),
            Err(message) => message,
        };

        assert!(error.contains("unknown Windows target"));
    }

    #[test]
    fn cargo_xwin_version_parses_expected_output() {
        assert_eq!(
            cargo_xwin_version("cargo-xwin 0.23.0\n"),
            Some(CARGO_XWIN_VERSION)
        );
        assert_eq!(cargo_xwin_version("cargo 1.93.0\n"), None);
    }

    #[test]
    fn missing_targets_message_includes_install_command() {
        let message =
            missing_targets_message(&["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]);

        assert!(message.contains("missing Rust Windows target(s):"));
        assert!(message.contains("  x86_64-pc-windows-msvc"));
        assert!(message.contains("  aarch64-pc-windows-msvc"));
        assert!(
            message.contains("rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc")
        );
    }

    #[test]
    fn cargo_xwin_build_args_include_native_packages() {
        let target = match find_target("x64") {
            Some(target) => target,
            None => panic!("x64 target should exist"),
        };

        let args = cargo_xwin_build_args(target);

        assert_eq!(
            args,
            vec![
                "build",
                "--release",
                "--target",
                "x86_64-pc-windows-msvc",
                "-p",
                "microsoft-webui-cli",
                "-p",
                "microsoft-webui-ffi",
                "-p",
                "microsoft-webui-node",
            ]
        );
    }

    #[test]
    fn expected_artifacts_match_existing_package_layout() {
        let target = match find_target("arm64") {
            Some(target) => target,
            None => panic!("arm64 target should exist"),
        };
        let root = Path::new("/repo");

        let artifacts = expected_artifacts(root, target);

        assert_eq!(
            artifacts,
            Vec::from([
                PathBuf::from("/repo/publish/native/webui-win32-arm64.exe"),
                PathBuf::from("/repo/packages/webui-win32-arm64/bin/webui.exe"),
                PathBuf::from("/repo/packages/webui-win32-arm64/webui.node"),
                PathBuf::from("/repo/dotnet/runtimes/win-arm64/native/webui_ffi.dll"),
            ])
        );
    }
}
