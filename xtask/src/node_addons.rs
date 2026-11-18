// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build and select the independent handler and compiler Node addons.

use crate::cargo_artifacts::{self, Kind};
use crate::util::{build_command, workspace_root};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    Handler,
    Build,
}

impl Variant {
    fn feature(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Build => "build",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Addons {
    pub(crate) runtime: PathBuf,
    pub(crate) builder: PathBuf,
}

impl Addons {
    pub(crate) fn apply(&self, command: &mut Command) {
        command
            .env("WEBUI_ADDON_PATH", &self.runtime)
            .env("WEBUI_BUILD_ADDON_PATH", &self.builder);
    }
}

pub(crate) fn build(release: bool) -> Result<Addons, String> {
    build_for(&workspace_root()?, release, None)
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let result = parse_release(args).and_then(build);
    match result {
        Ok(addons) => {
            eprintln!("WEBUI_ADDON_PATH={}", addons.runtime.display());
            eprintln!("WEBUI_BUILD_ADDON_PATH={}", addons.builder.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Node addon build failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_release(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [arg] if arg == "--release" => Ok(true),
        _ => Err("Usage: cargo xtask build-node-addons [--release]".into()),
    }
}

pub(crate) fn build_for(
    root: &Path,
    release: bool,
    target: Option<&str>,
) -> Result<Addons, String> {
    build_with(root, release, target, |mut command| {
        run_checked(&mut command, "WebUI Node addon build")
    })
}

/// Run each variant independently and select only that invocation's artifact.
pub(crate) fn build_with(
    root: &Path,
    release: bool,
    target: Option<&str>,
    mut run: impl FnMut(Command) -> Result<Output, String>,
) -> Result<Addons, String> {
    let target_dir = target_directory(root)?;
    let mut paths = Vec::with_capacity(2);
    for variant in [Variant::Handler, Variant::Build] {
        let mut command = variant_command("build", variant, &target_dir);
        command
            .current_dir(root)
            .arg("--message-format=json-render-diagnostics");
        if release {
            command.arg("--release");
        }
        if let Some(target) = target {
            command.args(["--target", target]);
        }
        let output = run(command)?;
        let path = cargo_artifacts::find(&output.stdout, "webui_node", Kind::Cdylib)?;
        if !path.is_file() {
            return Err(format!(
                "Cargo reported a WebUI Node addon that does not exist: {}",
                path.display()
            ));
        }
        paths.push(path);
    }
    let builder = paths.pop().ok_or("missing compiler addon")?;
    let runtime = paths.pop().ok_or("missing handler addon")?;
    if runtime == builder {
        return Err("Handler and compiler addons must use separate target roots".into());
    }
    Ok(Addons { runtime, builder })
}

pub(crate) fn variant_command(operation: &str, variant: Variant, target_dir: &Path) -> Command {
    let mut command = build_command(
        "cargo",
        &[
            operation,
            "--locked",
            "-p",
            "microsoft-webui-node",
            "--no-default-features",
            "--features",
            variant.feature(),
        ],
    );
    if variant == Variant::Build {
        command
            .arg("--target-dir")
            .arg(target_dir.join("node-build"));
    }
    command
}

/// Let Cargo resolve environment overrides and build.target-dir configuration.
pub(crate) fn target_directory(root: &Path) -> Result<PathBuf, String> {
    let output = run_checked(
        &mut metadata_command(root),
        "Cargo target directory discovery",
    )?;
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid Cargo metadata: {error}"))?;
    metadata["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "Cargo metadata did not report an absolute target_directory".into())
}

fn metadata_command(root: &Path) -> Command {
    let mut command = build_command(
        "cargo",
        &["metadata", "--locked", "--no-deps", "--format-version", "1"],
    );
    command.current_dir(root);
    command
}

pub(crate) fn run_checked(command: &mut Command, label: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn message(target: &str, file: &str) -> String {
        serde_json::json!({
            "reason": "compiler-artifact",
            "target": { "name": target, "crate_types": ["cdylib"] },
            "filenames": [file, format!("{target}.pdb"), format!("{target}.dll.lib")],
        })
        .to_string()
            + "\n"
    }

    #[test]
    fn variants_use_exact_features_and_separate_effective_target_roots() {
        let target = Path::new("/custom target");
        for operation in ["build", "test", "clippy"] {
            for variant in [Variant::Handler, Variant::Build] {
                let command = variant_command(operation, variant, target);
                let args: Vec<_> = command.get_args().collect();
                assert_eq!(args[0], operation);
                assert!(args
                    .windows(2)
                    .any(|pair| pair == [OsStr::new("-p"), OsStr::new("microsoft-webui-node")]));
                assert!(args
                    .windows(2)
                    .any(|pair| pair == [OsStr::new("--features"), OsStr::new(variant.feature())]));
                assert!(args.contains(&OsStr::new("--no-default-features")));
                assert!(args.contains(&OsStr::new("--locked")));
                if variant == Variant::Build {
                    assert!(args.windows(2).any(|pair| pair
                        == [
                            OsStr::new("--target-dir"),
                            target.join("node-build").as_os_str()
                        ]));
                } else {
                    assert!(!args.contains(&OsStr::new("--target-dir")));
                }
            }
        }
    }

    #[test]
    fn selects_both_reported_addons_across_platforms_and_target_directories() {
        for (runtime, builder) in [
            (
                "/custom target/debug/libwebui_node.so",
                "/custom target/node-build/debug/libwebui_node.so",
            ),
            (
                "/custom target/aarch64-apple-darwin/debug/libwebui_node.dylib",
                "/custom target/node-build/aarch64-apple-darwin/debug/libwebui_node.dylib",
            ),
            (
                "C:/custom target/debug/webui_node.dll",
                "C:/custom target/node-build/debug/webui_node.dll",
            ),
        ] {
            for file in [runtime, builder] {
                let output = message("dependency", "/other/libdependency.so")
                    + &message("webui_node", file)
                    + "{\"reason\":\"build-finished\",\"success\":true}\n";
                assert_eq!(
                    cargo_artifacts::find(output.as_bytes(), "webui_node", Kind::Cdylib),
                    Ok(PathBuf::from(file))
                );
            }
        }
    }

    #[test]
    fn rejects_missing_malformed_and_ambiguous_artifacts_for_either_addon() {
        let runtime = message("webui_node", "runtime.so");
        for output in [
            String::new(),
            "not json".into(),
            message("other", "other.so"),
            runtime.clone() + &runtime,
            r#"{"reason":"compiler-artifact","target":{"name":"webui_node","crate_types":["rlib"]},"filenames":["builder.so"]}"#.into(),
            r#"{"reason":"compiler-artifact","target":{"name":"webui_node","crate_types":["cdylib"]},"filenames":["one.so","two.so"]}"#.into(),
        ] {
            assert!(cargo_artifacts::find(output.as_bytes(), "webui_node", Kind::Cdylib).is_err(), "{output}");
        }
    }

    #[test]
    fn build_cli_accepts_only_the_selected_profile() {
        assert_eq!(parse_release(&[]), Ok(false));
        assert_eq!(parse_release(&["--release".into()]), Ok(true));
        assert!(parse_release(&["--features=handler,build".into()]).is_err());
    }

    #[test]
    fn builds_once_per_variant_and_uses_only_each_reported_artifact(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n")?;
        let target_dir = target_directory(root.path())?;
        let status =
            run_checked(&mut build_command("cargo", &["--version"]), "Cargo fixture")?.status;
        let runtime = root.path().join("reported handler.so");
        let builder = root.path().join("reported compiler.so");
        std::fs::write(&runtime, "handler")?;
        std::fs::write(&builder, "compiler")?;
        for release in [false, true] {
            for target in [None, Some("aarch64-pc-windows-msvc")] {
                let mut invocations = 0;
                let addons = build_with(root.path(), release, target, |command| {
                    let args: Vec<_> = command.get_args().collect();
                    let variant = [Variant::Handler, Variant::Build][invocations];
                    invocations += 1;
                    assert_eq!(args.contains(&OsStr::new("--release")), release);
                    assert_eq!(args.contains(&OsStr::new("--target")), target.is_some());
                    if let Some(target) = target {
                        assert!(args
                            .windows(2)
                            .any(|pair| pair == [OsStr::new("--target"), OsStr::new(target)]));
                    }
                    assert!(args
                        .windows(2)
                        .any(|pair| pair
                            == [OsStr::new("--features"), OsStr::new(variant.feature())]));
                    assert!(args.contains(&OsStr::new("--no-default-features")));
                    assert!(args.contains(&OsStr::new("--message-format=json-render-diagnostics")));
                    if variant == Variant::Build {
                        assert!(args.windows(2).any(|pair| pair
                            == [
                                OsStr::new("--target-dir"),
                                target_dir.join("node-build").as_os_str()
                            ]));
                    } else {
                        assert!(!args.contains(&OsStr::new("--target-dir")));
                    }
                    let file = if variant == Variant::Handler {
                        &runtime
                    } else {
                        &builder
                    };
                    Ok(Output {
                        status,
                        stdout: message("webui_node", &file.to_string_lossy()).into_bytes(),
                        stderr: Vec::new(),
                    })
                })?;
                assert_eq!(invocations, 2);
                assert_eq!(
                    addons,
                    Addons {
                        runtime: runtime.clone(),
                        builder: builder.clone()
                    }
                );
            }
        }
        // Existing artifacts cannot replace a missing Cargo artifact message.
        assert!(build_with(root.path(), false, None, |_| Ok(Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new()
        }))
        .is_err());
        Ok(())
    }

    #[test]
    fn metadata_honors_relative_and_absolute_environment_target_roots(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n[package]\nname='target-root-fixture'\nversion='0.0.0'\n[lib]\npath='lib.rs'\n")?;
        std::fs::write(root.path().join("lib.rs"), "")?;
        for requested in [
            PathBuf::from("custom target"),
            root.path().join("absolute target"),
        ] {
            let output = run_checked(
                metadata_command(root.path()).env("CARGO_TARGET_DIR", &requested),
                "target root fixture",
            )?;
            let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let expected = if requested.is_absolute() {
                requested
            } else {
                root.path().canonicalize()?.join(requested)
            };
            assert_eq!(metadata["target_directory"].as_str(), expected.to_str());
        }
        std::fs::create_dir(root.path().join(".cargo"))?;
        std::fs::write(
            root.path().join(".cargo/config.toml"),
            "[build]\ntarget-dir='configured target'\n",
        )?;
        let output = run_checked(
            metadata_command(root.path()).env_remove("CARGO_TARGET_DIR"),
            "configured target root fixture",
        )?;
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            metadata["target_directory"].as_str(),
            root.path()
                .canonicalize()?
                .join("configured target")
                .to_str()
        );
        Ok(())
    }

    #[test]
    fn propagates_failed_child_process_output() {
        let result = run_checked(
            Command::new("node")
                .args(["-e", "console.error('node-suite-failure'); process.exit(1)"]),
            "WebUI Node package tests",
        );
        let Err(error) = result else {
            panic!("a failing Node suite must fail the gate");
        };
        assert!(error.contains("WebUI Node package tests failed"));
        assert!(error.contains("node-suite-failure"));
    }
}
