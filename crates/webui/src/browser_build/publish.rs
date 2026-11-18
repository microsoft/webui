// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use webui_protocol::projection_manifest::ProjectionManifest;

use super::{resources, ClientBuild, ClientBuildError};

const STAGING: &str = ".webui-bundle-staging";

pub(super) fn temporary_roots(
    bundle: &ClientBuild,
    manifest_path: &Path,
) -> Result<BTreeSet<PathBuf>, ClientBuildError> {
    let manifest = relative(bundle.project(), manifest_path)?;
    bundle
        .files
        .keys()
        .chain([&manifest])
        .map(|path| {
            let relative = relative(bundle.project(), path)?;
            let parent = relative.parent().ok_or_else(|| {
                ClientBuildError::Invalid("native output must have a parent directory".into())
            })?;
            Ok(bundle.project().join(parent).join(STAGING))
        })
        .collect()
}

pub(super) fn write(
    bundle: &ClientBuild,
    manifest_path: &Path,
) -> Result<ProjectionManifest, ClientBuildError> {
    let relative_manifest = relative(bundle.project(), manifest_path)?;
    let manifest = bundle.projection_manifest(&relative_manifest)?;
    let json = serde_json::to_vec(&manifest).map_err(|error| {
        ClientBuildError::Invalid(format!("could not serialize native proof: {error}"))
    })?;
    let mut inputs = BTreeSet::new();
    inputs.extend(bundle.dependencies.iter().map(|path| destination_key(path)));
    for (id, input) in &bundle.metadata.inputs {
        if !id.starts_with('\0') {
            inputs.insert(destination_key(&bundle.project().join(&input.source)));
        }
    }
    let mut destinations = BTreeSet::new();
    let temporary_roots = temporary_roots(bundle, &relative_manifest)?;
    for root in &temporary_roots {
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(redirected(root)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(root, source)),
        }
    }
    for input in &inputs {
        if temporary_roots
            .iter()
            .any(|root| inside(input, &destination_key(root)))
        {
            return Err(ClientBuildError::Invalid(
                "a native input is inside an owned publication staging directory; move the input before publishing".into(),
            ));
        }
    }
    for path in bundle.files.keys().chain([&relative_manifest]) {
        let relative = relative(bundle.project(), path)?;
        let absolute = bundle.project().join(&relative);
        if temporary_roots
            .iter()
            .any(|root| inside(&destination_key(&absolute), &destination_key(root)))
        {
            return Err(ClientBuildError::Invalid(
                "native outputs cannot use owned publication staging directories".into(),
            ));
        }
        if !destinations.insert(destination_key(&relative)) {
            return Err(ClientBuildError::Invalid(
                "native manifest and browser output paths must not collide".into(),
            ));
        }
        let canonical = fs::canonicalize(&absolute).ok();
        if inputs.contains(&destination_key(&absolute))
            || canonical
                .as_ref()
                .is_some_and(|path| inputs.contains(&destination_key(path)))
        {
            return Err(ClientBuildError::Invalid(
                "native output cannot overwrite one of its captured source inputs; choose another outDir or manifest path".into(),
            ));
        }
        check_destination(bundle.project(), &relative)?;
    }
    let old =
        protect_existing_manifest(bundle.project(), &bundle.project().join(&relative_manifest))?;
    protect_existing_outputs(bundle, old.as_ref())?;
    for (path, bytes) in &bundle.files {
        write_file(bundle.project(), path, bytes)?;
    }
    // The proof is the commit marker: never publish it before matching bytes.
    write_file(bundle.project(), &relative_manifest, &json)?;
    Ok(manifest)
}

