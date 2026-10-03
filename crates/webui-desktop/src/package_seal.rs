// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded resource seals for precompiled package mappings.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{binary, open_input, output::PackageOutput, validation, CopySpec};
use crate::error::{DesktopError, Result};

pub(super) struct ResourceSeal {
    length: u64,
    sha256: [u8; 32],
}

impl ResourceSeal {
    pub(super) fn from_handle(file: &mut File, source: &Path) -> Result<Self> {
        file.seek(SeekFrom::Start(0))
            .map_err(|error| io_error("seeking package resource", source, error))?;
        let expected = file
            .metadata()
            .map_err(|error| io_error("checking package resource size", source, error))?
            .len();
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 16 * 1024];
        let mut length = 0u64;
        let mut bounded = file.by_ref().take(expected);
        loop {
            let count = bounded
                .read(&mut buffer)
                .map_err(|error| io_error("sealing package resource", source, error))?;
            if count == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(count).map_err(|_| {
                    validation(format!(
                        "package resource {} is too large",
                        source.display()
                    ))
                })?)
                .ok_or_else(|| {
                    validation(format!(
                        "package resource {} is too large",
                        source.display()
                    ))
                })?;
            digest.update(&buffer[..count]);
        }
        let mut extra = [0u8; 1];
        let grew = file
            .read(&mut extra)
            .map_err(|error| io_error("checking package resource size", source, error))?
            != 0;
        if length != expected || grew {
            return Err(changed_resource(source, "changed size during preflight"));
        }
        Ok(Self {
            length,
            sha256: digest.finalize().into(),
        })
    }
}

pub(super) fn copy_sealed(
    output: &mut PackageOutput,
    spec: CopySpec<'_>,
    seal: &ResourceSeal,
) -> Result<()> {
    let mut input = open_input(spec.source, spec.root, "resource")?;
    #[cfg(unix)]
    if spec.executable {
        super::require_executable(&input, spec.source)?;
    }
    let permissions = input
        .metadata()
        .map_err(|error| io_error("reading resource permissions", spec.source, error))?
        .permissions();
    let mut dest = output.create_file(spec.relative)?;
    let copied = std::io::copy(&mut input.by_ref().take(seal.length), &mut dest)
        .map_err(|error| io_error("copying package resource", spec.source, error))?;
    let mut extra = [0u8; 1];
    let has_extra = input
        .read(&mut extra)
        .map_err(|error| io_error("checking package resource length", spec.source, error))?
        != 0;
    if copied != seal.length || has_extra {
        return Err(changed_resource(
            spec.source,
            "changed size after preflight",
        ));
    }
    dest.set_permissions(permissions).map_err(|error| {
        io_error(
            "setting package resource permissions",
            spec.destination,
            error,
        )
    })?;
    verify_output(&mut dest, spec.destination, seal)?;
    if spec.executable {
        binary::validate_binary(&mut dest, spec.destination, spec.target)?;
    } else {
        binary::validate_data(&mut dest, spec.destination, spec.target)?;
    }
    Ok(())
}

