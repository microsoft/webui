// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Navigation policy delegate: only the app's own custom-scheme origin (and
//! `about:` URLs used by WebKit internally) may load; everything else is
//! denied by default and reported through `DesktopEvent::NavigationRequested`.

use crate::{DesktopEvent, EventRegistry, EventResponse, WindowId};
use block2::DynBlock;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol, NSURL};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView,
};

use super::dispatch_event;

pub(super) struct NavigationDelegateIvars {
    pub(super) events: EventRegistry,
}

define_class!(
    // SAFETY: Navigation delegate is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavigationDelegateIvars]
    pub(super) struct DesktopNavigationDelegate;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopNavigationDelegate {}

    // SAFETY: Method signatures match WKNavigationDelegate.
    #[allow(non_snake_case)]
    unsafe impl WKNavigationDelegate for DesktopNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn webView_decidePolicyForNavigationAction_decisionHandler(
            &self,
            web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let request = navigation_action.request();
            let policy = request
                .URL()
                .as_deref()
                .map_or(WKNavigationActionPolicy::Cancel, |url| {
                    let event = DesktopEvent::NavigationRequested {
                        window_id: WindowId::PRIMARY,
                        url: url
                            .absoluteString()
                            .map_or_else(String::new, |value| value.to_string()),
                    };
                    if is_allowed_navigation_url(url)
                        && dispatch_event(&self.ivars().events, web_view, event)
                            == EventResponse::Continue
                    {
                        WKNavigationActionPolicy::Allow
                    } else {
                        WKNavigationActionPolicy::Cancel
                    }
                });
            decision_handler.call((policy,));
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        unsafe fn webView_didFinishNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            let url = web_view
                .URL()
                .and_then(|url| url.absoluteString())
                .map_or_else(String::new, |value| value.to_string());
            dispatch_event(
                &self.ivars().events,
                web_view,
                DesktopEvent::NavigationCompleted {
                    window_id: WindowId::PRIMARY,
                    url,
                },
            );
        }
    }
);

impl DesktopNavigationDelegate {
    pub(super) fn new(mtm: MainThreadMarker, events: EventRegistry) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationDelegateIvars { events });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// Return whether a navigation target is on the app's own allowlisted origin.
///
/// Delegates to the shared cross-backend policy so macOS, Windows, and Linux
/// cannot drift apart. This previously accepted the entire `about:` scheme,
/// which was broader than the other backends.
fn is_allowed_navigation_url(url: &NSURL) -> bool {
    url.absoluteString().is_some_and(|value| {
        crate::is_allowed_navigation_url(&value.to_string(), crate::macos::APP_ORIGIN)
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn rejects_about_scheme_urls_other_than_blank() {
        let srcdoc =
            NSURL::URLWithString(&objc2_foundation::NSString::from_str("about:srcdoc")).unwrap();
        assert!(!is_allowed_navigation_url(&srcdoc));
    }

    #[test]
    fn allows_only_the_app_origin_and_about_urls() {
        let app =
            NSURL::URLWithString(&objc2_foundation::NSString::from_str("webui://app/foo")).unwrap();
        assert!(is_allowed_navigation_url(&app));

        let other = NSURL::URLWithString(&objc2_foundation::NSString::from_str("webui://evil/foo"))
            .unwrap();
        assert!(!is_allowed_navigation_url(&other));

        let https =
            NSURL::URLWithString(&objc2_foundation::NSString::from_str("https://example.com"))
                .unwrap();
        assert!(!is_allowed_navigation_url(&https));

        let about =
            NSURL::URLWithString(&objc2_foundation::NSString::from_str("about:blank")).unwrap();
        assert!(is_allowed_navigation_url(&about));
    }
}
