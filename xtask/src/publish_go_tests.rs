// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn fixture(root: &Path) {
    fs::create_dir_all(root.join("go")).expect("source");
    fs::write(
        root.join("go/go.mod"),
        format!("module {MODULE}\n\ngo 1.24\n"),
    )
    .expect("mod");
    fs::write(
        root.join("go/webui.go"),
        "package webui\nconst Version = \"3.1.2\"\n",
    )
    .expect("Go");
    fs::write(root.join("LICENSE"), "MIT fixture").expect("license");
}

#[test]
fn caller_owned_proxy_does_not_modify_shared_release_outputs_or_source() {
    let root = tempfile::tempdir().expect("workspace");
    let output = tempfile::Builder::new()
        .prefix("isolated Go proxy ")
        .tempdir()
        .expect("proxy output");
    fixture(root.path());
    let source_manifest = fs::read(root.path().join("go/go.mod")).expect("source manifest");
    fs::create_dir_all(root.path().join("publish/go")).expect("shared release outputs");
    fs::write(root.path().join("publish/go/preserve"), "shared").expect("shared marker");
    fs::write(output.path().join("other-module-marker"), "other").expect("unrelated proxy");
    crate::publish::stage_go_proxy(root.path(), "3.1.2", output.path())
        .expect("caller-owned proxy");
    validate_at(output.path(), "3.1.2").expect("complete Go proxy");
    assert!(!proxy(root.path()).exists());
    assert_eq!(
        fs::read_to_string(root.path().join("publish/go/preserve")).expect("shared marker"),
        "shared"
    );
    assert_eq!(
        fs::read_to_string(output.path().join("other-module-marker")).expect("unrelated proxy"),
        "other"
    );
    assert_eq!(
        fs::read(root.path().join("go/go.mod")).expect("unchanged manifest"),
        source_manifest
    );
}

#[test]
fn proxy_downloads_and_imports_from_external_app_without_workspace_or_node() {
    let root = tempfile::Builder::new()
        .prefix("Go proxy with spaces ")
        .tempdir()
        .expect("root");
    fixture(root.path());
    stage(root.path(), "3.1.2").expect("proxy");
    let archive = proxy(root.path()).join("v3.1.2.zip");
    let first = fs::read(&archive).expect("archive");
    stage(root.path(), "3.1.2").expect("repeat");
    assert_eq!(first, fs::read(&archive).expect("repeat archive"));
    let app = tempfile::tempdir().expect("external app");
    fs::write(
        app.path().join("go.mod"),
        format!("module consumer\n\ngo 1.24\nrequire {MODULE} v3.1.2\n"),
    )
    .expect("app mod");
    fs::write(app.path().join("main.go"), format!("package main\nimport (\"fmt\"; webui \"{MODULE}\")\nfunc main() {{fmt.Print(webui.Version)}}\n")).expect("app");
    let go_binary = Command::new("go")
        .args(["env", "GOROOT"])
        .output()
        .expect("Go");
    assert!(go_binary.status.success());
    let goroot = String::from_utf8(go_binary.stdout).expect("GOROOT");
    let bin = Path::new(goroot.trim()).join("bin");
    // PATH deliberately contains Go only: neither Node nor npm is available.
    let run = |args: &[&str]| {
        Command::new(bin.join("go"))
            .args(args)
            .current_dir(app.path())
            .env("PATH", &bin)
            .env("GOWORK", "off")
            .env("GOTOOLCHAIN", "local")
            .env(
                "GOPROXY",
                crate::publish::go_proxy_url(&root.path().join("publish/go"))
                    .expect("file proxy URL"),
            )
            .env("GOSUMDB", "off")
            .env("GONOSUMDB", "*")
            .env("GONOPROXY", "")
            .env("GOPRIVATE", "")
            .env("GOFLAGS", "")
            .env("CGO_ENABLED", "0")
            .env("GOMODCACHE", app.path().join("modcache"))
            .env("GOCACHE", app.path().join("cache"))
            .output()
            .expect("isolated Go consumer")
    };
    let download = run(&["mod", "download", "-json", &format!("{MODULE}@v3.1.2")]);
    assert!(
        download.status.success(),
        "{}",
        String::from_utf8_lossy(&download.stdout)
    );
    let downloaded: serde_json::Value =
        serde_json::from_slice(&download.stdout).expect("download JSON");
    assert!(
        Path::new(downloaded["Dir"].as_str().expect("module directory"))
            .join("LICENSE")
            .is_file()
    );
    let output = run(&["run", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"3.1.2");
}

#[test]
fn rejects_module_path_major_version_and_generated_native_payloads() {
    let root = tempfile::tempdir().expect("root");
    fixture(root.path());
    assert!(stage(root.path(), "4.0.0")
        .expect_err("major mismatch")
        .contains("v3"));
    assert!(stage(root.path(), "../3.1.2").is_err());
    assert!(stage(root.path(), "3.1.1")
        .expect_err("release mismatch")
        .contains("Version must match"));
    fs::write(
        root.path().join("go/go.mod"),
        "module wrong.example/go/v3\n",
    )
    .expect("wrong module");
    assert!(stage(root.path(), "3.1.2")
        .expect_err("module mismatch")
        .contains("module path mismatch"));
    fixture(root.path());
    stage(root.path(), "3.1.2").expect("initial");
    fs::write(root.path().join("go/libwebui_ffi.so"), "generated").expect("library");
    assert!(stage(root.path(), "3.1.2")
        .expect_err("native payload")
        .contains("payload forbidden"));
    assert!(!proxy(root.path()).exists());
}

#[cfg(unix)]
#[test]
fn rejects_symlink_source_escape_and_case_collisions() {
    let root = tempfile::tempdir().expect("root");
    fixture(root.path());
    std::os::unix::fs::symlink(
        root.path().join("LICENSE"),
        root.path().join("go/escape.go"),
    )
    .expect("symlink");
    assert!(stage(root.path(), "3.1.2")
        .expect_err("escape")
        .contains("symlink"));
    fs::remove_file(root.path().join("go/escape.go")).expect("remove");
    fs::write(root.path().join("go/license"), "conflicting").expect("collision");
    // LICENSE is supplied from root and must not collide on case-insensitive hosts.
    assert!(stage(root.path(), "3.1.2").is_err());
}
