// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use std::ffi::OsString;

#[test]
fn only_windows_drive_verbatim_prefixes_are_adapted() {
    assert_eq!(
        tools::protoc_path(Path::new(r"\\?\D:\schema with spaces\application.proto")).unwrap(),
        PathBuf::from(r"D:\schema with spaces\application.proto")
    );
    assert_eq!(
        tools::protoc_path(Path::new("/tmp/unchanged root/application.proto")).unwrap(),
        PathBuf::from("/tmp/unchanged root/application.proto")
    );
    for path in [
        r"\\?\UNC\server\share\application.proto",
        r"\\?\Volume{test}\application.proto",
    ] {
        assert_eq!(
            tools::protoc_path(Path::new(path)).unwrap_err().code(),
            "ipc-tool-path"
        );
    }
}

#[test]
fn compiler_options_keep_paths_with_spaces_in_one_argument() {
    let dir = tempfile::Builder::new()
        .prefix("output paths & spaces ")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let output = dir.path().join("descriptor with spaces.bin");
    let mut command = Command::new("protoc");
    tools::path_option(&mut command, "--descriptor_set_out=", &output).unwrap();
    let mut expected = OsString::from("--descriptor_set_out=");
    expected.push(tools::protoc_path(&std::path::absolute(output).unwrap()).unwrap());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![expected.as_os_str()]
    );
}

#[cfg(windows)]
#[test]
fn windows_root_and_output_arguments_are_plain_absolute_drive_paths() {
    let root = Path::new(r"\\?\D:\a\webui\schema space\application.proto");
    let mut command =
        proto_command(Path::new("protoc.exe"), &[root.parent().unwrap().into()]).unwrap();
    tools::path_option(
        &mut command,
        "--descriptor_set_out=",
        Path::new(r"\\?\D:\a\webui\output space\schema.bin"),
    )
    .unwrap();
    command.arg(tools::protoc_path(root).unwrap());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "-I",
            r"D:\a\webui\schema space",
            r"--descriptor_set_out=D:\a\webui\output space\schema.bin",
            r"D:\a\webui\schema space\application.proto",
        ]
    );
}

#[test]
fn protoc_include_arguments_do_not_expose_windows_verbatim_prefixes() {
    let cwd = std::env::current_dir().unwrap();
    let command = proto_command(
        Path::new("protoc"),
        &[
            PathBuf::from(r"\\?\D:\a\webui\webui\schema with spaces"),
            PathBuf::from(r"\\?\D:\a\webui\webui\.webui-ipc-1\include"),
        ],
    )
    .unwrap();
    let actual: Vec<_> = command.get_args().map(OsString::from).collect();
    assert_eq!(
        actual,
        [
            OsString::from("-I"),
            OsString::from(r"D:\a\webui\webui\schema with spaces"),
            OsString::from("-I"),
            OsString::from(r"D:\a\webui\webui\.webui-ipc-1\include"),
        ]
    );
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}
