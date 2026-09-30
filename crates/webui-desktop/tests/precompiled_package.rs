// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

#[path = "../runtime/deployment.rs"]
mod deployment;

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use webui_desktop::{
    package_precompiled_host, DesktopError, DesktopPackageTarget, PrecompiledHostOptions,
    PrecompiledResource, ResourceKind,
};

fn native(path: &Path, target: DesktopPackageTarget, arm: bool) {
    let mut bytes = vec![0_u8; 128];
    match target {
        DesktopPackageTarget::MacosApp => {
            bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
            bytes[4..8].copy_from_slice(if arm { &[12, 0, 0, 1] } else { &[7, 0, 0, 1] });
        }
        DesktopPackageTarget::WindowsPortable => {
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c] = 64;
            bytes[64..68].copy_from_slice(b"PE\0\0");
            bytes[68..70].copy_from_slice(if arm { &[0x64, 0xaa] } else { &[0x64, 0x86] });
        }
        DesktopPackageTarget::LinuxPortable => {
            bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
            bytes[18..20].copy_from_slice(if arm { &[0xb7, 0] } else { &[0x3e, 0] });
        }
    }
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn windows_runtime_files(root: &Path) {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    fs::copy(
        runtime.join("win-x64").join(deployment::BOOTSTRAP_DLL),
        root.join(deployment::BOOTSTRAP_DLL),
    )
    .unwrap();
    for name in deployment::NOTICES {
        fs::copy(runtime.join(name), root.join(name)).unwrap();
    }
}

fn long_pe(path: &Path, machine: u16) {
    let pe_offset = 8_192_u32;
    let mut bytes = vec![0_u8; pe_offset as usize + 6];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&pe_offset.to_le_bytes());
    bytes[pe_offset as usize..pe_offset as usize + 4].copy_from_slice(b"PE\0\0");
    bytes[pe_offset as usize + 4..pe_offset as usize + 6].copy_from_slice(&machine.to_le_bytes());
    fs::write(path, bytes).unwrap();
}

#[test]
fn accepts_windows_pe_header_beyond_initial_scan_buffer() {
    let dir = TempDir::new().unwrap();
    let target = DesktopPackageTarget::WindowsPortable;
    let inputs = options(dir.path(), target);
    windows_runtime_files(dir.path());
    let host = dir.path().join("host.exe");
    long_pe(&host, 0x8664);

    let result = package_precompiled_host(inputs).unwrap();
    assert_eq!(fs::metadata(&result.host_path).unwrap().len(), 8_198);
}

#[test]
fn rejects_wrong_machine_after_long_pe_stub() {
    let dir = TempDir::new().unwrap();
    let inputs = options(dir.path(), DesktopPackageTarget::WindowsPortable);
    windows_runtime_files(dir.path());
    long_pe(&dir.path().join("host.exe"), 0xaa64);
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));
}

