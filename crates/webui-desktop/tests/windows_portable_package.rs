// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

#[path = "../runtime/deployment.rs"]
mod deployment;

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use webui_desktop::{
    package_desktop_bundle, BundleIntegrity, DesktopBundleManifest, DesktopError,
    DesktopPackageOptions, DesktopPackageTarget, DesktopShellConfig, WindowOptions,
};

fn fixture(arch: &str) -> TempDir {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let test_root = crate_root.join("target");
    fs::create_dir_all(&test_root).unwrap();
    let dir = tempfile::tempdir_in(test_root).unwrap();
    let bundle = dir.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    let manifest = DesktopBundleManifest {
        manifest_version: DesktopBundleManifest::VERSION,
        app_id: "test.windows-package".into(),
        app_name: "Package".into(),
        version: "1.0.0".into(),
        publisher: "WebUI".into(),
        entry: "index.html".into(),
        plugin: None,
        protocol_path: "protocol.bin".into(),
        state_path: None,
        assets_dir: "assets".into(),
        ipc_schema: None,
        window: WindowOptions::default(),
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
        integrity: BundleIntegrity::default(),
    };
    fs::write(
        bundle.join("manifest.webui-desktop.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(bundle.join("protocol.bin"), b"protocol").unwrap();
    fs::write(dir.path().join("runner.exe"), b"custom runner").unwrap();
    let runtime = crate_root.join("runtime");
    fs::copy(
        runtime.join(arch).join(deployment::BOOTSTRAP_DLL),
        dir.path().join(deployment::BOOTSTRAP_DLL),
    )
    .unwrap();
    for name in deployment::NOTICES {
        fs::copy(runtime.join(name), dir.path().join(name)).unwrap();
    }
    dir
}

fn options(root: &Path, target: DesktopPackageTarget) -> DesktopPackageOptions {
    DesktopPackageOptions {
        bundle_dir: root.join("bundle"),
        out_dir: root.join("out"),
        runner_exe: root.join("runner.exe"),
        target,
    }
}

#[test]
fn windows_portable_preserves_each_architecture_bootstrap_and_all_notices() {
    for arch in ["win-x64", "win-arm64"] {
        let dir = fixture(arch);
        let result =
            package_desktop_bundle(options(dir.path(), DesktopPackageTarget::WindowsPortable))
                .unwrap();
        for name in std::iter::once("runner.exe")
            .chain(std::iter::once(deployment::BOOTSTRAP_DLL))
            .chain(deployment::NOTICES)
        {
            assert_eq!(
                fs::read(result.output_path.join(name)).unwrap(),
                fs::read(dir.path().join(name)).unwrap()
            );
        }
        assert!(result
            .output_path
            .join("resources/webui/protocol.bin")
            .is_file());
    }
}

#[test]
fn missing_windows_inputs_preserve_existing_output() {
    for name in std::iter::once("runner.exe")
        .chain(std::iter::once(deployment::BOOTSTRAP_DLL))
        .chain(deployment::NOTICES)
    {
        let dir = fixture("win-x64");
        let output = dir.path().join("out/Package-windows-portable");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("keep.txt"), b"previous package").unwrap();
        fs::remove_file(dir.path().join(name)).unwrap();
        let error =
            package_desktop_bundle(options(dir.path(), DesktopPackageTarget::WindowsPortable))
                .unwrap_err();
        assert!(error.to_string().contains(name), "{error}");
        assert!(error.to_string().contains("rebuild the runner"), "{error}");
        assert_eq!(
            fs::read(output.join("keep.txt")).unwrap(),
            b"previous package"
        );
    }
}

#[test]
fn empty_windows_input_preserves_existing_output() {
    let dir = fixture("win-x64");
    fs::write(dir.path().join(deployment::BOOTSTRAP_DLL), b"").unwrap();
    let output = dir.path().join("out/Package-windows-portable");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("keep.txt"), b"previous package").unwrap();
    assert!(
        package_desktop_bundle(options(dir.path(), DesktopPackageTarget::WindowsPortable)).is_err()
    );
    assert!(output.join("keep.txt").is_file());
}

#[test]
fn runtime_inputs_are_protected_from_output_overlap() {
    for name in std::iter::once(deployment::BOOTSTRAP_DLL).chain(deployment::NOTICES) {
        let dir = fixture("win-x64");
        let mut options = options(dir.path(), DesktopPackageTarget::WindowsPortable);
        options.out_dir = dir.path().join(name);
        let error = package_desktop_bundle(options).unwrap_err();
        assert!(
            matches!(error, DesktopError::OutputPathOverlap { .. }),
            "{error}"
        );
        assert!(dir.path().join(name).is_file());
    }
}

#[test]
fn other_platforms_do_not_copy_or_require_windows_files() {
    for target in [
        DesktopPackageTarget::LinuxPortable,
        DesktopPackageTarget::MacosApp,
    ] {
        let dir = fixture("win-x64");
        let result = package_desktop_bundle(options(dir.path(), target)).unwrap();
        assert!(!result.output_path.join(deployment::BOOTSTRAP_DLL).exists());
        for name in deployment::NOTICES {
            assert!(!result.output_path.join(name).exists());
            fs::remove_file(dir.path().join(name)).unwrap();
        }
        fs::remove_file(dir.path().join(deployment::BOOTSTRAP_DLL)).unwrap();
        assert!(package_desktop_bundle(options(dir.path(), target)).is_ok());
    }
}
