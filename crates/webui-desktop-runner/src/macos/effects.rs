// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Window background painting and platform visual-effect support.

use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowTabbingMode,
};
use objc2_foundation::NSRect;
use objc2_web_kit::WKWebView;
use webui_desktop::{Rgba, WindowEffect};

/// The macOS treatment chosen for a requested [`WindowEffect`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResolvedEffect {
    /// No visual effect; the webview is the window's content view directly.
    None,
    /// `NSVisualEffectView` vibrancy behind a transparent webview.
    Vibrancy,
    /// Native window tabbing, with no content-view change.
    Tabbed,
}

/// Map a cross-platform [`WindowEffect`] to the nearest macOS treatment.
///
/// `Acrylic` and `Mica` are Windows-only materials with no macOS equivalent,
/// so both degrade to `Vibrancy`, the closest native translucency effect.
#[must_use]
pub(super) fn resolve_effect(effect: WindowEffect) -> ResolvedEffect {
    match effect {
        WindowEffect::None => ResolvedEffect::None,
        WindowEffect::Vibrancy | WindowEffect::Acrylic | WindowEffect::Mica => {
            ResolvedEffect::Vibrancy
        }
        WindowEffect::Tabbed => ResolvedEffect::Tabbed,
    }
}

/// Convert a manifest RGBA color into an `NSColor`.
#[must_use]
pub(super) fn native_color(color: Rgba) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
        f64::from(color.a) / 255.0,
    )
}

/// Paint the window's pre-paint background and the webview's under-page color
/// so no white flash is visible before the first frame of web content paints.
pub(super) fn apply_background(window: &NSWindow, webview: &WKWebView, background: Option<Rgba>) {
    let Some(color) = background else { return };
    let native = native_color(color);
    window.setBackgroundColor(Some(&native));
    // SAFETY: `setUnderPageBackgroundColor:` is a plain WebKit setter on a
    // live, main-thread `WKWebView`; it only changes the paint color shown
    // before content loads and during elastic overscroll.
    unsafe { webview.setUnderPageBackgroundColor(Some(&native)) };
}

/// Install the window's content view, wrapping the webview in an
/// `NSVisualEffectView` when a vibrancy-family effect was requested.
///
/// Returns the resolved effect so callers can log degradation decisions.
pub(super) fn install_content_view(
    mtm: objc2::MainThreadMarker,
    window: &NSWindow,
    webview: &WKWebView,
    effect: WindowEffect,
) -> ResolvedEffect {
    let resolved = resolve_effect(effect);
    match resolved {
        ResolvedEffect::None => window.setContentView(Some(webview)),
        ResolvedEffect::Vibrancy => {
            let content_rect = window.frame();
            let effect_view = build_vibrancy_view(mtm, content_rect);
            webview.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            webview.setFrame(effect_view.bounds());
            effect_view.addSubview(webview);
            window.setContentView(Some(&effect_view));
        }
        ResolvedEffect::Tabbed => {
            window.setTabbingMode(NSWindowTabbingMode::Preferred);
            window.setContentView(Some(webview));
        }
    }
    resolved
}

fn build_vibrancy_view(
    mtm: objc2::MainThreadMarker,
    frame: NSRect,
) -> Retained<NSVisualEffectView> {
    let rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), frame.size);
    let view = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    view.setState(NSVisualEffectState::FollowsWindowActiveState);
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    view
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn vibrancy_family_effects_degrade_to_vibrancy() {
        assert_eq!(
            resolve_effect(WindowEffect::Vibrancy),
            ResolvedEffect::Vibrancy
        );
        assert_eq!(
            resolve_effect(WindowEffect::Acrylic),
            ResolvedEffect::Vibrancy
        );
        assert_eq!(resolve_effect(WindowEffect::Mica), ResolvedEffect::Vibrancy);
    }

    #[test]
    fn tabbed_and_none_are_not_degraded() {
        assert_eq!(resolve_effect(WindowEffect::None), ResolvedEffect::None);
        assert_eq!(resolve_effect(WindowEffect::Tabbed), ResolvedEffect::Tabbed);
    }
}
