// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve_icon_path(
    icon_path: Option<&Path>,
    bundle_root: Option<&Path>,
) -> Option<PathBuf> {
    let path = icon_path?;
    if let Some(root) = bundle_root {
        if !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return None;
        }
        let resolved = root.join(path).canonicalize().ok()?;
        return (resolved.starts_with(root) && resolved.is_file()).then_some(resolved);
    }
    path.is_file().then(|| path.to_path_buf())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::resolve_icon_path;
    use std::path::Path;

    #[test]
    fn missing_icon_is_ignored() {
        assert_eq!(resolve_icon_path(None, None), None);
    }

    #[test]
    fn bundled_relative_icon_resolves_inside_bundle() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let icon = root.join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("icon.icns")), Some(&root)),
            Some(icon)
        );
    }

    #[test]
    fn bundled_absolute_icon_is_rejected() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let icon = root.join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        assert_eq!(resolve_icon_path(Some(&icon), Some(&root)), None);
    }

    #[test]
    fn bundled_parent_traversal_is_rejected() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().join("bundle");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(dir.path().join("outside.icns"), b"icns").unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("../outside.icns")), Some(&root)),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn bundled_symlink_escape_is_rejected() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().join("bundle");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let outside = dir.path().join("outside.icns");
        std::fs::write(&outside, b"icns").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked.icns")).unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("linked.icns")), Some(&root)),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn bundled_symlink_within_root_is_allowed() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let icon = root.join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();
        std::os::unix::fs::symlink(&icon, root.join("linked.icns")).unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("linked.icns")), Some(&root)),
            Some(icon)
        );
    }

    #[test]
    fn source_absolute_icon_is_preserved() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let icon = dir.path().join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        assert_eq!(resolve_icon_path(Some(&icon), None), Some(icon));
    }

    #[test]
    fn missing_and_directory_icons_are_ignored() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("directory.icns")).unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("missing.icns")), Some(&root)),
            None
        );
        assert_eq!(
            resolve_icon_path(Some(Path::new("directory.icns")), Some(&root)),
            None
        );
    }
}
