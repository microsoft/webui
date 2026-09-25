// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Navigation policy delegate: only the app's own custom-scheme origin (and
//! `about:` URLs used by WebKit internally) may load; everything else is
//! denied by default and reported through `DesktopEvent::NavigationRequested`.

use crate::{DesktopEvent, EventRegistry, EventResponse, WindowId};
use block2::DynBlock;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{ns_string, NSObject, NSObjectProtocol, NSURL};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView,
};

use super::dispatch_event;

pub(super) struct NavigationDelegateIvars {
    pub(super) events: EventRegistry,
    #[cfg(feature = "application-ipc")]
    ipc: Option<std::rc::Rc<super::ipc::MacIpc>>,
    #[cfg(feature = "application-ipc")]
    // Outer None: no pending navigation. Inner None: WebKit supplied a nil
    // identity (as it does for cross-document Navigation API navigations).
    ipc_navigation: std::cell::RefCell<Option<Option<Retained<WKNavigation>>>>,
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
        #[cfg(feature = "application-ipc")]
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        unsafe fn started(&self, _web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            if let Some(ipc) = &self.ivars().ipc {
                ipc.navigation_started();
                let previous = self
                    .ivars()
                    .ipc_navigation
                    .replace(Some(navigation.map(objc2::Message::retain)));
                drop(previous);
            }
        }

        #[cfg(feature = "application-ipc")]
        #[unsafe(method(webView:didCommitNavigation:))]
        unsafe fn committed(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            if let Some(ipc) = &self.ivars().ipc {
                let matches = committed_navigation_matches(
                    self.ivars()
                        .ipc_navigation
                        .borrow()
                        .as_ref()
                        .map(|value| value.as_deref()),
                    navigation,
                );
                if matches {
                    self.ivars().ipc_navigation.borrow_mut().take();
                    ipc.committed(web_view);
                }
            }
        }

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
            #[cfg(feature = "application-ipc")]
            if policy == WKNavigationActionPolicy::Allow
                && navigation_action.navigationType()
                    == objc2_web_kit::WKNavigationType::BackForward
                && navigation_action
                    .targetFrame()
                    .is_some_and(|frame| frame.isMainFrame())
                && self.ivars().ipc.as_ref().is_some_and(|ipc| {
                    ipc.retire_before_history_navigation(web_view, decision_handler)
                })
            {
                return;
            }
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

#[cfg(feature = "application-ipc")]
fn committed_navigation_matches<T>(started: Option<Option<&T>>, committed: Option<&T>) -> bool {
    match (started, committed) {
        (Some(Some(started)), Some(committed)) => std::ptr::eq(started, committed),
        // Both notifications are native main-document callbacks. A nil commit
        // is valid only after an observed nil start, never without a start or
        // after a pointer-identified start. Epoch checks still guard every
        // asynchronous nonce probe and activation against later navigations.
        (Some(None), None) => true,
        _ => false,
    }
}

impl DesktopNavigationDelegate {
    pub(super) fn new(
        mtm: MainThreadMarker,
        events: EventRegistry,
        #[cfg(feature = "application-ipc")] ipc: Option<std::rc::Rc<super::ipc::MacIpc>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationDelegateIvars {
            events,
            #[cfg(feature = "application-ipc")]
            ipc,
            #[cfg(feature = "application-ipc")]
            ipc_navigation: std::cell::RefCell::new(None),
        });
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

pub(super) fn trusted_app_url(url: &NSURL) -> bool {
    url.scheme()
        .is_some_and(|scheme| scheme.isEqualToString(ns_string!("webui")))
        && url
            .host()
            .is_some_and(|host| host.isEqualToString(ns_string!("app")))
        && url.port().is_none()
        && url.user().is_none()
        && url.password().is_none()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[cfg(feature = "application-ipc")]
    #[test]
    fn nil_navigation_api_identity_requires_an_observed_matching_start() {
        let first = 1;
        let second = 2;
        assert!(committed_navigation_matches::<u8>(Some(None), None));
        assert!(!committed_navigation_matches::<u8>(None, None));
        assert!(!committed_navigation_matches(Some(Some(&first)), None));
        assert!(!committed_navigation_matches(Some(None), Some(&first)));
        assert!(!committed_navigation_matches(
            Some(Some(&first)),
            Some(&second)
        ));
        assert!(committed_navigation_matches(
            Some(Some(&first)),
            Some(&first)
        ));
    }

    #[test]
    fn rejects_about_scheme_urls_other_than_blank() {
        let srcdoc =
            NSURL::URLWithString(&objc2_foundation::NSString::from_str("about:srcdoc")).unwrap();
        assert!(!is_allowed_navigation_url(&srcdoc));
    }

    #[test]
    fn resource_and_ipc_origin_exclude_blank_credentials_and_lookalikes() {
        for (value, trusted) in [
            ("webui://app/asset", true),
            ("webui://app.evil/asset", false),
            ("webui://user@app/asset", false),
            ("webui://app:80/asset", false),
            ("about:blank", false),
        ] {
            let url = NSURL::URLWithString(&objc2_foundation::NSString::from_str(value)).unwrap();
            assert_eq!(trusted_app_url(&url), trusted);
        }
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
