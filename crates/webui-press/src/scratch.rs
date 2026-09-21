// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Volume-aware scratch directories for Press builds.
//!
//! Press materializes its embedded template and built-in components into a
//! content-addressed cache, and those files include real TypeScript sources
//! (`template/index.ts`, `components/*/*.ts`) that become bundler inputs. The
//! state-projection manifest keys every input, output, and the manifest itself
//! as a path relative to one build root, so a bundle whose inputs live on a
//! different filesystem volume than its outputs has no expressible root and the
//! build fails.
//!
//! On Windows that happens whenever `TEMP`/`TMP` point at `C:\…` while the
//! build sits on another drive. This module keeps every Press scratch
//! directory on the build's own volume in that case, and otherwise keeps the
//! shared system temp directory so unrelated projects still share one cache.
//!
//! Staying on one volume also keeps the cache publish step a same-volume
//! `rename`, which is the only form that is atomic on Windows.

use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use std::{fs, path};

/// Project-local scratch directory used when the system temp directory lives
/// on another volume. Self-ignoring, so consuming repositories need no change.
const PROJECT_SCRATCH_DIR: &str = ".webui-press-cache";

/// Returns the base directory for Press scratch and cache directories.
///
/// `build_dir` is the configured output directory, which decides the volume:
/// it holds the generated entry points, the esbuild `outbase`/`outdir`, and the
/// projection manifest, so the build root always contains it. `out_dir` is
/// resolved independently of the config file (it may be absolute or relative to
/// the working directory), which is why it cannot be inferred from
/// `project_dir`.
///
/// `project_dir` is the config directory, and only supplies a home for the
/// fallback cache. A project whose sources and output directory are themselves
/// on different volumes cannot be expressed in one build root at all; no
/// scratch placement changes that, and `PROJ-C015` reports it.
///
/// Callers hoist the result for the whole build, so this stays uncached: a
/// process-wide cache would hand a second project the first project's base.
pub(crate) fn scratch_base(build_dir: &Path, project_dir: &Path) -> io::Result<PathBuf> {
    resolve_scratch_base(build_dir, project_dir, &std::env::temp_dir())
}

/// Resolves the scratch base against an explicit `system_temp`.
///
/// Split from [`scratch_base`] so tests can exercise cross-volume behavior
/// without a second drive letter.
fn resolve_scratch_base(
    build_dir: &Path,
    project_dir: &Path,
    system_temp: &Path,
) -> io::Result<PathBuf> {
    let build = path::absolute(build_dir)?;
    let system = path::absolute(system_temp)?;
    if same_volume(&system, &build) {
        return Ok(system);
    }
    let local = path::absolute(project_dir)?.join(PROJECT_SCRATCH_DIR);
    prepare_project_scratch(&local)?;
    Ok(local)
}

/// Creates the project-local scratch directory and marks it ignored.
///
/// The directory holds extracted build inputs, never sources, so it must never
/// reach a commit. Writing the ignore rule inside the directory itself keeps
/// consuming repositories free of Press-specific `.gitignore` entries.
fn prepare_project_scratch(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let ignore = directory.join(".gitignore");
    if ignore.exists() {
        return Ok(());
    }
    fs::write(ignore, b"*\n")
}

/// Reports whether two absolute paths live on the same filesystem volume.
///
/// Paths without a volume prefix (every Unix path) share one root by
/// definition, so they always compare equal.
fn same_volume(left: &Path, right: &Path) -> bool {
    volume_key(left) == volume_key(right)
}

