// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Runtime Dock icon (`NSApplication.applicationIconImage`) support.
//!
//! Packaged `.app` bundles get their icon from `CFBundleIconFile` in
//! `Info.plist`, which AppKit reads before any code runs. A bare runner binary
//! launched from `cargo run` has no bundle, so the Dock falls back to a generic
//! executable icon. Setting the icon at runtime closes that gap and matches the
//! Windows backend, where the embedded icon resource applies to source launches
//! too.

use std::path::{Path, PathBuf};

use objc2::AnyThread;
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSString;

/// Apply `icon_path` as the running application's Dock icon, if it resolves to
/// a readable image.
///
/// A missing, unreadable, or undecodable icon leaves the Dock icon untouched:
/// artwork is cosmetic and must never prevent the window from opening.
pub(super) fn install_app_icon(app: &NSApplication, icon_path: Option<&Path>) {
    let base = crate::find_packaged_resources_dir().unwrap_or_else(|| PathBuf::from("."));
    let Some(path) = resolve_icon_path(icon_path, &base) else {
        return;
    };
    let path_string = NSString::from_str(&path.to_string_lossy());
    if let Some(image) = NSImage::initWithContentsOfFile(NSImage::alloc(), &path_string) {
        // SAFETY: AppKit calls this during `applicationDidFinishLaunching:`, so
        // the receiver is the main-thread `NSApplication`. The image is a
        // freshly initialized `NSImage` that AppKit retains for the Dock tile.
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

/// Resolve a configured icon path against `base`, returning it only when the
/// file exists.
///
/// `DesktopShellConfig::icon_path` is documented as bundle-root relative, so a
/// relative path is joined onto the runtime's resource root. Absolute paths -
/// which source-mode hosts supply directly - are used unchanged.
fn resolve_icon_path(icon_path: Option<&Path>, base: &Path) -> Option<PathBuf> {
    let path = icon_path?;
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    resolved.is_file().then_some(resolved)
}

#[cfg(test)]
mod tests {
    use super::resolve_icon_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn returns_none_without_a_configured_icon() {
        assert_eq!(resolve_icon_path(None, Path::new("/bundle")), None);
    }

    #[test]
    fn joins_relative_paths_onto_the_resource_root() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("icon.icns")), dir.path()),
            Some(icon)
        );
    }

    #[test]
    fn keeps_absolute_paths_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        assert_eq!(
            resolve_icon_path(Some(icon.as_path()), Path::new("/unused")),
            Some(icon)
        );
    }

    #[test]
    fn rejects_paths_that_do_not_exist() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("missing.icns")), dir.path()),
            None
        );
    }

    #[test]
    fn rejects_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("icon.icns");
        std::fs::create_dir(&nested).unwrap();

        assert_eq!(
            resolve_icon_path(Some(Path::new("icon.icns")), dir.path()),
            None
        );
        assert!(PathBuf::from(&nested).is_dir());
    }
}
