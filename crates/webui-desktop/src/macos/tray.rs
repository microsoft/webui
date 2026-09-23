// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native menu-bar tray icon (`NSStatusItem`) support.

use std::path::Path;

use crate::TrayConfig;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSImage, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::NSString;

/// Install the menu-bar tray icon described by `config`, if any.
///
/// Returns the retained status item so the caller keeps it alive for the app
/// lifetime; dropping it removes the icon from the menu bar.
#[must_use]
pub(super) fn install_tray(
    mtm: MainThreadMarker,
    config: &TrayConfig,
) -> Option<Retained<NSStatusItem>> {
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        match load_tray_image(&config.icon_path) {
            Some(image) => button.setImage(Some(&image)),
            None => button.setTitle(&NSString::from_str("•")),
        }
        if let Some(tooltip) = &config.tooltip {
            button.setToolTip(Some(&NSString::from_str(tooltip)));
        }
    }
    Some(item)
}

fn load_tray_image(path: &Path) -> Option<Retained<NSImage>> {
    let path_string = NSString::from_str(&path.to_string_lossy());
    NSImage::initWithContentsOfFile(NSImage::alloc(), &path_string)
}
