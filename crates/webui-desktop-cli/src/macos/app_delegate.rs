// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The `NSApplicationDelegate`/`NSWindowDelegate` that owns the window,
//! webview, and every native peripheral (menu, tray, theme observer, and
//! persisted geometry) for the lifetime of the app.
//!
//! Window/webview construction lives in [`super::launch`] to keep this
//! file's cognitive complexity low; this file only holds the ivars and the
//! `NSObject`/`NSApplicationDelegate`/`NSWindowDelegate` trait implementation
//! that AppKit calls into directly.

use std::cell::OnceCell;

use objc2::rc::{autoreleasepool, Retained};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSStatusItem, NSWindow,
    NSWindowDelegate,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};
use objc2_web_kit::WKWebView;
use webui_desktop::{
    DesktopEvent, DesktopShellConfig, EventRegistry, EventResponse, WindowId, WindowOptions,
};

use super::geometry::{clamp_coordinate, clamp_dimension};
use super::host_message::DesktopHostMessageHandler;
use super::launch::{build_window_and_webview, persist_window_state_if_enabled};
use super::navigation::DesktopNavigationDelegate;
use super::scheme::DesktopSchemeHandler;
use super::theme::DesktopThemeObserver;
use super::window::DesktopWindow;
use super::{dispatch_event, MacosLaunchOptions};

pub(super) struct AppDelegateIvars {
    pub(in crate::macos) window: OnceCell<Retained<DesktopWindow>>,
    pub(in crate::macos) webview: OnceCell<Retained<WKWebView>>,
    pub(in crate::macos) scheme_handler: OnceCell<Retained<DesktopSchemeHandler>>,
    pub(in crate::macos) navigation_delegate: OnceCell<Retained<DesktopNavigationDelegate>>,
    pub(in crate::macos) host_message_handler: OnceCell<Retained<DesktopHostMessageHandler>>,
    pub(in crate::macos) theme_observer: OnceCell<Retained<DesktopThemeObserver>>,
    pub(in crate::macos) tray_item: OnceCell<Retained<NSStatusItem>>,
    pub(in crate::macos) title: Retained<NSString>,
    pub(in crate::macos) options: WindowOptions,
    pub(in crate::macos) shell: DesktopShellConfig,
    pub(in crate::macos) events: EventRegistry,
    pub(in crate::macos) window_handle: webui_desktop::WindowHandle,
}

define_class!(
    // SAFETY: Delegate is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    pub(super) struct DesktopAppDelegate;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopAppDelegate {}

    // SAFETY: Method signatures match NSApplicationDelegate.
    unsafe impl NSApplicationDelegate for DesktopAppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            autoreleasepool(|_| {
                let Some(app_obj) = notification.object() else {
                    return;
                };
                let Ok(app) = app_obj.downcast::<NSApplication>() else {
                    return;
                };
                build_window_and_webview(self, &app);
                app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            });
        }
    }

    // SAFETY: Method signatures match NSWindowDelegate.
    #[allow(non_snake_case)]
    unsafe impl NSWindowDelegate for DesktopAppDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn windowShouldClose(&self, _window: &NSWindow) -> bool {
            !matches!(
                dispatch_for_delegate(
                    self,
                    DesktopEvent::WindowCloseRequested {
                        window_id: WindowId(0)
                    }
                ),
                EventResponse::PreventDefault
            )
        }

        #[unsafe(method(windowDidResize:))]
        fn windowDidResize(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                let size = window.frame().size;
                dispatch_for_delegate(
                    self,
                    DesktopEvent::WindowResized {
                        window_id: WindowId(0),
                        width: clamp_dimension(size.width),
                        height: clamp_dimension(size.height),
                    },
                );
            }
        }

        #[unsafe(method(windowDidMove:))]
        fn windowDidMove(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                let origin = window.frame().origin;
                dispatch_for_delegate(
                    self,
                    DesktopEvent::WindowMoved {
                        window_id: WindowId(0),
                        x: clamp_coordinate(origin.x),
                        y: clamp_coordinate(origin.y),
                    },
                );
            }
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn windowDidChangeBackingProperties(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                dispatch_for_delegate(
                    self,
                    DesktopEvent::ScaleFactorChanged {
                        scale: window.backingScaleFactor(),
                    },
                );
            }
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn windowDidMiniaturize(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowMinimized {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowDidDeminiaturize:))]
        fn windowDidDeminiaturize(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowRestored {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowDidBecomeKey:))]
        fn windowDidBecomeKey(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowFocused {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowDidResignKey:))]
        fn windowDidResignKey(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowBlurred {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowDidEnterFullScreen:))]
        fn windowDidEnterFullScreen(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowEnteredFullscreen {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowDidExitFullScreen:))]
        fn windowDidExitFullScreen(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowLeftFullscreen {
                    window_id: WindowId(0),
                },
            );
        }
        #[unsafe(method(windowWillClose:))]
        fn windowWillClose(&self, _notification: &NSNotification) {
            persist_window_state_if_enabled(self);
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowClosed {
                    window_id: WindowId(0),
                },
            );
            dispatch_for_delegate(self, DesktopEvent::Exiting);
            // SAFETY: Called on the main thread by AppKit while the shared app exists.
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

pub(super) fn dispatch_for_delegate(
    delegate: &DesktopAppDelegate,
    event: DesktopEvent,
) -> EventResponse {
    let ivars = delegate.ivars();
    let Some(webview) = ivars.webview.get() else {
        return EventResponse::Continue;
    };
    dispatch_event(&ivars.events, webview, event)
}

impl DesktopAppDelegate {
    pub(super) fn new(mtm: MainThreadMarker, options: MacosLaunchOptions) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            window: OnceCell::new(),
            webview: OnceCell::new(),
            scheme_handler: OnceCell::new(),
            navigation_delegate: OnceCell::new(),
            host_message_handler: OnceCell::new(),
            theme_observer: OnceCell::new(),
            tray_item: OnceCell::new(),
            title: options.title,
            options: options.options,
            shell: options.shell,
            events: options.events,
            window_handle: options.window_handle,
        });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}
