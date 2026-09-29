// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Navigation policy delegate: only the app's own custom-scheme origin (and
//! `about:` URLs used by WebKit internally) may load; everything else is
//! denied by default and reported through `DesktopEvent::NavigationRequested`.

use crate::window::LiveBackground;
use crate::{DesktopEvent, EventRegistry, EventResponse, WindowId};
use block2::DynBlock;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{ns_string, NSObject, NSObjectProtocol, NSURL};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView,
};
#[cfg(feature = "local-server")]
use objc2_web_kit::{WKNavigationResponse, WKNavigationResponsePolicy};
use std::sync::Arc;

use super::dispatch_event;

pub(super) struct NavigationDelegateIvars {
    pub(super) events: EventRegistry,
    live_background: Arc<LiveBackground>,
    #[cfg(feature = "local-server")]
    local_origin: Option<crate::LoopbackOrigin>,
    #[cfg(feature = "local-server")]
    lifetime: Option<crate::HostLifetime>,
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
        #[cfg(feature = "local-server")]
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        unsafe fn webView_decidePolicyForNavigationResponse_decisionHandler(
            &self,
            _web_view: &WKWebView,
            response: &WKNavigationResponse,
            decision_handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let allowed = self.ivars().local_origin.as_ref().is_none_or(|origin| {
                // SAFETY: WebKit provides the response for the duration of
                // this synchronous policy callback on the UI thread.
                unsafe {
                    response.isForMainFrame()
                        && response
                            .response()
                            .URL()
                            .and_then(|url| url.absoluteString())
                            .is_some_and(|url| {
                                self.ivars().lifetime.as_ref().is_some_and(|lifetime| {
                                    lifetime.allows_navigation(origin, &url.to_string())
                                })
                            })
                }
            });
            decision_handler.call((if allowed {
                WKNavigationResponsePolicy::Allow
            } else {
                WKNavigationResponsePolicy::Cancel
            },));
        }

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

        #[cfg(any(feature = "local-server", feature = "application-ipc"))]
        #[unsafe(method(webView:didCommitNavigation:))]
        unsafe fn committed(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            #[cfg(feature = "local-server")]
            if self
                .ivars()
                .lifetime
                .as_ref()
                .is_some_and(|lifetime| !lifetime.is_active())
            {
                // SAFETY: WebKit invokes this callback on its main-thread view;
                // stop before a retired owner's document can proceed.
                unsafe { web_view.stopLoading() };
                if let Some(window) = web_view.window() {
                    window.close();
                }
                return;
            }
            #[cfg(not(feature = "application-ipc"))]
            let _ = navigation;
            #[cfg(feature = "application-ipc")]
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
                    let permitted = self.allowed_navigation(url, navigation_action)
                        && dispatch_event(&self.ivars().events, web_view, event)
                            == EventResponse::Continue;
                    #[cfg(feature = "local-server")]
                    let permitted =
                        permitted && navigation_lifetime_active(self.ivars().lifetime.as_ref());
                    if permitted {
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
            #[cfg(feature = "local-server")]
            if self
                .ivars()
                .lifetime
                .as_ref()
                .is_some_and(|lifetime| !lifetime.is_active())
            {
                // SAFETY: The live view is still on WebKit's main thread.
                unsafe { web_view.stopLoading() };
                if let Some(window) = web_view.window() {
                    window.close();
                }
                return;
            }
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
            if let Some(color) = self.ivars().live_background.current() {
                super::commands::update_document_background(web_view, color);
            }
        }
    }
);

#[cfg(feature = "local-server")]
fn navigation_lifetime_active(lifetime: Option<&crate::HostLifetime>) -> bool {
    lifetime.is_none_or(crate::HostLifetime::is_active)
}

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
    fn allowed_navigation(&self, url: &NSURL, action: &WKNavigationAction) -> bool {
        #[cfg(feature = "local-server")]
        if let Some(origin) = &self.ivars().local_origin {
            // SAFETY: WebKit supplies this live navigation action during the
            // synchronous delegate callback; both identities are native-owned.
            return unsafe {
                action
                    .targetFrame()
                    .is_some_and(|frame| frame.isMainFrame())
            } && url.absoluteString().is_some_and(|value| {
                self.ivars()
                    .lifetime
                    .as_ref()
                    .is_some_and(|lifetime| lifetime.allows_navigation(origin, &value.to_string()))
            });
        }
        let _ = action;
        is_allowed_navigation_url(url)
    }

    pub(super) fn new(
        mtm: MainThreadMarker,
        events: EventRegistry,
        live_background: Arc<LiveBackground>,
        #[cfg(feature = "local-server")] local: Option<(
            crate::LoopbackOrigin,
            crate::HostLifetime,
        )>,
        #[cfg(feature = "application-ipc")] ipc: Option<std::rc::Rc<super::ipc::MacIpc>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationDelegateIvars {
            events,
            live_background,
            #[cfg(feature = "local-server")]
            local_origin: local.as_ref().map(|(origin, _)| origin.clone()),
            #[cfg(feature = "local-server")]
            lifetime: local.map(|(_, lifetime)| lifetime),
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

    #[cfg(feature = "local-server")]
    #[test]
    fn synchronous_navigation_handler_retirement_cancels_its_own_request() {
        let (owner, lifetime) = crate::HostLifetime::new();
        let events = EventRegistry::default();
        events
            .on_event(move |_| {
                assert!(owner.revoke().is_ok());
                EventResponse::Continue
            })
            .unwrap();
        assert!(navigation_lifetime_active(Some(&lifetime)));
        let response = events.dispatch(&DesktopEvent::NavigationRequested {
            window_id: WindowId::PRIMARY,
            url: "http://127.0.0.1:3456/deep".into(),
        });
        assert_eq!(response, EventResponse::Continue);
        assert!(!navigation_lifetime_active(Some(&lifetime)));
        assert!(navigation_lifetime_active(None));
    }

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
