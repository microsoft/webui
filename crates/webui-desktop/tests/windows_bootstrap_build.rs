// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

#[path = "../runtime/build_support.rs"]
mod build_support;
#[path = "../runtime/deployment.rs"]
mod deployment;

use std::fs;
use std::path::PathBuf;

fn test_dir() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn selects_target_architecture_not_host_architecture() {
    assert_eq!(
        build_support::runtime_architecture("x86_64").unwrap(),
        "win-x64"
    );
    assert_eq!(
        build_support::runtime_architecture("aarch64").unwrap(),
        "win-arm64"
    );
    assert_eq!(
        build_support::runtime_architecture("x86").unwrap(),
        "win-x86"
    );
    assert!(build_support::runtime_architecture("riscv64").is_err());
}

#[test]
fn derives_profile_for_host_cross_and_custom_profiles() {
    let dir = test_dir();
    for profile in ["debug", "aarch64-pc-windows-msvc/release", "custom"] {
        let profile = dir.path().join(profile);
        let out = profile.join("build").join("desktop-123").join("out");
        fs::create_dir_all(&out).unwrap();
        assert_eq!(
            build_support::profile_directory(&out).unwrap(),
            profile.canonicalize().unwrap()
        );
    }
    assert!(build_support::profile_directory(&dir.path().join("out")).is_err());
    assert!(build_support::profile_directory(&PathBuf::from("build/desktop/out")).is_err());
}

#[test]
fn stages_exact_bootstrap_and_notices_in_profile_and_deps() {
    let dir = test_dir();
    let profile = dir.path().join("debug");
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
    for arch in ["x86_64", "aarch64", "x86"] {
        let bootstrap = runtime
            .join(build_support::runtime_architecture(arch).unwrap())
            .join(deployment::BOOTSTRAP_DLL);
        build_support::stage_runtime(&runtime, &bootstrap, &profile).unwrap();
        for destination in [&profile, &profile.join("deps")] {
            assert_eq!(
                fs::read(destination.join(deployment::BOOTSTRAP_DLL)).unwrap(),
                fs::read(&bootstrap).unwrap()
            );
            for name in deployment::NOTICES {
                assert_eq!(
                    fs::read(destination.join(name)).unwrap(),
                    fs::read(runtime.join(name)).unwrap()
                );
            }
        }
    }
}

#[test]
fn unchanged_files_are_not_rewritten() {
    let dir = test_dir();
    let file = dir.path().join("bootstrap.dll");
    build_support::write_if_changed(&file, b"bootstrap").unwrap();
    let before = fs::metadata(&file).unwrap().modified().unwrap();
    build_support::write_if_changed(&file, b"bootstrap").unwrap();
    assert_eq!(before, fs::metadata(&file).unwrap().modified().unwrap());
    build_support::write_if_changed(&file, b"updated").unwrap();
    assert_eq!(fs::read(file).unwrap(), b"updated");
}

#[cfg(windows)]
#[test]
fn unchanged_bootstrap_can_be_staged_while_writes_are_locked() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = test_dir();
    let file = dir.path().join("bootstrap.dll");
    build_support::write_if_changed(&file, b"bootstrap").unwrap();
    // FILE_SHARE_READ allows comparison, but deliberately denies write/delete.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&file)
        .unwrap();
    build_support::write_if_changed(&file, b"bootstrap").unwrap();
    let error = build_support::write_if_changed(&file, b"updated").unwrap_err();
    assert!(error.to_string().contains("close applications"));
    drop(locked);
}

#[test]
fn missing_asset_does_not_change_existing_staged_files() {
    let dir = test_dir();
    let runtime = dir.path().join("runtime");
    let profile = dir.path().join("debug");
    fs::create_dir_all(&runtime).unwrap();
    fs::create_dir_all(&profile).unwrap();
    let bootstrap = runtime.join(deployment::BOOTSTRAP_DLL);
    fs::write(&bootstrap, b"new bootstrap").unwrap();
    fs::write(profile.join(deployment::BOOTSTRAP_DLL), b"old bootstrap").unwrap();
    let error = build_support::stage_runtime(&runtime, &bootstrap, &profile).unwrap_err();
    assert!(error.to_string().contains("restore"));
    assert_eq!(
        fs::read(profile.join(deployment::BOOTSTRAP_DLL)).unwrap(),
        b"old bootstrap"
    );
    assert!(!profile.join("deps").exists());
}