fn universal(path: &Path, offset: u32, size: u32, inner_arm: bool) {
    let mut bytes = vec![0_u8; 128];
    bytes[..4].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
    bytes[7] = 1; // One fat_arch record.
    bytes[8..12].copy_from_slice(&0x0100_0007_u32.to_be_bytes());
    bytes[16..20].copy_from_slice(&offset.to_be_bytes());
    bytes[20..24].copy_from_slice(&size.to_be_bytes());
    bytes[64..68].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    bytes[68..72].copy_from_slice(if inner_arm {
        &[12, 0, 0, 1]
    } else {
        &[7, 0, 0, 1]
    });
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn options(root: &Path, target: DesktopPackageTarget) -> PrecompiledHostOptions {
    let triple = match target {
        DesktopPackageTarget::MacosApp => "x86_64-apple-darwin",
        DesktopPackageTarget::WindowsPortable => "x86_64-pc-windows-msvc",
        DesktopPackageTarget::LinuxPortable => "x86_64-unknown-linux-gnu",
    };
    let host_exe = root.join(if target == DesktopPackageTarget::WindowsPortable {
        "host.exe"
    } else {
        "host"
    });
    native(&host_exe, target, false);
    let worker = root.join("worker");
    native(&worker, target, false);
    let sealed = root.join("sealed.webui");
    fs::write(&sealed, b"sealed consumer bundle").unwrap();
    configured(root, target, host_exe, triple)
        .add_resource(
            PrecompiledResource::new(worker, "workers/worker", ResourceKind::Executable).unwrap(),
        )
        .unwrap()
        .add_resource(
            PrecompiledResource::new(sealed, "sealed/webui.bundle", ResourceKind::Data).unwrap(),
        )
        .unwrap()
}

fn configured(
    root: &Path,
    target: DesktopPackageTarget,
    host: PathBuf,
    triple: &str,
) -> PrecompiledHostOptions {
    PrecompiledHostOptions::new(host, target, root.join("out"))
        .unwrap()
        .identity("com.example.player", "Goku Player", "1.2.3")
        .unwrap()
        .target_triple(triple)
        .unwrap()
}

#[cfg(unix)]
#[test]
#[allow(unsafe_code)]
fn packages_many_resources_under_child_fd_limit() {
    const CHILD: &str = "WEBUI_PACKAGE_FD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "packages_many_resources_under_child_fd_limit"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child packaging failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let dir = TempDir::new().unwrap();
    let host = dir.path().join("host");
    native(&host, DesktopPackageTarget::MacosApp, false);
    let mut options = configured(
        dir.path(),
        DesktopPackageTarget::MacosApp,
        host,
        "x86_64-apple-darwin",
    );
    // SAFETY: The limit changes only in this isolated child test process.
    unsafe {
        let mut original = std::mem::zeroed::<libc::rlimit>();
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut original), 0);
        assert!(original.rlim_cur >= 64);
        let limited = libc::rlimit {
            rlim_cur: 64,
            rlim_max: original.rlim_max,
        };
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &limited), 0);
    }
    for index in 0..80 {
        let name = format!("{index}.txt");
        let source = dir.path().join(&name);
        fs::write(&source, b"fixture").unwrap();
        options = options
            .add_resource(PrecompiledResource::new(source, name, ResourceKind::Data).unwrap())
            .unwrap();
    }
    let package = package_precompiled_host(options).unwrap();
    assert_eq!(package.resource_paths.len(), 80);
    for resource in package.resource_paths {
        assert_eq!(fs::read(resource).unwrap(), b"fixture");
    }
}

#[test]
fn builder_checks_required_identity_triple_icon_and_relative_mapping() {
    let dir = TempDir::new().unwrap();
    let target = DesktopPackageTarget::MacosApp;
    assert!(PrecompiledHostOptions::new("", target, dir.path()).is_err());
    let new = || {
        PrecompiledHostOptions::new(dir.path().join("host"), target, dir.path().join("out"))
            .unwrap()
    };
    assert!(new().identity("not-a-domain", "Player", "1.0.0").is_err());
    assert!(new()
        .identity("com.example.player", "Player", "1.0.0")
        .unwrap()
        .target_triple("x86_64-unknown-linux-gnu")
        .is_err());
    assert!(new().icon("icon.png").is_err());
    assert!(
        PrecompiledResource::new(dir.path().join("bundle"), "../escape", ResourceKind::Data)
            .is_err()
    );
    native(&dir.path().join("host"), target, false);
    let missing = new()
        .identity("com.example.player", "Player", "1.0.0")
        .unwrap();
    assert!(matches!(
        package_precompiled_host(missing),
        Err(DesktopError::PackageValidation { .. })
    ));
}

