// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::{
    fs,
    path::{Path, PathBuf},
};
use webui_desktop_build::{generate, GenerateConfig};

#[path = "support/publication.rs"]
mod publication;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../webui-desktop/tests/fixtures/typed-ipc")
}
fn config(base: &Path) -> GenerateConfig {
    GenerateConfig {
        roots: vec![base.join("application.proto")],
        includes: vec![base.to_owned()],
        rust_out: base.join("rust"),
        ts_out: base.join("ts"),
        lock_file: base.join("ipc-schema.lock.json"),
        check: false,
        protoc: None,
        ts_proto_plugin: Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../packages/webui-desktop/node_modules/.bin/protoc-gen-ts_proto"),
        ),
    }
}
fn temporary() -> (tempfile::TempDir, GenerateConfig) {
    // Task artifacts stay inside the worktree, including compiler staging.
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::copy(
        fixture().join("application.proto"),
        dir.path().join("application.proto"),
    )
    .unwrap();
    fs::copy(
        fixture().join("types.proto"),
        dir.path().join("types.proto"),
    )
    .unwrap();
    let cfg = config(dir.path());
    (dir, cfg)
}

#[test]
fn deterministic_and_check_mode_is_read_only() {
    let (_dir, mut cfg) = temporary();
    let first = generate(&cfg).unwrap();
    let lock = fs::read(&cfg.lock_file).unwrap();
    let second = generate(&cfg).unwrap();
    assert_eq!(first.schema_hash, second.schema_hash);
    assert_eq!(lock, fs::read(&cfg.lock_file).unwrap());
    cfg.check = true;
    generate(&cfg).unwrap();
    let path = cfg.rust_out.join("ipc.rs");
    fs::write(&path, "drift").unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-drift");
    assert_eq!(fs::read_to_string(path).unwrap(), "drift");
    assert_eq!(fs::read(&cfg.lock_file).unwrap(), lock);
}

#[test]
fn comments_do_not_change_schema_hash() {
    let (_dir, cfg) = temporary();
    let first = generate(&cfg).unwrap();
    let path = &cfg.roots[0];
    let source = fs::read_to_string(path).unwrap();
    fs::write(path, format!("{source}\n// changed comment\n")).unwrap();
    assert_eq!(first.schema_hash, generate(&cfg).unwrap().schema_hash);
}

#[test]
fn invalid_schemas_have_stable_codes() {
    let cases = [
        ("= 1101;", "= 1;", "ipc-method-id"),
        ("= 2001;", "= 1101;", "ipc-duplicate-id"),
        ("rpc Save(Item)", "rpc Save(stream Item)", "ipc-streaming"),
        (
            "message Label { string text = 1; }",
            "message Label { Label child = 1; }",
            "ipc-recursion",
        ),
        (
            "option (webui.ipc.notification) = true;",
            "option (webui.ipc.notification) = false;",
            "ipc-notification",
        ),
        ("option (webui.ipc.receiver) = HOST;", "", "ipc-receiver"),
        (
            "message Label { string text = 1; }",
            "message Label { Other other = 1; }\nmessage Other { Label parent = 1; }",
            "ipc-recursion",
        ),
        (
            "message Label { string text = 1; }",
            "message Label { string text = 1; }\nmessage Unused { Unused cycle = 1; }",
            "ipc-recursion",
        ),
        (
            "message Label { string text = 1; }",
            "message Label { webui.ipc.Notification bad = 1; }",
            "ipc-unsupported-message",
        ),
        (
            "message Label { string text = 1; }",
            "import \"google/protobuf/any.proto\";\nmessage Label { google.protobuf.Any bad = 1; }",
            "ipc-unsupported-message",
        ),
        (
            "message Label { string text = 1; }",
            "import \"google/protobuf/descriptor.proto\";\nextend google.protobuf.FieldOptions { bool extra = 52000; }\nmessage Label { string text = 1 [(extra) = true]; }",
            "ipc-unsupported-option",
        ),
    ];
    for (from, to, code) in cases {
        let (_dir, cfg) = temporary();
        let source = fs::read_to_string(&cfg.roots[0]).unwrap();
        fs::write(&cfg.roots[0], source.replace(from, to)).unwrap();
        let error = generate(&cfg).unwrap_err();
        assert_eq!(error.code(), code, "{error}");
        assert!(!cfg.lock_file.exists());
    }
}

#[test]
fn deep_acyclic_schemas_are_bounded() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    let mut chain = String::from("message Label { Level0 child = 1; }\n");
    for i in 0..16 {
        chain.push_str(&format!(
            "message Level{i} {{ Level{} child = 1; }}\n",
            i + 1
        ));
    }
    chain.push_str("message Level16 { string value = 1; }\n");
    fs::write(
        &cfg.roots[0],
        source.replace("message Label { string text = 1; }", &chain),
    )
    .unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-depth");
}

#[test]
fn method_ids_and_enum_reservations_are_permanent() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    generate(&cfg).unwrap();
    fs::write(&cfg.roots[0], source.replace("= 1101;", "= 1201;")).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-method-id-change");
    fs::write(&cfg.roots[0], source.replace("READY = 1;", "")).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-enum-retirement");
    fs::write(
        &cfg.roots[0],
        source.replace("READY = 1;", "reserved 1; reserved \"READY\";"),
    )
    .unwrap();
    generate(&cfg).unwrap();
    fs::write(&cfg.roots[0], source).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-retired-enum");
}

