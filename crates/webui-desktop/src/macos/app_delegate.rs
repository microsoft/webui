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

#[cfg(feature = "local-server")]
use std::cell::RefCell;
use std::cell::{Cell, OnceCell};

use crate::{
    DesktopEvent, DesktopShellConfig, EventRegistry, EventResponse, WindowId, WindowOptions,
    WindowStateStore,
};
use objc2::rc::{autoreleasepool, Retained};
#[cfg(feature = "local-server")]
use objc2::sel;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSApplicationTerminateReply;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSStatusItem, NSWindow,
    NSWindowDelegate,
};
#[cfg(feature = "native-url-activation")]
use objc2_foundation::{NSArray, NSURL};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};
#[cfg(feature = "local-server")]
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_web_kit::WKWebView;

use super::geometry::{clamp_coordinate, clamp_dimension};
use super::host_message::DesktopHostMessageHandler;
use super::launch::{build_window_and_webview, persist_window_state_if_enabled};
use super::menu::NativeMenu;
use super::navigation::DesktopNavigationDelegate;
use super::scheme::DesktopSchemeHandler;
use super::theme::DesktopThemeObserver;
use super::window::{align_overlay_controls, DesktopWindow};
use super::{dispatch_event, MacosLaunchOptions};

pub(super) struct AppDelegateIvars {
    pub(in crate::macos) executor: std::sync::Arc<crate::execution::ApplicationExecutor>,
    pub(in crate::macos) runtime: Option<std::sync::Arc<crate::DesktopRuntime>>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) local_origin: Option<crate::LoopbackOrigin>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) local_url: Option<String>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) lifetime: Option<crate::HostLifetime>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) frame_policy: Option<std::sync::Arc<crate::frame_policy::FramePolicy>>,
    #[cfg(feature = "native-url-activation")]
    pub(in crate::macos) url_activation:
        Option<std::sync::Arc<crate::local_server::url_activation::ActivationSender>>,
    #[cfg(feature = "native-url-activation")]
    pub(in crate::macos) pending_url_activations: RefCell<Vec<crate::UrlActivation>>,
    #[cfg(feature = "native-url-activation")]
    pub(in crate::macos) url_activation_ready: Cell<bool>,
    #[cfg(feature = "native-services")]
    pub(in crate::macos) native_services: Option<crate::NativeServices>,
    #[cfg(feature = "native-services")]
    pub(in crate::macos) geometry_registration:
        OnceCell<crate::native_services::GeometryRegistration>,
    #[cfg(feature = "native-services")]
    pub(in crate::macos) theme_registration: OnceCell<crate::native_theme::platform::Registration>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) owner_close_wake: OnceCell<super::commands::CommandWake>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) owner_close_registration:
        std::cell::RefCell<Option<crate::local_server::HostCloseRegistration>>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) startup_error: std::cell::RefCell<Option<crate::DesktopError>>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) quit_close_pending: Cell<bool>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) quit_close_deadline: RefCell<Option<Retained<NSTimer>>>,
    pub(in crate::macos) live_background: std::sync::Arc<crate::window::LiveBackground>,
    pub(in crate::macos) command_wake: OnceCell<super::commands::CommandWake>,
    #[cfg(feature = "application-ipc")]
    pub(in crate::macos) ipc: Option<std::rc::Rc<super::ipc::MacIpc>>,
    pub(in crate::macos) window: OnceCell<Retained<DesktopWindow>>,
    pub(in crate::macos) webview: OnceCell<Retained<WKWebView>>,
    pub(in crate::macos) scheme_handler: OnceCell<Retained<DesktopSchemeHandler>>,
    pub(in crate::macos) navigation_delegate: OnceCell<Retained<DesktopNavigationDelegate>>,
    pub(in crate::macos) host_message_handler: OnceCell<Retained<DesktopHostMessageHandler>>,
    pub(in crate::macos) theme_observer: OnceCell<Retained<DesktopThemeObserver>>,
    pub(in crate::macos) tray_item: OnceCell<Retained<NSStatusItem>>,
    pub(in crate::macos) main_menu: OnceCell<NativeMenu>,
    pub(in crate::macos) title: Retained<NSString>,
    pub(in crate::macos) options: WindowOptions,
    pub(in crate::macos) shell: DesktopShellConfig,
    pub(in crate::macos) events: EventRegistry,
    pub(in crate::macos) window_handle: crate::WindowHandle,
    pub(in crate::macos) state_store: Option<WindowStateStore>,
    #[cfg(feature = "local-server")]
    pub(in crate::macos) persistent_website_data: bool,
    /// Last observed `NSWindow::isZoomed` value.
    ///
    /// AppKit has no `windowDidZoom:` notification, so maximize transitions are
    /// derived by diffing this against `isZoomed` on each resize. Without it
    /// macOS would be the only backend that never emits
    /// `WindowMaximized`/`WindowUnmaximized`.
    pub(in crate::macos) zoomed: Cell<bool>,
    pub(in crate::macos) exiting: Cell<bool>,
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
    #[allow(non_snake_case)]
    unsafe impl NSApplicationDelegate for DesktopAppDelegate {
        #[cfg(feature = "native-url-activation")]
        #[unsafe(method(application:openURLs:))]
        fn application_openURLs(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            self.receive_open_urls(urls);
        }

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

        #[unsafe(method(applicationShouldTerminate:))]
        fn applicationShouldTerminate(&self, app: &NSApplication) -> NSApplicationTerminateReply {
            // A bundled app keeps AppKit's existing termination behavior. A
            // local HTTP host must instead regain control after WindowClosed
            // to retire its IPC pin and drain its own server off this thread.
            #[cfg(feature = "local-server")]
            if self.ivars().local_origin.is_some() {
                if self.ivars().exiting.get() {
                    super::stop_local_app(app);
                    return NSApplicationTerminateReply::TerminateCancel;
                }
                if self.ivars().quit_close_pending.get() {
                    return NSApplicationTerminateReply::TerminateCancel;
                }
                if self.ivars().window.get().is_none() {
                    self.ivars().startup_error.replace(Some(local_quit_error(
                        "AppKit Quit arrived before the local-server window was ready",
                    )));
                    super::stop_local_app(app);
                    return NSApplicationTerminateReply::TerminateCancel;
                }
                self.ivars().quit_close_pending.set(true);
                // The command runs on the next main-queue turn, outside
                // applicationShouldTerminate's AppKit dispatch stack. Queueing
                // is not proof that windowShouldClose accepted the request.
                if let Err(error) = self.ivars().window_handle.request_close() {
                    self.ivars().startup_error.replace(Some(local_quit_error(&format!(
                        "AppKit Quit could not queue the window close: {error}"
                    ))));
                    // A failed queue admission must not unwind the frame and
                    // release its listener pin while the native window lives.
                    // Try AppKit's ordinary cancellable close on this thread;
                    // if vetoed, wait for a later successful close to return
                    // the recorded error to the host.
                    if let Some(window) = self.ivars().window.get() {
                        window.performClose(None);
                    }
                    if !self.ivars().exiting.get() {
                        self.ivars().quit_close_pending.set(false);
                    }
                } else {
                    let timer = unsafe {
                        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                            15.0,
                            self,
                            sel!(localQuitCloseTimedOut:),
                            None,
                            false,
                        )
                    };
                    // Keep the deadline active during AppKit's event-tracking
                    // modes, not only the default run-loop mode.
                    // SAFETY: This timer and run loop both belong to the
                    // proven AppKit main thread.
                    unsafe {
                        NSRunLoop::currentRunLoop()
                            .addTimer_forMode(&timer, NSRunLoopCommonModes);
                    }
                    self.ivars().quit_close_deadline.replace(Some(timer));
                }
                return NSApplicationTerminateReply::TerminateCancel;
            }
            #[cfg(not(feature = "local-server"))]
            let _ = app;
            NSApplicationTerminateReply::TerminateNow
        }
    }

    // SAFETY: Method signatures match NSWindowDelegate.
    #[allow(non_snake_case)]
    unsafe impl NSWindowDelegate for DesktopAppDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn windowShouldClose(&self, _window: &NSWindow) -> bool {
            let allowed = !matches!(
                dispatch_for_delegate(
                    self,
                    DesktopEvent::WindowCloseRequested {
                        window_id: WindowId::PRIMARY
                    }
                ),
                EventResponse::PreventDefault
            );
            #[cfg(feature = "local-server")]
            if !allowed {
                // A veto ends this attempt, not the session. A later Cmd+Q
                // must be able to make a fresh request.
                self.cancel_quit_deadline();
            }
            allowed
        }

        #[unsafe(method(windowDidResize:))]
        fn windowDidResize(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                align_overlay_controls(window, &self.ivars().options);
                // Emit the state transition before the geometry so handlers see
                // the window become maximized before its new size, matching the
                // ordering the Windows and Linux backends use.
                let zoomed = window.isZoomed();
                if self.ivars().zoomed.replace(zoomed) != zoomed {
                    dispatch_for_delegate(
                        self,
                        if zoomed {
                            DesktopEvent::WindowMaximized {
                                window_id: WindowId::PRIMARY,
                            }
                        } else {
                            DesktopEvent::WindowUnmaximized {
                                window_id: WindowId::PRIMARY,
                            }
                        },
                    );
                }
                let size = window.frame().size;
                dispatch_for_delegate(
                    self,
                    DesktopEvent::WindowResized {
                        window_id: WindowId::PRIMARY,
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
                        window_id: WindowId::PRIMARY,
                        x: clamp_coordinate(origin.x),
                        y: clamp_coordinate(origin.y),
                    },
                );
            }
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn windowDidChangeBackingProperties(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                align_overlay_controls(window, &self.ivars().options);
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
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowDidDeminiaturize:))]
        fn windowDidDeminiaturize(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowRestored {
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowDidBecomeKey:))]
        fn windowDidBecomeKey(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowFocused {
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowDidResignKey:))]
        fn windowDidResignKey(&self, _notification: &NSNotification) {
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowBlurred {
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowDidEnterFullScreen:))]
        fn windowDidEnterFullScreen(&self, _notification: &NSNotification) {
            #[cfg(feature = "local-server")]
            self.update_local_caption_insets(true);
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowEnteredFullscreen {
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowDidExitFullScreen:))]
        fn windowDidExitFullScreen(&self, _notification: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                align_overlay_controls(window, &self.ivars().options);
            }
            #[cfg(feature = "local-server")]
            self.update_local_caption_insets(false);
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowLeftFullscreen {
                    window_id: WindowId::PRIMARY,
                },
            );
        }
        #[unsafe(method(windowWillClose:))]
        fn windowWillClose(&self, _notification: &NSNotification) {
            // AppKit termination can reenter this notification while closing
            // the same window. Claim teardown before any native call or callback.
            if self.ivars().exiting.replace(true) {
                return;
            }
            #[cfg(feature = "local-server")]
            if let Some(handler) = self.ivars().host_message_handler.get() {
                handler.close();
            }
            #[cfg(feature = "local-server")]
            self.cancel_quit_deadline();
            #[cfg(feature = "native-url-activation")]
            if let Some(sender) = &self.ivars().url_activation {
                sender.close();
                self.ivars().pending_url_activations.borrow_mut().clear();
            }
            if let Some(wake) = self.ivars().command_wake.get() {
                wake.close();
            }
            #[cfg(feature = "native-services")]
            if let Some(registration) = self.ivars().geometry_registration.get() {
                registration.close();
            }
            #[cfg(feature = "native-services")]
            if let Some(registration) = self.ivars().theme_registration.get() {
                registration.close();
            }
            #[cfg(feature = "local-server")]
            if let Some(wake) = self.ivars().owner_close_wake.get() {
                wake.close();
            }
            #[cfg(feature = "local-server")]
            self.ivars().owner_close_registration.borrow_mut().take();
            #[cfg(feature = "application-ipc")]
            if let Some(ipc) = &self.ivars().ipc {
                ipc.close();
            }
            persist_window_state_if_enabled(self);
            dispatch_for_delegate(
                self,
                DesktopEvent::WindowClosed {
                    window_id: WindowId::PRIMARY,
                },
            );
            dispatch_for_delegate(self, DesktopEvent::Exiting);
            // SAFETY: Called on the main thread by AppKit while the shared app exists.
            let app = NSApplication::sharedApplication(self.mtm());
            #[cfg(feature = "local-server")]
            if self.ivars().local_origin.is_some() {
                super::stop_local_app(&app);
                return;
            }
            app.terminate(None);
        }
    }

    impl DesktopAppDelegate {
        #[cfg(feature = "local-server")]
        #[unsafe(method(localQuitCloseTimedOut:))]
        fn local_quit_close_timed_out(&self, _timer: &NSTimer) {
            if !self.ivars().quit_close_pending.get() || self.ivars().exiting.get() {
                return;
            }
            self.ivars().quit_close_deadline.borrow_mut().take();
            self.ivars().startup_error.replace(Some(local_quit_error(
                "AppKit Quit window close was not acknowledged within 15 seconds",
            )));
            // The queue wake was not a close acknowledgement. Try a direct
            // cancellable AppKit close, but never return a frame with a live
            // window and unpinned HTTP origin. A veto keeps the session alive.
            if let Some(window) = self.ivars().window.get() {
                window.performClose(None);
            }
            if self.ivars().quit_close_pending.get() && !self.ivars().exiting.get() {
                self.ivars().quit_close_pending.set(false);
                eprintln!("WebUI: AppKit Quit close remains unacknowledged; keep the listener bound and close the window before retiring the host");
            }
        }
    }
);