#[test]
fn macos_and_linux_map_host_worker_and_sealed_bundle_once_without_sources() {
    for target in [
        DesktopPackageTarget::MacosApp,
        DesktopPackageTarget::LinuxPortable,
    ] {
        let dir = TempDir::new().unwrap();
        let inputs = options(dir.path(), target);
        let source = dir.path().join("removed-source");
        fs::write(&source, b"not part of the layout").unwrap();
        fs::remove_file(&source).unwrap();
        let icon = dir
            .path()
            .join(if target == DesktopPackageTarget::MacosApp {
                "icon.icns"
            } else {
                "icon.png"
            });
        fs::write(&icon, b"icon").unwrap();
        let inputs = inputs.icon(icon).unwrap();
        let result = package_precompiled_host(inputs).unwrap();
        assert_eq!(
            fs::read(&result.resource_paths[1]).unwrap(),
            b"sealed consumer bundle"
        );
        assert_eq!(result.resource_paths.len(), 2);
        assert!(result.host_path.is_file());
        assert!(result.resource_paths[0].is_file());
        assert!(result.icon_path.as_ref().unwrap().is_file());
        if target == DesktopPackageTarget::MacosApp {
            assert!(result.host_path.ends_with("Contents/MacOS/host"));
            assert!(result.resource_paths[0].ends_with("Contents/MacOS/workers/worker"));
            assert!(result.resource_paths[1].ends_with("Contents/Resources/sealed/webui.bundle"));
            let plist = fs::read_to_string(result.output_path.join("Contents/Info.plist")).unwrap();
            assert!(plist.contains("com.example.player"));
            assert!(plist.contains("AppIcon.icns"));
        } else {
            assert!(result.host_path.ends_with("host"));
            assert!(result.resource_paths[0].ends_with("workers/worker"));
            assert!(result.resource_paths[1].ends_with("resources/sealed/webui.bundle"));
        }
        let mut found = Vec::new();
        let mut dirs = vec![result.output_path];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    dirs.push(entry.path());
                } else if fs::read(entry.path()).unwrap() == b"sealed consumer bundle" {
                    found.push(entry.path());
                }
            }
        }
        assert_eq!(found.len(), 1, "sealed bundle must not be copied twice");
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn accepts_actual_compiled_rust_host_without_source_tree() {
    let dir = TempDir::new().unwrap();
    let target = if cfg!(target_os = "macos") {
        DesktopPackageTarget::MacosApp
    } else {
        DesktopPackageTarget::LinuxPortable
    };
    let triple = match (target, cfg!(target_arch = "aarch64")) {
        (DesktopPackageTarget::MacosApp, true) => "aarch64-apple-darwin",
        (DesktopPackageTarget::MacosApp, false) => "x86_64-apple-darwin",
        (_, true) => "aarch64-unknown-linux-gnu",
        (_, false) => "x86_64-unknown-linux-gnu",
    };
    let host = std::env::current_exe().unwrap();
    let sealed = dir.path().join("sealed.webui");
    fs::write(&sealed, b"sealed").unwrap();
    let result = package_precompiled_host(
        PrecompiledHostOptions::new(host.clone(), target, dir.path().join("out"))
            .unwrap()
            .identity("com.example.actual", "Actual Rust Host", "1.0.0")
            .unwrap()
            .target_triple(triple)
            .unwrap()
            .add_resource(
                PrecompiledResource::new(sealed, "sealed.webui", ResourceKind::Data).unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&host).unwrap().len(),
        fs::metadata(&result.host_path).unwrap().len()
    );
    assert_eq!(fs::read(&result.resource_paths[0]).unwrap(), b"sealed");
    assert_eq!(result.resource_paths.len(), 1);
    if std::env::var_os("WEBUI_DESKTOP_KEEP_PACKAGE").is_some() {
        let retained = dir.keep();
        eprintln!(
            "precompiled package retained at {}",
            result.output_path.display()
        );
        eprintln!("test fixture root {}", retained.display());
    }
}

#[test]
fn rejects_architecture_paths_collisions_and_existing_output_before_writing() {
    let dir = TempDir::new().unwrap();
    let inputs = options(dir.path(), DesktopPackageTarget::LinuxPortable);
    native(
        &dir.path().join("host"),
        DesktopPackageTarget::LinuxPortable,
        true,
    );
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));

    assert!(matches!(
        PrecompiledResource::new(
            dir.path().join("sealed.webui"),
            "../escape",
            ResourceKind::Data
        ),
        Err(DesktopError::PackageValidation { .. })
    ));
    assert!(matches!(
        PrecompiledResource::new(
            dir.path().join("sealed.webui"),
            "sealed/./bundle",
            ResourceKind::Data
        ),
        Err(DesktopError::PackageValidation { .. })
    ));

    for alias in [
        "sealed/webui.bundle.",
        "sealed/webui.bundle ",
        "sealed./webui.bundle",
    ] {
        assert!(
            matches!(
                PrecompiledResource::new(
                    dir.path().join("sealed.webui"),
                    alias,
                    ResourceKind::Data
                ),
                Err(DesktopError::PackageValidation { .. })
            ),
            "{alias} must not alias a Windows destination"
        );
    }

    let inputs = options(dir.path(), DesktopPackageTarget::LinuxPortable);
    native(
        &dir.path().join("sealed.webui"),
        DesktopPackageTarget::WindowsPortable,
        false,
    );
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));

    let inputs = options(dir.path(), DesktopPackageTarget::LinuxPortable)
        .add_resource(
            PrecompiledResource::new(
                dir.path().join("sealed.webui"),
                "SEALED/WEBUI.BUNDLE",
                ResourceKind::Data,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));

    let inputs = options(dir.path(), DesktopPackageTarget::LinuxPortable);
    let output = dir.path().join("out/Goku-Player-linux-portable");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("keep"), b"user data").unwrap();
    assert!(package_precompiled_host(inputs).is_err());
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"user data");
}

