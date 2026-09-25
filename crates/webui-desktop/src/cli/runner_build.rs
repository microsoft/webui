// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::Path;
use std::process::Command;

pub(crate) struct RunnerBuildOptions {
    pub(crate) release: bool,
    pub(crate) default_features: bool,
    pub(crate) features: Vec<String>,
}

pub(crate) fn build_command(
    package: &str,
    manifest: Option<&Path>,
    options: &RunnerBuildOptions,
) -> Command {
    let mut command = Command::new("cargo");
    command.args(["build", "-p", package]);
    if let Some(manifest) = manifest {
        command.arg("--manifest-path").arg(manifest);
    }
    if options.release {
        command.arg("--release");
    }
    if !options.default_features {
        command.arg("--no-default-features");
    }
    if !options.features.is_empty() {
        command.arg("--features").arg(options.features.join(","));
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_build_is_optimized_and_excludes_development_defaults() {
        let command = build_command(
            "sample-app",
            Some(Path::new("desktop/Cargo.toml")),
            &RunnerBuildOptions {
                release: true,
                default_features: false,
                features: Vec::new(),
            },
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "build",
                "-p",
                "sample-app",
                "--manifest-path",
                "desktop/Cargo.toml",
                "--release",
                "--no-default-features",
            ]
        );
    }

    #[test]
    fn explicit_capabilities_do_not_restore_development_defaults() {
        let command = build_command(
            "sample-app",
            None,
            &RunnerBuildOptions {
                release: true,
                default_features: false,
                features: vec!["tray".to_string(), "native-dialogs".to_string()],
            },
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "build",
                "-p",
                "sample-app",
                "--release",
                "--no-default-features",
                "--features",
                "tray,native-dialogs",
            ]
        );
    }

    #[test]
    fn custom_default_features_and_debug_profile_are_explicit() {
        let command = build_command(
            "sample-app",
            None,
            &RunnerBuildOptions {
                release: false,
                default_features: true,
                features: Vec::new(),
            },
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["build", "-p", "sample-app"]);
    }
}