#[test]
fn descriptor_order_does_not_change_hash() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    let first = generate(&cfg).unwrap();
    let reordered = source.replace("  uint64 id = 1;\n", "").replace(
        "  int64 minimum = 4;",
        "  int64 minimum = 4;\n  uint64 id = 1;",
    );
    fs::write(&cfg.roots[0], reordered).unwrap();
    assert_eq!(first.schema_hash, generate(&cfg).unwrap().schema_hash);
}

#[test]
fn missing_tools_report_actionable_errors_without_outputs() {
    let (_dir, mut cfg) = temporary();
    cfg.protoc = Some(cfg.rust_out.join("missing-protoc"));
    let error = generate(&cfg).unwrap_err();
    assert_eq!(error.code(), "ipc-tool");
    assert!(error.to_string().contains("GenerateConfig.protoc"));
    assert!(!cfg.lock_file.exists());
}

#[test]
fn paths_with_spaces_and_removed_codec_modules() {
    let (dir, mut cfg) = temporary();
    let directory = dir.path().join("schemas with spaces");
    fs::create_dir(&directory).unwrap();
    fs::rename(&cfg.roots[0], directory.join("application.proto")).unwrap();
    fs::rename(
        dir.path().join("types.proto"),
        directory.join("types.proto"),
    )
    .unwrap();
    cfg.roots[0] = directory.join("application.proto");
    cfg.includes.clear(); // root parents are inferred
    generate(&cfg).unwrap();
    assert!(cfg.ts_out.join("application.ts").exists());
    fs::rename(&cfg.roots[0], directory.join("renamed.proto")).unwrap();
    cfg.roots[0] = directory.join("renamed.proto");
    cfg.check = true;
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-drift");
    assert!(cfg.ts_out.join("application.ts").exists());
    cfg.check = false;
    generate(&cfg).unwrap();
    assert!(!cfg.ts_out.join("application.ts").exists());
    assert!(cfg.ts_out.join("renamed.ts").exists());
}

#[test]
fn proto2_and_notification_request_are_rejected() {
    let (_dir, cfg) = temporary();
    let source = r#"syntax = "proto2";
import "webui/ipc/options.proto";
option (webui.ipc.contract_name) = "proto2";
option (webui.ipc.contract_major) = 1;
message Request { optional string text = 1; }
service Host {
  option (webui.ipc.receiver) = HOST;
  rpc Save(Request) returns (Request) { option (webui.ipc.id) = 1101; }
}"#;
    fs::write(&cfg.roots[0], source).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-proto3");
    let source = fs::read_to_string(fixture().join("application.proto")).unwrap();
    fs::write(
        &cfg.roots[0],
        source.replace("rpc Save(Item)", "rpc Save(webui.ipc.Notification)"),
    )
    .unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-notification");
}

#[test]
fn disconnected_types_do_not_lose_compatibility_history() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    generate(&cfg).unwrap();
    let disconnected = source
        .replace(
            "rpc LabelFor(Item) returns (Label)",
            "rpc LabelFor(Item) returns (google.protobuf.Empty)",
        )
        .replace("contract_major) = 1", "contract_major) = 2");
    fs::write(&cfg.roots[0], disconnected).unwrap();
    generate(&cfg).unwrap();
    let reconnected = source
        .replace(
            "message Label { string text = 1; }",
            "message Label { bytes text = 1; }",
        )
        .replace("contract_major) = 1", "contract_major) = 3");
    fs::write(&cfg.roots[0], reconnected).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-field-type");
}

#[test]
fn retired_ids_cannot_be_reused() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    generate(&cfg).unwrap();
    let removed = source.replace(
        "  rpc Save(Item) returns (google.protobuf.Empty) { option (webui.ipc.id) = 1101; }\n",
        "",
    );
    fs::write(&cfg.roots[0], &removed).unwrap();
    generate(&cfg).unwrap();
    fs::write(&cfg.roots[0], &source).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-retired-id");
}

#[test]
fn removed_fields_require_reservations() {
    let (_dir, cfg) = temporary();
    let source = fs::read_to_string(&cfg.roots[0]).unwrap();
    generate(&cfg).unwrap();
    fs::write(
        &cfg.roots[0],
        source.replace("  optional bool enabled = 11;", ""),
    )
    .unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-field-retirement");
    fs::write(
        &cfg.roots[0],
        source.replace(
            "  optional bool enabled = 11;",
            "  reserved 11;\n  reserved \"enabled\";",
        ),
    )
    .unwrap();
    generate(&cfg).unwrap();
    fs::write(&cfg.roots[0], source).unwrap();
    assert_eq!(generate(&cfg).unwrap_err().code(), "ipc-retired-field");
}

#[test]
fn shared_fixture_matches_generator() {
    let mut cfg = config(&fixture());
    cfg.check = std::env::var_os("WEBUI_UPDATE_IPC_FIXTURE").is_none();
    if !cfg.check {
        // These directories exclusively contain outputs from this generator.
        for directory in [&cfg.rust_out, &cfg.ts_out] {
            if directory.exists() {
                fs::remove_dir_all(directory).unwrap();
            }
        }
    }
    generate(&cfg).unwrap();
}