#[test]
fn universal_macho_requires_a_real_bounded_matching_inner_slice() {
    let dir = TempDir::new().unwrap();
    let inputs = options(dir.path(), DesktopPackageTarget::MacosApp);
    universal(&dir.path().join("host"), 4096, 8, false);
    assert!(
        matches!(
            package_precompiled_host(inputs),
            Err(DesktopError::PackageValidation { .. })
        ),
        "fat table CPU without a bounded inner Mach-O is insufficient"
    );

    let inputs = options(dir.path(), DesktopPackageTarget::MacosApp);
    universal(&dir.path().join("host"), 64, 64, true);
    assert!(
        matches!(
            package_precompiled_host(inputs),
            Err(DesktopError::PackageValidation { .. })
        ),
        "inner slice CPU must match the selected fat architecture"
    );

    let inputs = options(dir.path(), DesktopPackageTarget::MacosApp);
    universal(&dir.path().join("host"), 64, 64, false);
    assert!(package_precompiled_host(inputs).is_ok());
}

#[cfg(unix)]
#[test]
fn rejects_symlink_input_and_output_overlap_without_deleting_source() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let _ = options(dir.path(), DesktopPackageTarget::LinuxPortable);
    let source = dir.path().join("sealed.webui");
    let link = dir.path().join("link");
    symlink(&source, &link).unwrap();
    let inputs = configured(
        dir.path(),
        DesktopPackageTarget::LinuxPortable,
        dir.path().join("host"),
        "x86_64-unknown-linux-gnu",
    )
    .add_resource(
        PrecompiledResource::new(
            dir.path().join("worker"),
            "workers/worker",
            ResourceKind::Executable,
        )
        .unwrap(),
    )
    .unwrap()
    .add_resource(
        PrecompiledResource::new(link, "sealed/webui.bundle", ResourceKind::Data).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));
    assert_eq!(fs::read(&source).unwrap(), b"sealed consumer bundle");

    let target = dir.path().join("out/Goku-Player-linux-portable");
    fs::create_dir_all(&target).unwrap();
    let nested = target.join("sealed.webui");
    fs::copy(&source, &nested).unwrap();
    let inputs = configured(
        dir.path(),
        DesktopPackageTarget::LinuxPortable,
        dir.path().join("host"),
        "x86_64-unknown-linux-gnu",
    )
    .add_resource(
        PrecompiledResource::new(
            dir.path().join("worker"),
            "workers/worker",
            ResourceKind::Executable,
        )
        .unwrap(),
    )
    .unwrap()
    .add_resource(
        PrecompiledResource::new(nested.clone(), "sealed/webui.bundle", ResourceKind::Data)
            .unwrap(),
    )
    .unwrap();
    let err = package_precompiled_host(inputs).unwrap_err();
    assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
    assert_eq!(fs::read(nested).unwrap(), b"sealed consumer bundle");
}

#[test]
fn windows_layout_preserves_bootstrap_and_notices_and_rejects_wrong_worker_arch() {
    let dir = TempDir::new().unwrap();
    let inputs = options(dir.path(), DesktopPackageTarget::WindowsPortable);
    windows_runtime_files(dir.path());
    native(
        &dir.path().join("worker"),
        DesktopPackageTarget::WindowsPortable,
        true,
    );
    assert!(matches!(
        package_precompiled_host(inputs),
        Err(DesktopError::PackageValidation { .. })
    ));
    let inputs = options(dir.path(), DesktopPackageTarget::WindowsPortable);
    let result = package_precompiled_host(inputs).unwrap();
    assert!(result.resource_paths[0].ends_with("workers/worker"));
    assert!(result.resource_paths[1].ends_with("resources/sealed/webui.bundle"));
    for name in std::iter::once(deployment::BOOTSTRAP_DLL).chain(deployment::NOTICES) {
        assert_eq!(
            fs::read(result.output_path.join(name)).unwrap(),
            fs::read(dir.path().join(name)).unwrap()
        );
    }
    let other = TempDir::new().unwrap();
    let arm_inputs = options(other.path(), DesktopPackageTarget::WindowsPortable)
        .target_triple("aarch64-pc-windows-msvc")
        .unwrap();
    native(
        &other.path().join("host.exe"),
        DesktopPackageTarget::WindowsPortable,
        true,
    );
    native(
        &other.path().join("worker"),
        DesktopPackageTarget::WindowsPortable,
        true,
    );
    windows_runtime_files(other.path());
    assert!(matches!(
        package_precompiled_host(arm_inputs),
        Err(DesktopError::PackageValidation { .. })
    ));
}