#[cfg(feature = "local-server")]
fn local_quit_error(message: &str) -> crate::DesktopError {
    crate::DesktopError::Backend {
        source: Box::new(std::io::Error::other(format!(
            "{message}; keep the listener bound until WindowClosed and inspect the native close failure"
        ))),
    }
}

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
    #[cfg(feature = "local-server")]
    fn update_local_caption_insets(&self, fullscreen: bool) {
        let ivars = self.ivars();
        if !matches!(
            ivars.options.titlebar,
            crate::TitlebarStyle::HiddenInset | crate::TitlebarStyle::Overlay { .. }
        ) || !ivars
            .lifetime
            .as_ref()
            .is_some_and(crate::HostLifetime::is_active)
        {
            return;
        }
        let Some((origin, webview)) = ivars.local_origin.as_ref().zip(ivars.webview.get()) else {
            return;
        };
        // SAFETY: AppKit supplies this callback on the owning window's UI thread.
        if (unsafe { webview.URL() })
            .and_then(|url| url.absoluteString())
            .is_some_and(|url| origin.allows(&url.to_string()))
        {
            super::commands::update_local_caption_insets(webview, fullscreen);
        }
    }

    #[cfg(feature = "native-url-activation")]
    fn receive_open_urls(&self, urls: &NSArray<NSURL>) {
        use crate::local_server::url_activation::{reject, Rejection};

        let Some(sender) = &self.ivars().url_activation else {
            return;
        };
        let count = urls.count();
        if count > crate::MAX_URL_ACTIVATIONS_PER_BATCH {
            reject(Rejection::TooMany);
            return;
        }
        for index in 0..count {
            let url = urls.objectAtIndex(index);
            let Some(raw) = url.absoluteString() else {
                reject(Rejection::InvalidUrl);
                continue;
            };
            // NSString length bounds conversion to at most 4x this many
            // UTF-8 bytes before the exact byte limit is checked.
            if raw.length() > crate::MAX_URL_ACTIVATION_BYTES {
                reject(Rejection::TooLong);
                continue;
            }
            match sender.accept(&raw.to_string()) {
                Ok(activation) if self.ivars().url_activation_ready.get() => {
                    sender.send(activation)
                }
                Ok(activation) => {
                    let mut pending = self.ivars().pending_url_activations.borrow_mut();
                    if pending.len() < crate::MAX_URL_ACTIVATIONS_PER_BATCH {
                        pending.push(activation);
                    } else {
                        reject(Rejection::Full);
                    }
                }
                Err(reason) => reject(reason),
            }
        }
    }

    #[cfg(feature = "native-url-activation")]
    pub(super) fn url_window_ready(&self) {
        self.ivars().url_activation_ready.set(true);
        if let Some(sender) = &self.ivars().url_activation {
            for activation in self.ivars().pending_url_activations.borrow_mut().drain(..) {
                sender.send(activation);
            }
        }
    }

    #[cfg(feature = "local-server")]
    pub(super) fn cancel_quit_deadline(&self) {
        self.ivars().quit_close_pending.set(false);
        if let Some(timer) = self.ivars().quit_close_deadline.borrow_mut().take() {
            timer.invalidate();
        }
    }

    pub(super) fn new(mtm: MainThreadMarker, options: MacosLaunchOptions) -> Retained<Self> {
        let maximized = options.options.maximized;
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            executor: options.executor,
            runtime: options.runtime,
            #[cfg(feature = "local-server")]
            local_origin: options.local_origin,
            #[cfg(feature = "local-server")]
            local_url: options.local_url,
            #[cfg(feature = "local-server")]
            lifetime: options.lifetime,
            #[cfg(feature = "local-server")]
            frame_policy: options.frame_policy,
            #[cfg(feature = "native-url-activation")]
            url_activation: options.url_activation,
            #[cfg(feature = "native-url-activation")]
            pending_url_activations: RefCell::new(Vec::new()),
            #[cfg(feature = "native-url-activation")]
            url_activation_ready: Cell::new(false),
            #[cfg(feature = "native-services")]
            native_services: options.native_services,
            #[cfg(feature = "native-services")]
            geometry_registration: OnceCell::new(),
            #[cfg(feature = "native-services")]
            theme_registration: OnceCell::new(),
            #[cfg(feature = "local-server")]
            owner_close_wake: OnceCell::new(),
            #[cfg(feature = "local-server")]
            owner_close_registration: std::cell::RefCell::new(None),
            #[cfg(feature = "local-server")]
            startup_error: std::cell::RefCell::new(None),
            #[cfg(feature = "local-server")]
            quit_close_pending: Cell::new(false),
            #[cfg(feature = "local-server")]
            quit_close_deadline: RefCell::new(None),
            live_background: options.live_background,
            command_wake: OnceCell::new(),
            #[cfg(feature = "application-ipc")]
            ipc: options.ipc,
            window: OnceCell::new(),
            webview: OnceCell::new(),
            scheme_handler: OnceCell::new(),
            navigation_delegate: OnceCell::new(),
            host_message_handler: OnceCell::new(),
            theme_observer: OnceCell::new(),
            tray_item: OnceCell::new(),
            main_menu: OnceCell::new(),
            title: options.title,
            options: options.options,
            shell: options.shell,
            events: options.events,
            window_handle: options.window_handle,
            state_store: options.state_store,
            #[cfg(feature = "local-server")]
            persistent_website_data: options.persistent_website_data,
            zoomed: Cell::new(maximized),
            exiting: Cell::new(false),
        });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopAppDelegate;
    use objc2::{sel, ClassType};

    #[test]
    fn incoming_url_selector_requires_explicit_feature() {
        assert_eq!(
            DesktopAppDelegate::class()
                .instance_method(sel!(application:openURLs:))
                .is_some(),
            cfg!(feature = "native-url-activation"),
        );
    }
}
