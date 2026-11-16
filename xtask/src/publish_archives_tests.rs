// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::collections::BTreeSet;

fn fixture(root: &Path, platform: &PlatformEntry) {
    sdk::tests::fixture(root, platform);
    let build = root.join("build");
    fs::create_dir_all(&build).expect("build");
    fs::write(build.join(platform.ffi_lib), "native ffi").expect("FFI");
    fs::write(build.join("webui_ffi.dll.lib"), "import").expect("import");
    sdk::stage(root, platform, &build).expect("canonical SDK");
}

#[test]
fn all_archives_extract_to_sdk_root_with_identical_metadata_and_checksums() {
    let root = tempfile::tempdir().expect("root");
    for platform in PLATFORMS {
        fixture(root.path(), platform);
        let archive = stage(root.path(), platform).expect("archive");
        let extracted = tempfile::tempdir().expect("extract");
        let result = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(extracted.path())
            .output()
            .expect("tar");
        assert!(result.status.success());
        let original = sdk::directory(root.path(), platform);
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(extracted.path().join("sdk.json")).expect("metadata"))
                .expect("JSON");
        assert_eq!(
            fs::read(original.join("sdk.json")).expect("original"),
            fs::read(extracted.path().join("sdk.json")).expect("extracted")
        );
        let mut expected = BTreeSet::from(["sdk.json".to_string()]);
        for (path, hash) in metadata["files"].as_object().expect("files") {
            assert_eq!(
                hash,
                &sdk::sha256(&extracted.path().join(path)).expect("hash")
            );
            expected.insert(path.to_string());
        }
        let listing = Command::new("tar")
            .arg("-tzf")
            .arg(&archive)
            .output()
            .expect("list");
        let actual: BTreeSet<_> = String::from_utf8(listing.stdout)
            .expect("listing")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(actual, expected);
    }
    validate_release(root.path(), "3.1.2").expect("all six SDK archives");
    let archive = root
        .path()
        .join("publish/sdk")
        .join(name("3.1.2", &PLATFORMS[0]));
    fs::write(archive, "tampered").expect("tamper");
    assert!(validate_release(root.path(), "3.1.2")
        .expect_err("corrupt archive")
        .contains("checksum"));
}

#[test]
fn stale_incomplete_and_corrupt_sdk_cannot_leave_release_archive() {
    for relative in ["include/webui_ffi.h", "lib/libwebui_ffi.so", "sdk.json"] {
        let root = tempfile::tempdir().expect("root");
        let platform = &PLATFORMS[0];
        fixture(root.path(), platform);
        let archive = stage(root.path(), platform).expect("initial");
        let sdk = sdk::directory(root.path(), platform);
        fs::remove_file(sdk.join(relative)).expect("remove required input");
        assert!(stage(root.path(), platform).is_err());
        assert!(!archive.exists());
        assert!(!archive.with_extension("gz.sha256").exists());
    }
    let root = tempfile::tempdir().expect("root");
    fixture(root.path(), &PLATFORMS[0]);
    fs::write(
        root.path().join("crates/webui-ffi/include/webui_ffi.h"),
        "new header",
    )
    .expect("header");
    assert!(stage(root.path(), &PLATFORMS[0])
        .expect_err("stale header")
        .contains("stale SDK"));
}

#[test]
fn exports_preserve_other_targets_and_include_matching_sidecars() {
    let root = tempfile::tempdir().expect("root");
    let output = tempfile::tempdir().expect("output");
    for platform in &PLATFORMS[..2] {
        fixture(root.path(), platform);
        stage(root.path(), platform).expect("archive");
        export(root.path(), output.path(), platform).expect("export");
    }
    assert_eq!(
        super::super::count_regular_files(&output.path().join("publish/sdk")),
        4
    );
}
