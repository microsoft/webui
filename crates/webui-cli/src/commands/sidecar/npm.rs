// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{Sidecar, WEBUI_VERSION};

fn package_near(path: &Path, package: &str) -> Option<PathBuf> {
    path.ancestors()
        .map(|directory| directory.join("node_modules").join(package))
        .find(|root| root.join("package.json").is_file())
}

fn platform_package(sidecar: Sidecar) -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        os => os,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        arch => arch,
    };
    format!("@microsoft/{}-{os}-{arch}", sidecar.binary())
}

pub(super) fn sidecar_near(sidecar: Sidecar, path: &Path) -> Result<Option<PathBuf>> {
    let package_name = format!("@microsoft/{}", sidecar.binary());
    let Some(root) = package_near(path, &package_name) else {
        return Ok(None);
    };
    let root = fs::canonicalize(root)?;
    let platform = platform_package(sidecar);
    let native = package_near(&root, &platform).with_context(|| {
        format!("Missing {platform}.\nhelp: Reinstall {package_name} with optional dependencies enabled.")
    })?;
    for package in [&root, &native] {
        #[derive(serde::Deserialize)]
        struct Manifest {
            version: String,
        }
        let manifest = package.join("package.json");
        let parsed: Manifest = serde_json::from_slice(&fs::read(&manifest)?)
            .with_context(|| format!("Invalid package manifest at {}", manifest.display()))?;
        if parsed.version != WEBUI_VERSION {
            bail!(
                "{} package version mismatch at {}: found {}, but webui is {WEBUI_VERSION}.\nhelp: Reinstall matching WebUI and {package_name} packages.",
                sidecar.label(), manifest.display(), parsed.version
            );
        }
    }
    Ok(Some(native.join("bin").join(format!(
        "{}{}",
        sidecar.binary(),
        std::env::consts::EXE_SUFFIX
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_native_sidecars_in_hoisted_and_nested_installs() -> Result<()> {
        for sidecar in [Sidecar::Desktop, Sidecar::Press] {
            let temp = tempfile::tempdir()?;
            let root = fs::canonicalize(temp.path())?;
            assert_eq!(sidecar_near(sidecar, &root)?, None);
            let package = root.join("node_modules/@microsoft").join(sidecar.binary());
            fs::create_dir_all(&package)?;
            let manifest = format!(r#"{{"version":"{WEBUI_VERSION}"}}"#);
            fs::write(package.join("package.json"), &manifest)?;
            for modules in [root.join("node_modules"), package.join("node_modules")] {
                let native = modules.join(platform_package(sidecar));
                fs::create_dir_all(native.join("bin"))?;
                fs::write(native.join("package.json"), &manifest)?;
                let binary = native.join("bin").join(format!(
                    "{}{}",
                    sidecar.binary(),
                    std::env::consts::EXE_SUFFIX
                ));
                fs::write(&binary, [])?;
                for start in [
                    root.join("node_modules/.bin/webui"),
                    root.join("src/app"),
                    root.join("node_modules/.pnpm/core/node_modules/@microsoft/webui/bin/webui"),
                ] {
                    assert_eq!(sidecar_near(sidecar, &start)?, Some(binary.clone()));
                }
                fs::write(native.join("package.json"), r#"{"version":"0.0.0"}"#)?;
                assert!(sidecar_near(sidecar, &root)
                    .unwrap_err()
                    .to_string()
                    .contains("version mismatch"));
                fs::remove_dir_all(native)?;
            }
            assert!(sidecar_near(sidecar, &root)
                .unwrap_err()
                .to_string()
                .contains("optional dependencies"));
        }
        Ok(())
    }
}
