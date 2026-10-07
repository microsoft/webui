// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const ASSET_NAMES: [&str; 6] = [
    "native-bootstrap.js",
    "desktop-runtime.js",
    "local-native-bootstrap.js",
    "local-desktop-runtime.js",
    "linux-local-entry.js",
    "linux-local-mediator.js",
];

pub(crate) fn stage_assets(
    manifest_dir: &Path,
    out_dir: &Path,
    source_override: Option<&OsStr>,
) -> io::Result<(PathBuf, PathBuf)> {
    let source = locate_assets(manifest_dir, source_override)?;
    let destination = out_dir.join("webui-desktop-ipc");
    if fs::symlink_metadata(&destination).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing redirected desktop browser asset directory {}",
                destination.display()
            ),
        ));
    }
    fs::create_dir_all(&destination)?;
    for name in ASSET_NAMES {
        let input = source.join(name);
        let bytes = read_asset(&input)?;
        write_if_changed(&destination.join(name), &bytes)?;
    }
    Ok((source, destination))
}

fn locate_assets(manifest_dir: &Path, source_override: Option<&OsStr>) -> io::Result<PathBuf> {
    if let Some(source) = source_override {
        let source = PathBuf::from(source);
        validate_asset_set(&source)?;
        return Ok(source);
    }
    let Some(workspace) = manifest_dir.parent().and_then(Path::parent) else {
        return Err(missing_assets(manifest_dir));
    };
    let generated = workspace
        .join("target")
        .join("webui-desktop-assets")
        .join("ipc");
    if complete_asset_set(&generated) {
        return Ok(generated);
    }
    let packaged = manifest_dir.join("assets").join("ipc");
    validate_asset_set(&packaged)?;
    Ok(packaged)
}

fn complete_asset_set(path: &Path) -> bool {
    ASSET_NAMES
        .iter()
        .all(|name| fs::metadata(path.join(name)).is_ok_and(|metadata| metadata.len() > 0))
}

fn validate_asset_set(path: &Path) -> io::Result<()> {
    if complete_asset_set(path) {
        Ok(())
    } else {
        Err(missing_assets(path))
    }
}

fn missing_assets(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "desktop browser assets are missing from {}; repository builds must run `cargo xtask desktop-assets` before enabling `application-ipc`, while published crates must contain assets/ipc",
            path.display()
        ),
    )
}

fn read_asset(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot read desktop browser asset {}: {error}",
                path.display()
            ),
        )
    })?;
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("desktop browser asset {} is empty", path.display()),
        ));
    }
    Ok(bytes)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing redirected desktop browser asset {}",
                path.display()
            ),
        ));
    }
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::{stage_assets, ASSET_NAMES};
    use std::fs;
    use tempfile::TempDir;

    fn write_assets(path: &std::path::Path, marker: &[u8]) {
        fs::create_dir_all(path).unwrap();
        for name in ASSET_NAMES {
            let mut bytes = marker.to_vec();
            bytes.extend_from_slice(name.as_bytes());
            fs::write(path.join(name), bytes).unwrap();
        }
    }

    #[test]
    fn repository_outputs_take_precedence_over_transient_package_assets() {
        let temporary = TempDir::new().unwrap();
        let manifest = temporary.path().join("workspace/crates/webui-desktop");
        let packaged = manifest.join("assets/ipc");
        let generated = temporary
            .path()
            .join("workspace/target/webui-desktop-assets/ipc");
        let out = temporary.path().join("out");
        write_assets(&packaged, b"package:");
        write_assets(&generated, b"workspace:");
        fs::create_dir_all(&out).unwrap();

        let (_, staged) = stage_assets(&manifest, &out, None).unwrap();
        assert!(fs::read(staged.join(ASSET_NAMES[0]))
            .unwrap()
            .starts_with(b"workspace:"));
    }

    #[test]
    fn workspace_outputs_support_repository_builds() {
        let temporary = TempDir::new().unwrap();
        let manifest = temporary.path().join("workspace/crates/webui-desktop");
        let generated = temporary
            .path()
            .join("workspace/target/webui-desktop-assets/ipc");
        let out = temporary.path().join("out");
        write_assets(&generated, b"workspace:");
        fs::create_dir_all(&out).unwrap();

        let (_, staged) = stage_assets(&manifest, &out, None).unwrap();
        for name in ASSET_NAMES {
            assert_eq!(
                fs::read(staged.join(name)).unwrap(),
                fs::read(generated.join(name)).unwrap()
            );
        }
    }

    #[test]
    fn missing_assets_report_the_generation_command() {
        let temporary = TempDir::new().unwrap();
        let manifest = temporary.path().join("workspace/crates/webui-desktop");
        let out = temporary.path().join("out");
        fs::create_dir_all(&out).unwrap();

        let error = stage_assets(&manifest, &out, None).unwrap_err();
        assert!(error.to_string().contains("cargo xtask desktop-assets"));
    }
}
