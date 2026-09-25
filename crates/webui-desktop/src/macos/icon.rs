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

use std::path::Path;

use objc2::AnyThread;
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSString;

/// Apply `icon_path` as the running application's Dock icon, if it resolves to
/// a readable image.
///
/// A missing, unreadable, or undecodable icon leaves the Dock icon untouched:
/// artwork is cosmetic and must never prevent the window from opening.
pub(super) fn install_app_icon(
    app: &NSApplication,
    icon_path: Option<&Path>,
    bundle_root: Option<&Path>,
) {
    let Some(path) = crate::icon_path::resolve_icon_path(icon_path, bundle_root) else {
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
