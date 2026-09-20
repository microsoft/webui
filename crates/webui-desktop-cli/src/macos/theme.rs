// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native dark/light appearance detection and change notifications.

use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAppearance, NSAppearanceNameDarkAqua, NSApplication};
use objc2_foundation::{
    NSArray, NSDistributedNotificationCenter, NSNotification, NSObject, NSObjectProtocol, NSString,
};
use objc2_web_kit::WKWebView;
use webui_desktop::{DesktopEvent, EventRegistry};

use super::dispatch_event;

/// Return whether the effective application appearance is a dark variant.
#[must_use]
pub(super) fn is_dark_appearance(mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    appearance_is_dark(&app.effectiveAppearance())
}

fn appearance_is_dark(appearance: &NSAppearance) -> bool {
    // SAFETY: `NSAppearanceNameDarkAqua` is a statically initialized AppKit
    // constant that is valid for the entire process lifetime.
    let dark_aqua = unsafe { NSAppearanceNameDarkAqua };
    appearance
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[dark_aqua]))
        .is_some()
}

pub(super) struct ThemeObserverIvars {
    events: EventRegistry,
    webview: Retained<WKWebView>,
}

define_class!(
    // SAFETY: Observer is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ThemeObserverIvars]
    pub(super) struct DesktopThemeObserver;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopThemeObserver {}

    impl DesktopThemeObserver {
        #[unsafe(method(desktopThemeChanged:))]
        fn desktop_theme_changed(&self, _notification: &NSNotification) {
            let dark = is_dark_appearance(self.mtm());
            let ivars = self.ivars();
            dispatch_event(&ivars.events, &ivars.webview, DesktopEvent::ThemeChanged { dark });
        }
    }
);

impl DesktopThemeObserver {
    fn new(
        mtm: MainThreadMarker,
        events: EventRegistry,
        webview: Retained<WKWebView>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ThemeObserverIvars { events, webview });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// Install a distributed-notification observer for AppKit interface-theme
/// changes and dispatch an initial `ThemeChanged` event so JS state starts in
/// sync with the native appearance.
pub(super) fn install_theme_observer(
    mtm: MainThreadMarker,
    events: EventRegistry,
    webview: Retained<WKWebView>,
) -> Retained<DesktopThemeObserver> {
    let observer = DesktopThemeObserver::new(mtm, events.clone(), webview.clone());
    let center = NSDistributedNotificationCenter::defaultCenter();
    let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
    // SAFETY: `observer` is retained for the app lifetime by its OnceCell
    // owner, and the registered selector matches the method defined above.
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(desktopThemeChanged:),
            Some(&name),
            None,
        );
    }
    dispatch_event(
        &events,
        &webview,
        DesktopEvent::ThemeChanged {
            dark: is_dark_appearance(mtm),
        },
    );
    observer
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn dark_aqua_is_recognized_as_dark() {
        // SAFETY: See `appearance_is_dark`.
        let dark_aqua = unsafe { NSAppearanceNameDarkAqua };
        let appearance = NSAppearance::appearanceNamed(dark_aqua).unwrap();
        assert!(appearance_is_dark(&appearance));
    }
}
