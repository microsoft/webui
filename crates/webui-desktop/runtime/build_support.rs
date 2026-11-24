// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[path = "architecture.rs"]
mod architecture;
use crate::deployment::{BOOTSTRAP_DLL, NOTICES};
pub(crate) use architecture::runtime_architecture;

pub(crate) fn profile_directory(out: &Path) -> io::Result<PathBuf> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unexpected Cargo OUT_DIR {}; expected an absolute <target>/<profile>/build/<package>[/<unit>]/out path; build with Cargo",
                out.display()
            ),
        )
    };
    if !out.is_absolute()
        || out
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || out.file_name().is_none_or(|name| name != "out")
    {
        return Err(invalid());
    }
    let unit = out.parent().ok_or_else(invalid)?;
    let parent = unit.parent().ok_or_else(invalid)?;
    // Cargo 1.100 separates the package name from the build-unit hash.
    let build = if parent.file_name().is_some_and(|name| name == "build") {
        parent
    } else {
        parent.parent().ok_or_else(invalid)?
    };
    let profile = build.parent().ok_or_else(invalid)?;
    if unit.file_name().is_none()
        || build.file_name().is_none_or(|name| name != "build")
        || profile.file_name().is_none()
    {
        return Err(invalid());
    }
    // Validate the canonical layout too, so a redirected build directory cannot
    // cause staging outside the profile Cargo actually assigned.
    let canonical_out = out.canonicalize()?;
    let canonical_profile = profile.canonicalize()?;
    let expected = canonical_profile.join(out.strip_prefix(profile).map_err(|_| invalid())?);
    if canonical_out != expected {
        return Err(invalid());
    }
    Ok(canonical_profile)
}

pub(crate) fn stage_runtime(runtime: &Path, bootstrap: &Path, profile: &Path) -> io::Result<()> {
    let bootstrap_bytes = read_asset(bootstrap)?;
    let mut notices = Vec::with_capacity(NOTICES.len());
    for name in NOTICES {
        notices.push((name, read_asset(&runtime.join(name))?));
    }
    let deps = profile.join("deps");
    if fs::symlink_metadata(&deps).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing redirected Cargo deps directory {}",
                deps.display()
            ),
        ));
    }
    fs::create_dir_all(&deps)?;
    for destination in [profile, &deps] {
        write_if_changed(&destination.join(BOOTSTRAP_DLL), &bootstrap_bytes)?;
        for (name, bytes) in &notices {
            write_if_changed(&destination.join(name), bytes)?;
        }
    }
    Ok(())
}

fn read_asset(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot read Windows App SDK asset {}: {error}; restore the complete microsoft-webui-desktop crate runtime directory (repository maintainers: run python scripts\\acquire-windows-app-sdk.py)",
                path.display()
            ),
        )
    })?;
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "empty Windows App SDK asset {}; restore the runtime directory",
                path.display()
            ),
        ));
    }
    Ok(bytes)
}

pub(crate) fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing redirected Windows App SDK asset {}",
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
    fs::write(path, bytes).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot stage {}: {error}; close applications using an older bootstrap DLL and rebuild",
                path.display()
            ),
        )
    })
}
