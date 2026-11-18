// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::Path;

use crate::{resources, ClientBuildError};

pub(super) fn path(path: &Path) -> Result<String, ClientBuildError> {
    let path = resources::utf8(path)?;
    #[cfg(windows)]
    return windows_path(path);
    #[cfg(not(windows))]
    Ok(path.to_owned())
}

#[cfg(any(windows, test))]
fn windows_path(path: &str) -> Result<String, ClientBuildError> {
    // Resolver IDs treat '?' as a query delimiter, not a filesystem namespace.
    // Canonical verbatim paths remain authoritative for physical I/O/containment.
    let (prefix, tail) = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        ("//", unc)
    } else if let Some(disk) = path.strip_prefix(r"\\?\") {
        let bytes = disk.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return Err(ClientBuildError::Invalid(
                "browser resolver paths require a drive or UNC share; choose a project on a supported filesystem path".into(),
            ));
        }
        ("", disk)
    } else {
        ("", path)
    };
    let mut output = String::with_capacity(prefix.len() + tail.len());
    output.push_str(prefix);
    output.extend(tail.chars().map(|ch| if ch == '\\' { '/' } else { ch }));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_namespaces_become_complete_resolver_ids_without_query_delimiters(
    ) -> Result<(), ClientBuildError> {
        for (physical, module) in [
            (
                r"\\?\D:\a\webui\webui\index.ts",
                "D:/a/webui/webui/index.ts",
            ),
            (r"D:\a\webui\webui\index.ts", "D:/a/webui/webui/index.ts"),
            (
                r"\\?\UNC\server\share\source with spaces\雪.ts",
                "//server/share/source with spaces/雪.ts",
            ),
            (
                r"\\server\share\source with spaces\雪.ts",
                "//server/share/source with spaces/雪.ts",
            ),
        ] {
            let id = windows_path(physical)?;
            assert_eq!(id, module);
            assert!(!id.contains('?'));
        }
        assert!(windows_path(r"\\?\GLOBALROOT\Device\HarddiskVolume1\index.ts").is_err());
        Ok(())
    }
}
