// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::collections::BTreeMap;

fn snapshot(files: &webui_desktop_build::GeneratedFiles) -> BTreeMap<PathBuf, Vec<u8>> {
    files
        .rust
        .iter()
        .chain(&files.typescript)
        .chain([&files.manifest, &files.lock_file, &files.inventory])
        .map(|path| (path.clone(), fs::read(path).unwrap()))
        .collect()
}

#[cfg(unix)]
#[test]
fn current_artifact_symlink_never_overwrites_external_data() {
    let (dir, mut cfg) = temporary();
    generate(&cfg).unwrap();
    let sentinel = dir.path().join("unrelated-sentinel");
    fs::write(&sentinel, b"unrelated bytes").unwrap();
    let destination = cfg.rust_out.join("ipc.rs");
    fs::remove_file(&destination).unwrap();
    std::os::unix::fs::symlink(&sentinel, &destination).unwrap();
    for check in [true, false] {
        cfg.check = check;
        let result = generate(&cfg);
        assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated bytes");
        assert_eq!(result.unwrap_err().code(), "ipc-output-path");
    }
}

#[cfg(unix)]
#[test]
fn current_parent_symlink_is_rejected_even_when_bytes_match() {
    let (dir, mut cfg) = temporary();
    generate(&cfg).unwrap();
    let parent = cfg.ts_out.join("google/protobuf");
    let elsewhere = dir.path().join("unrelated-directory");
    fs::rename(&parent, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &parent).unwrap();
    let original = fs::read(elsewhere.join("empty.ts")).unwrap();
    cfg.check = true;
    let result = generate(&cfg);
    assert_eq!(fs::read(elsewhere.join("empty.ts")).unwrap(), original);
    assert_eq!(result.unwrap_err().code(), "ipc-output-path");
}

#[cfg(unix)]
#[test]
fn late_write_failure_preserves_previous_outputs_and_metadata() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut cfg) = temporary();
    let files = generate(&cfg).unwrap();
    let before = snapshot(&files);
    let sentinel = dir.path().join("unrelated-sentinel");
    fs::write(&sentinel, b"keep").unwrap();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    fs::write(
        &cfg.roots[0],
        source.replace(
            "message Label { string text = 1; }",
            "message Label { string text = 1; string extra = 2; }",
        ),
    )
    .unwrap();
    cfg.ts_out = dir.path().join("zz-unwritable");
    fs::create_dir(&cfg.ts_out).unwrap();
    fs::set_permissions(&cfg.ts_out, fs::Permissions::from_mode(0o555)).unwrap();
    let result = generate(&cfg);
    fs::set_permissions(&cfg.ts_out, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert!(
        snapshot(&files) == before,
        "publication advanced original artifacts after failure"
    );
    assert_eq!(fs::read(sentinel).unwrap(), b"keep");
}

#[test]
fn generated_constructor_and_normalized_symbols_are_rejected_before_publication() {
    let cases = [
        ("rpc LabelFor(Item)", "rpc New(Item)"),
        ("rpc LabelFor(Item)", "rpc NEW_(Item)"),
        ("service Host {", "service Messages {"),
        ("rpc Save(Item)", "rpc Rpc(Item)"),
        ("rpc Save(Item)", "rpc Host(Item)"),
        ("rpc Save(Item)", "rpc Constructor(Item)"),
        ("rpc Save(Item)", "rpc Selected_(Item)"),
        ("rpc Save(Item)", "rpc _(Item)"),
        ("rpc Save(Item)", "rpc _123(Item)"),
    ];
    for (from, to) in cases {
        let (_dir, cfg) = temporary();
        let source = fs::read_to_string(&cfg.roots[0]).unwrap();
        fs::write(&cfg.roots[0], source.replace(from, to)).unwrap();
        let error = generate(&cfg).unwrap_err();
        assert_eq!(error.code(), "ipc-name-collision", "{error}");
        assert!(!cfg.lock_file.exists());
        assert!(!cfg.rust_out.exists());
    }
}

#[test]
fn escaped_keywords_remain_valid_but_normalized_keyword_duplicates_fail() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    fs::write(
        &cfg.roots[0],
        source.replace("rpc Save(Item)", "rpc Type(Item)"),
    )
    .unwrap();
    generate(&cfg).unwrap();
    assert!(fs::read_to_string(cfg.rust_out.join("ipc.rs"))
        .unwrap()
        .contains("fn r#type("));
    let (_dir, cfg) = temporary();
    fs::write(
        &cfg.roots[0],
        source
            .replace("rpc Save(Item)", "rpc Type(Item)")
            .replace("rpc Selected(Item)", "rpc type_(Item)"),
    )
    .unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-name-collision");
    assert!(!cfg.lock_file.exists());
}

#[cfg(unix)]
#[test]
fn metadata_root_and_dangling_symlinks_are_rejected() {
    for target in ["lock", "inventory", "root", "dangling"] {
        let (dir, cfg) = temporary();
        let files = generate(&cfg).unwrap();
        let path = match target {
            "lock" => cfg.lock_file.clone(),
            "inventory" => files.inventory.clone(),
            "root" => cfg.rust_out.clone(),
            _ => cfg.rust_out.join("ipc.rs"),
        };
        let elsewhere = dir.path().join("outside-output");
        if target == "dangling" {
            fs::remove_file(&path).unwrap();
        } else {
            fs::rename(&path, &elsewhere).unwrap();
        }
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert_eq!(
            generate(&cfg).unwrap_err().code(),
            "ipc-output-path",
            "{target}"
        );
        assert_eq!(elsewhere.exists(), target != "dangling");
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn stale_symlink_and_directory_fail_without_advancing_lock() {
    for symlink in [false, true] {
        let (dir, cfg) = temporary();
        let files = generate(&cfg).unwrap();
        let old_lock = fs::read(&cfg.lock_file).unwrap();
        let old_inventory = fs::read(&files.inventory).unwrap();
        let sentinel = dir.path().join("unrelated");
        fs::write(&sentinel, "// @generated\nunrelated bytes").unwrap();
        let source = fs::read_to_string(&cfg.roots[0]).unwrap();
        // Disconnect Label and its module by renaming the application file.
        let next = dir.path().join("renamed.proto");
        fs::write(&next, source).unwrap();
        let mut cfg = cfg;
        cfg.roots[0] = next;
        let obsolete = cfg.ts_out.join("application.ts");
        fs::remove_file(&obsolete).unwrap();
        if symlink {
            std::os::unix::fs::symlink(&sentinel, &obsolete).unwrap();
        } else {
            fs::create_dir(&obsolete).unwrap();
        }
        assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-output-path");
        assert_eq!(fs::read(&cfg.lock_file).unwrap(), old_lock);
        assert_eq!(fs::read(&files.inventory).unwrap(), old_inventory);
        assert_eq!(
            fs::read(sentinel).unwrap(),
            b"// @generated\nunrelated bytes"
        );
    }
}
