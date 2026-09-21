// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{TitlebarStyle, WindowOptions};
use objc2_app_kit::NSWindowStyleMask;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NativeWindowStyle {
    pub(super) mask: NSWindowStyleMask,
    pub(super) transparent_titlebar: bool,
    pub(super) hidden_title: bool,
    pub(super) frameless: bool,
}

pub(super) fn native_window_style(options: &WindowOptions) -> NativeWindowStyle {
    let mut mask =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
    if options.resizable {
        mask |= NSWindowStyleMask::Resizable;
    }
    match options.titlebar {
        TitlebarStyle::Native => NativeWindowStyle {
            mask,
            transparent_titlebar: false,
            hidden_title: false,
            frameless: false,
        },
        TitlebarStyle::HiddenInset | TitlebarStyle::Overlay { .. } => NativeWindowStyle {
            mask: mask | NSWindowStyleMask::FullSizeContentView,
            transparent_titlebar: true,
            hidden_title: true,
            frameless: false,
        },
        TitlebarStyle::None => NativeWindowStyle {
            mask: NSWindowStyleMask::Borderless,
            transparent_titlebar: false,
            hidden_title: true,
            frameless: true,
        },
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn hidden_inset_uses_full_size_content_view() {
        let style = native_window_style(&WindowOptions {
            titlebar: TitlebarStyle::HiddenInset,
            ..WindowOptions::default()
        });
        assert_ne!(style.mask.0 & NSWindowStyleMask::FullSizeContentView.0, 0);
        assert!(style.transparent_titlebar);
        assert!(style.hidden_title);
    }

    #[test]
    fn frameless_window_does_not_retain_standard_chrome() {
        let style = native_window_style(&WindowOptions {
            titlebar: TitlebarStyle::None,
            ..WindowOptions::default()
        });
        assert_eq!(style.mask, NSWindowStyleMask::Borderless);
        assert!(style.frameless);
    }

    #[test]
    fn non_resizable_native_window_omits_resizable_mask() {
        let style = native_window_style(&WindowOptions {
            resizable: false,
            ..WindowOptions::default()
        });
        assert_eq!(style.mask.0 & NSWindowStyleMask::Resizable.0, 0);
    }
}