fn inside(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn destination_key(path: &Path) -> String {
    // Reject portable case-only collisions even on case-sensitive hosts:
    // generated bundles must also be safe on default APFS/NTFS installations.
    resources::portable(path).to_lowercase()
}

fn protect_existing_manifest(
    project: &Path,
    path: &Path,
) -> Result<Option<ProjectionManifest>, ClientBuildError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(path, source)),
        Ok(_) => {}
    }
    // A user-selected manifest name must never turn an existing configuration
    // or authored document into a proof file, including resolver inputs whose
    // identities the pinned upstream does not expose.
    let (file, _) = resources::open_bounded(path, 64 * 1024 * 1024)?;
    let bytes = resources::read_bounded(file, path, 64 * 1024 * 1024)?;
    let old: ProjectionManifest = serde_json::from_slice(&bytes).map_err(|_| {
        ClientBuildError::Invalid(
            "refusing to replace an existing non-manifest file; select a new manifest path".into(),
        )
    })?;
    old.validate()?;
    let root = path
        .parent()
        .ok_or_else(|| {
            ClientBuildError::Invalid("previous native proof has no parent directory".into())
        })?
        .join(&old.root);
    if fs::canonicalize(root).map_err(|source| io_error(path, source))? != project {
        return Err(ClientBuildError::Invalid(
            "previous native proof belongs to another project".into(),
        ));
    }
    Ok(Some(old))
}

fn protect_existing_outputs(
    bundle: &ClientBuild,
    previous: Option<&ProjectionManifest>,
) -> Result<(), ClientBuildError> {
    for path in bundle.files.keys() {
        if fs::symlink_metadata(bundle.project().join(path)).is_ok()
            && previous.is_none_or(|proof| !proof.outputs.contains_key(&resources::portable(path)))
        {
            return Err(ClientBuildError::Invalid(format!(
                "refusing to replace existing file {} not owned by the previous native proof; choose a fresh outDir or explicitly remove obsolete generated outputs",
                path.display()
            )));
        }
    }
    Ok(())
}

fn relative(project: &Path, path: &Path) -> Result<PathBuf, ClientBuildError> {
    let path = if path.is_absolute() {
        path.strip_prefix(project).map_err(|_| {
            ClientBuildError::Invalid(
                "native output must be project-relative or canonical-contained inside the project"
                    .into(),
            )
        })?
    } else {
        path
    };
    resources::output_path(project, path)?;
    Ok(path.to_owned())
}

fn check_destination(project: &Path, relative: &Path) -> Result<(), ClientBuildError> {
    let mut path = project.to_owned();
    check_directory(&path)?;
    let parent = relative.parent().ok_or_else(|| {
        ClientBuildError::Invalid("native output must have a parent directory".into())
    })?;
    for part in parent.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(redirected(&path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    check_file(&project.join(relative))
}

fn check_directory(path: &Path) -> Result<(), ClientBuildError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(redirected(path));
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<(), ClientBuildError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(redirected(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn ensure_parent(project: &Path, relative: &Path) -> Result<PathBuf, ClientBuildError> {
    check_directory(project)?;
    let mut directory = project.to_owned();
    let parent = relative.parent().ok_or_else(|| {
        ClientBuildError::Invalid("native output must have a parent directory".into())
    })?;
    for part in parent.components() {
        directory.push(part);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io_error(&directory, source)),
        }
        check_directory(&directory)?;
    }
    let canonical = fs::canonicalize(&directory).map_err(|source| io_error(&directory, source))?;
    if !canonical.starts_with(project) {
        return Err(redirected(&directory));
    }
    Ok(directory)
}

fn write_file(project: &Path, relative: &Path, bytes: &[u8]) -> Result<(), ClientBuildError> {
    let directory = ensure_parent(project, relative)?;
    let staging = directory.join(STAGING);
    match fs::create_dir(&staging) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(io_error(&staging, source)),
    }
    check_directory(&staging)?;
    let path = project.join(relative);
    check_file(&path)?;
    // A same-filesystem staging subtree gives complete-file replacement without
    // noisy random names beside watched inputs. Never remove unknown contents.
    let mut temporary =
        tempfile::NamedTempFile::new_in(&staging).map_err(|source| io_error(&path, source))?;
    temporary
        .write_all(bytes)
        .map_err(|source| io_error(&path, source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))
            .map_err(|source| io_error(&path, source))?;
    }
    check_file(&path)?;
    temporary
        .persist(&path)
        .map_err(|error| io_error(&path, error.error))?;
    Ok(())
}

#[cold]
#[inline(never)]
fn redirected(path: &Path) -> ClientBuildError {
    ClientBuildError::Invalid(format!(
        "native output path {} is redirected or has an invalid file type; remove the link or choose a regular output location",
        path.display()
    ))
}

#[cold]
#[inline(never)]
fn io_error(path: &Path, source: std::io::Error) -> ClientBuildError {
    ClientBuildError::Write {
        path: path.to_owned(),
        source,
    }
}