/// Extracts a case-insensitive volume identity from a path prefix.
fn volume_key(path: &Path) -> Option<String> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    Some(match prefix.kind() {
        // `drive` is the ASCII byte, so widen before formatting: `u8` would
        // render as a number.
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            char::from(drive).to_ascii_lowercase().to_string()
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
            r"\\{}\{}",
            server.to_string_lossy().to_ascii_lowercase(),
            share.to_string_lossy().to_ascii_lowercase()
        ),
        Prefix::DeviceNS(name) | Prefix::Verbatim(name) => {
            name.to_string_lossy().to_ascii_lowercase()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn unique_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("webui-press-scratch-{name}-{}", std::process::id()))
    }

    #[test]
    fn same_volume_matches_identical_roots() {
        if cfg!(windows) {
            assert!(same_volume(
                Path::new(r"C:\Users\me\AppData\Local\Temp"),
                Path::new(r"C:\repos\site")
            ));
            assert!(same_volume(Path::new(r"c:\lower"), Path::new(r"C:\UPPER")));
            assert!(same_volume(
                Path::new(r"\\?\E:\repos\site"),
                Path::new(r"E:\repos\site")
            ));
        } else {
            assert!(same_volume(Path::new("/tmp"), Path::new("/home/me/site")));
        }
    }

    #[cfg(windows)]
    #[test]
    fn same_volume_separates_distinct_windows_volumes() {
        assert!(!same_volume(
            Path::new(r"C:\Users\me\AppData\Local\Temp"),
            Path::new(r"E:\repos\site")
        ));
        assert!(!same_volume(
            Path::new(r"\\server\share\site"),
            Path::new(r"C:\repos\site")
        ));
        assert!(same_volume(
            Path::new(r"\\server\share\site"),
            Path::new(r"\\SERVER\SHARE\other")
        ));
    }

    #[test]
    fn same_volume_keeps_system_temp_when_volumes_match() -> TestResult {
        let build = unique_dir("same-volume");
        fs::create_dir_all(&build)?;
        let system = std::env::temp_dir();
        let outcome = resolve_scratch_base(&build, &build, &system);
        let _ = fs::remove_dir_all(&build);
        assert_eq!(outcome?, path::absolute(&system)?);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn cross_volume_system_temp_falls_back_to_project_scratch() -> TestResult {
        let project = unique_dir("cross-volume");
        fs::create_dir_all(&project)?;
        // A UNC path is a distinct volume from any local drive, so this models
        // the `TEMP` on `C:` / build on `E:` case without a second drive.
        let foreign_temp = Path::new(r"\\webui-press-test\temp");
        let outcome = resolve_scratch_base(&project, &project, foreign_temp);
        let resolved = match outcome {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = fs::remove_dir_all(&project);
                return Err(error.into());
            }
        };
        let expected = path::absolute(&project)?.join(PROJECT_SCRATCH_DIR);
        let ignored = fs::read_to_string(resolved.join(".gitignore"));
        let _ = fs::remove_dir_all(&project);

        assert_eq!(resolved, expected);
        assert_eq!(ignored?, "*\n");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn output_volume_decides_the_base_not_the_config_directory() -> TestResult {
        // An output directory configured onto the system temp volume keeps the
        // shared cache even though the config directory is elsewhere: the
        // generated entry points, `outbase`/`outdir`, and the manifest all live
        // under the output directory, so that is the volume the build root
        // must be on.
        let system = std::env::temp_dir();
        let build = system.join("webui-press-output-volume");
        let project = Path::new(r"\\webui-press-test\project");
        assert_eq!(
            resolve_scratch_base(&build, project, &system)?,
            path::absolute(&system)?
        );
        Ok(())
    }

    #[test]
    fn project_scratch_preparation_is_idempotent() -> TestResult {
        let directory = unique_dir("idempotent");
        let _ = fs::remove_dir_all(&directory);
        let outcome: TestResult = (|| {
            prepare_project_scratch(&directory)?;
            fs::write(directory.join(".gitignore"), b"custom\n")?;
            prepare_project_scratch(&directory)?;
            assert_eq!(
                fs::read_to_string(directory.join(".gitignore"))?,
                "custom\n"
            );
            Ok(())
        })();
        let _ = fs::remove_dir_all(&directory);
        outcome
    }
}
