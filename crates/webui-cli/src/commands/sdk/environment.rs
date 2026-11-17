// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use super::{invalid, SdkError};

pub(super) fn configure(command: &mut Command, root: &Path) -> Result<(), SdkError> {
    let include = quote_flag("-I", &root.join("include"))?;
    let library = root.join("lib");
    let link = quote_flag("-L", &library)?;
    command.env("WEBUI_SDK_DIR", root).env("CGO_ENABLED", "1");
    for (name, value) in [
        ("CGO_CPPFLAGS", &include),
        ("CGO_CFLAGS", &include),
        ("CGO_LDFLAGS", &link),
    ] {
        let mut flags = OsString::from(value);
        if let Some(inherited) = std::env::var_os(name).filter(|value| !value.is_empty()) {
            flags.push(" ");
            flags.push(inherited);
        }
        command.env(name, flags);
    }
    let loader = loader_variable();
    let inherited = std::env::var_os(&loader);
    let paths = std::iter::once(library).chain(
        inherited
            .as_deref()
            .into_iter()
            .flat_map(std::env::split_paths),
    );
    let joined = std::env::join_paths(paths).map_err(|error| {
        invalid(
            root,
            format!("cannot construct SDK loader search path: {error}"),
        )
    })?;
    command.env(loader, joined);
    Ok(())
}

fn quote_flag(prefix: &str, path: &Path) -> Result<String, SdkError> {
    let path_text = path
        .to_str()
        .ok_or_else(|| invalid(path, "cgo SDK paths must be valid UTF-8"))?;
    let path_text = if let Some(unc) = path_text.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        path_text
            .strip_prefix(r"\\?\")
            .unwrap_or(path_text)
            .replace('\\', "/")
    };
    serde_json::to_string(&format!("{prefix}{path_text}"))
        .map_err(|error| invalid(path, error.to_string()))
}

fn loader_variable() -> OsString {
    if cfg!(windows) {
        std::env::vars_os()
            .find(|(name, _)| name.eq_ignore_ascii_case(OsStr::new("PATH")))
            .map_or_else(|| OsString::from("PATH"), |(name, _)| name)
    } else if cfg!(target_os = "macos") {
        OsString::from("DYLD_LIBRARY_PATH")
    } else {
        OsString::from("LD_LIBRARY_PATH")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgo_flags_preserve_windows_paths_with_spaces_without_verbatim_prefixes(
    ) -> Result<(), SdkError> {
        for (path, expected) in [
            (
                r"\\?\C:\SDK with spaces\include",
                "\"-IC:/SDK with spaces/include\"",
            ),
            (
                r"\\?\UNC\server\share\SDK\include",
                "\"-I//server/share/SDK/include\"",
            ),
            (
                "/tmp/SDK with spaces/include",
                "\"-I/tmp/SDK with spaces/include\"",
            ),
        ] {
            assert_eq!(quote_flag("-I", Path::new(path))?, expected);
        }
        Ok(())
    }
}