fn verify_output(dest: &mut File, path: &Path, seal: &ResourceSeal) -> Result<()> {
    dest.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("rewinding package resource output", path, error))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut remaining = seal.length;
    while remaining > 0 {
        let count = usize::try_from(remaining.min(16 * 1024))
            .map_err(|_| validation(format!("package resource {} is too large", path.display())))?;
        let read = dest
            .read(&mut buffer[..count])
            .map_err(|error| io_error("verifying package resource output", path, error))?;
        if read == 0 {
            return Err(changed_resource(path, "output was truncated"));
        }
        digest.update(&buffer[..read]);
        remaining -= u64::try_from(read)
            .map_err(|_| validation(format!("package resource {} is too large", path.display())))?;
    }
    let mut extra = [0u8; 1];
    let has_extra = dest
        .read(&mut extra)
        .map_err(|error| io_error("checking package resource output length", path, error))?
        != 0;
    let observed: [u8; 32] = digest.finalize().into();
    if has_extra || observed != seal.sha256 {
        return Err(changed_resource(
            path,
            "changed after preflight or output differs",
        ));
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn changed_resource(path: &Path, reason: &str) -> DesktopError {
    DesktopError::PackageValidation {
        message: format!("package resource {} {reason}", path.display()),
        help: "Keep mapped resources immutable through packaging, check output writes, and retry into a new output directory; a partial new package may remain",
    }
}

#[cold]
#[inline(never)]
fn io_error(context: &str, path: &Path, source: std::io::Error) -> DesktopError {
    DesktopError::Io {
        context: format!("{context} {}", path.display()),
        source,
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("input");
        fs::write(&source, b"original bytes").unwrap();
        let output_path = dir.path().join("Output.app");
        (dir, source, output_path)
    }

    fn copy(source: &Path, root: &Path, seal: &ResourceSeal) -> Result<()> {
        let mut output = PackageOutput::create(root.parent().unwrap(), root.file_name().unwrap())?;
        let dest = root.join("Contents/Resources/input");
        let target = binary::parse_target("x86_64-apple-darwin", binary::Format::Mach)?;
        copy_sealed(
            &mut output,
            CopySpec::new(source, &dest, root, target, false)?,
            seal,
        )
    }

    #[test]
    fn rejects_changed_bytes_even_with_the_same_size_and_mtime() {
        let (_dir, source, root) = setup();
        let mut input = open_input(&source, &root, "resource").unwrap();
        let seal = ResourceSeal::from_handle(&mut input, &source).unwrap();
        let modified = input.metadata().unwrap().modified().unwrap();
        drop(input);
        let file = fs::OpenOptions::new().write(true).open(&source).unwrap();
        file.set_len(0).unwrap();
        std::io::Write::write_all(&mut &file, b"replaced bytes").unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(fs::metadata(&source).unwrap().len(), seal.length);
        assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), modified);
        let error = copy(&source, &root, &seal).unwrap_err();
        assert!(matches!(
            error,
            DesktopError::PackageValidation { help, .. } if help.contains("immutable")
        ));
    }

    #[test]
    fn rejects_a_symlink_substituted_after_preflight() {
        let (dir, source, root) = setup();
        let mut input = open_input(&source, &root, "resource").unwrap();
        let seal = ResourceSeal::from_handle(&mut input, &source).unwrap();
        drop(input);
        let replacement = dir.path().join("other");
        fs::write(&replacement, b"original bytes").unwrap();
        fs::rename(&source, dir.path().join("previous")).unwrap();
        symlink(&replacement, &source).unwrap();
        assert!(matches!(
            copy(&source, &root, &seal),
            Err(DesktopError::PackageValidation { .. })
        ));
        assert_eq!(fs::read(&replacement).unwrap(), b"original bytes");
    }

    #[test]
    fn rejects_regular_file_replacement_and_size_drift() {
        for bytes in [
            b"replaced bytes".as_slice(),
            b"short",
            b"original bytes plus",
        ] {
            let (dir, source, root) = setup();
            let mut input = open_input(&source, &root, "resource").unwrap();
            let seal = ResourceSeal::from_handle(&mut input, &source).unwrap();
            drop(input);
            let replacement = dir.path().join("replacement");
            fs::write(&replacement, bytes).unwrap();
            fs::rename(&replacement, &source).unwrap();
            assert!(matches!(
                copy(&source, &root, &seal),
                Err(DesktopError::PackageValidation { .. })
            ));
        }
    }

    #[test]
    fn rejects_corrupted_destination_bytes() {
        let (_dir, source, root) = setup();
        let mut input = open_input(&source, &root, "resource").unwrap();
        let seal = ResourceSeal::from_handle(&mut input, &source).unwrap();
        let mut output =
            PackageOutput::create(root.parent().unwrap(), root.file_name().unwrap()).unwrap();
        let dest_path = root.join("Contents/Resources/input");
        let mut dest = output
            .create_file(Path::new("Contents/Resources/input"))
            .unwrap();
        input.seek(SeekFrom::Start(0)).unwrap();
        std::io::copy(&mut input, &mut dest).unwrap();
        dest.seek(SeekFrom::Start(0)).unwrap();
        std::io::Write::write_all(&mut dest, b"corrupt").unwrap();
        assert!(matches!(
            verify_output(&mut dest, &dest_path, &seal),
            Err(DesktopError::PackageValidation { .. })
        ));
    }
}
