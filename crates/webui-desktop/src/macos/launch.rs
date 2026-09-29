// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Window and webview construction for `applicationDidFinishLaunching:`, and
//! the `windowWillClose:` state-persistence hook. Split out of
//! [`super::app_delegate`] to keep that file's cognitive complexity low and
//! these steps independently readable.

use crate::{DesktopEvent, WindowOptions, WindowState, WindowStateStore, DRAG_REGION_SCRIPT};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{
    WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};

use super::app_delegate::DesktopAppDelegate;
use super::effects::{apply_background, install_content_view};
use super::host_message::DesktopHostMessageHandler;
use super::icon::install_app_icon;
use super::menu::build_main_menu;
use super::navigation::DesktopNavigationDelegate;
use super::scheme::DesktopSchemeHandler;
use super::state::{apply_state, capture_state, display_bounds};
use super::theme::install_theme_observer;
use super::tray::install_tray;
use super::window::{apply_window_options, DesktopWindow};
use super::{devtools_enabled_by_env, dispatch_event, startup_url};

struct WebviewHandlers<'a> {
    scheme: Option<&'a Retained<DesktopSchemeHandler>>,
    host: Option<&'a Retained<DesktopHostMessageHandler>>,
    #[cfg(feature = "application-ipc")]
    ipc: Option<&'a std::rc::Rc<super::ipc::MacIpc>>,
}

/// Build the window and webview, wire every peripheral, then show the window
/// and load the startup URL.
pub(super) fn build_window_and_webview(delegate: &DesktopAppDelegate, app: &NSApplication) {
    let mtm = delegate.mtm();
    let ivars = delegate.ivars();
    let restored_state = restore_state_if_enabled(ivars.state_store.as_ref(), mtm);
    let rect = initial_content_rect(&ivars.options, restored_state.as_ref());

    // SAFETY: NSWindow is allocated and initialized on the main thread, with a
    // valid content rect and standard style flags. `DesktopWindow` has no
    // designated initializer of its own, so the call must go through the
    // inherited `NSWindow` initializer via `super`.
    let window: Retained<DesktopWindow> = unsafe {
        msg_send![
            super(DesktopWindow::alloc(mtm).set_ivars(())),
            initWithContentRect: rect,
            styleMask: super::options::native_window_style(&ivars.options).mask,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    // SAFETY: The window is retained in the delegate OnceCell, so it must not
    // auto-release itself when closed.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&ivars.title);
    apply_window_options(&window, &ivars.options);
    if let Some(state) = &restored_state {
        apply_state(&window, state);
    } else if ivars.options.center {
        window.center();
    }

    let scheme_handler = ivars.runtime.as_ref().map(|runtime| {
        DesktopSchemeHandler::new(
            mtm,
            std::sync::Arc::clone(runtime),
            std::sync::Arc::clone(&ivars.executor),
            #[cfg(feature = "application-ipc")]
            ivars.ipc.clone(),
        )
    });
    let navigation_delegate = DesktopNavigationDelegate::new(
        mtm,
        ivars.events.clone(),
        std::sync::Arc::clone(&ivars.live_background),
        #[cfg(feature = "local-server")]
        ivars.local_origin.clone().zip(ivars.lifetime.clone()),
        #[cfg(feature = "application-ipc")]
        ivars.ipc.clone(),
    );
    let host_message_handler = ivars
        .runtime
        .as_ref()
        .map(|_| DesktopHostMessageHandler::new(mtm));
    let webview = build_webview(
        mtm,
        &ivars.options,
        WebviewHandlers {
            scheme: scheme_handler.as_ref(),
            host: host_message_handler.as_ref(),
            #[cfg(feature = "application-ipc")]
            ipc: ivars.ipc.as_ref(),
        },
        rect,
    );
    #[cfg(feature = "application-ipc")]
    if let Some(ipc) = &ivars.ipc {
        ipc.attach(&webview);
    }
    // SAFETY: `navigation_delegate` is retained in the delegate OnceCell for
    // the app lifetime. The policy implementation allows only the custom app
    // origin and cancels everything else.
    unsafe {
        webview.setNavigationDelegate(Some(ProtocolObject::from_ref(&*navigation_delegate)));
    }
    apply_background(&window, &webview, ivars.options.background);
    install_content_view(mtm, &window, &webview, ivars.options.effect);
    window.setDelegate(Some(ProtocolObject::from_ref(delegate)));

    let menu_webview = webview.clone();
    let menu = build_main_menu(mtm, &ivars.shell.menus, move |script| {
        // SAFETY: AppKit invokes menu actions on the main thread; the receiver
        // retains this webview until the owning native menu is torn down.
        unsafe {
            menu_webview.evaluateJavaScript_completionHandler(&NSString::from_str(script), None)
        };
    });
    app.setMainMenu(Some(menu.menu()));
    let _ = ivars.main_menu.set(menu);
    install_app_icon(
        app,
        ivars.shell.icon_path.as_deref(),
        ivars
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.bundle_root()),
    );
    if let Some(tray) = ivars
        .shell
        .tray
        .as_ref()
        .and_then(|tray| install_tray(mtm, tray))
    {
        let _ = ivars.tray_item.set(tray);
    }
    let theme_observer = install_theme_observer(mtm, ivars.events.clone(), webview.clone());
    let _ = ivars.theme_observer.set(theme_observer);

    let _ = ivars.command_wake.set(super::commands::install_wakeup(
        &window,
        &webview,
        &ivars.window_handle,
    ));
    let local_url = {
        #[cfg(feature = "local-server")]
        {
            ivars.local_url.as_deref()
        }
        #[cfg(not(feature = "local-server"))]
        {
            None
        }
    };
    let _ = ivars.window.set(window.clone());
    let _ = ivars.webview.set(webview.clone());
    if let Some(scheme_handler) = scheme_handler {
        let _ = ivars.scheme_handler.set(scheme_handler);
    }
    let _ = ivars.navigation_delegate.set(navigation_delegate);
    if let Some(host_message_handler) = host_message_handler {
        let _ = ivars.host_message_handler.set(host_message_handler);
    }
    #[cfg(feature = "local-server")]
    if let Some(lifetime) = &ivars.lifetime {
        match super::commands::install_owner_close(&window, lifetime) {
            Ok((wake, registration)) => {
                let _ = ivars.owner_close_wake.set(wake);
                ivars.owner_close_registration.replace(Some(registration));
            }
            Err(error) => {
                abort_local_startup(delegate, app, &window, error);
                return;
            }
        }
        if !lifetime.is_active() {
            abort_local_startup(delegate, app, &window, crate::local_server::retired_host());
            return;
        }
    }
    window.makeKeyAndOrderFront(None);
    load_startup_url(&webview, local_url);
    #[cfg(feature = "local-server")]
    if ivars
        .lifetime
        .as_ref()
        .is_some_and(|lifetime| !lifetime.is_active())
    {
        // SAFETY: This is the UI thread and the view is live. A revoke between
        // the last admission check and load cannot leave a pending document.
        unsafe { webview.stopLoading() };
        abort_local_startup(delegate, app, &window, crate::local_server::retired_host());
        return;
    }
    dispatch_event(&ivars.events, &webview, DesktopEvent::Ready);
}

#[cfg(feature = "local-server")]
fn abort_local_startup(
    delegate: &DesktopAppDelegate,
    app: &NSApplication,
    window: &DesktopWindow,
    error: crate::DesktopError,
) {
    delegate.ivars().startup_error.replace(Some(error));
    // Skip the normal windowWillClose termination path: no document was
    // admitted, so AppKit must return the recoverable startup error.
    window.setDelegate(None);
    window.close();
    app.stop(None);
}

fn initial_content_rect(options: &WindowOptions, restored: Option<&WindowState>) -> NSRect {
    let (width, height) = restored.map_or((options.width, options.height), |state| {
        (state.width, state.height)
    });
    NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(f64::from(width), f64::from(height)),
    )
}

fn restore_state_if_enabled(
    store: Option<&WindowStateStore>,
    mtm: MainThreadMarker,
) -> Option<WindowState> {
    let store = store?;
    let displays = display_bounds(mtm);
    match store.load_valid(&displays) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("WebUI: failed to restore window state: {error}");
            None
        }
    }
}

/// Persist the window's current geometry when `remember_state` is enabled.
/// Called once from `windowWillClose:`, not on every resize/move tick, to
/// avoid unnecessary disk writes during live drag.
pub(super) fn persist_window_state_if_enabled(delegate: &DesktopAppDelegate) {
    let ivars = delegate.ivars();
    let Some(store) = ivars.state_store.as_ref() else {
        return;
    };
    let Some(window) = ivars.window.get() else {
        return;
    };
    let state = capture_state(window);
    if let Err(error) = store.save(&state) {
        eprintln!("WebUI: failed to persist window state: {error}");
    }
}

fn build_webview(
    mtm: MainThreadMarker,
    options: &WindowOptions,
    handlers: WebviewHandlers<'_>,
    rect: NSRect,
) -> Retained<WKWebView> {
    // SAFETY: WKWebViewConfiguration::new and WKWebView initialization must
    // run on the main thread; mtm proves this. The frame is valid.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        // SAFETY: The handler object lives for the app lifetime via the
        // `scheme_handler` OnceCell in the caller, and WebKit calls it only on
        // the main thread for the registered custom scheme.
        if let Some(scheme_handler) = handlers.scheme {
            config.setURLSchemeHandler_forURLScheme(
                Some(ProtocolObject::from_ref(&**scheme_handler)),
                &NSString::from_str("webui"),
            );
        }
        config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
        let content = config.userContentController();
        if let Some(host_message_handler) = handlers.host {
            let mut source = String::with_capacity(DRAG_REGION_SCRIPT.len() + 88);
            source.push_str(
            "window.webuiHostPostMessage=m=>window.webkit.messageHandlers.webuiHost.postMessage(m);",
        );
            source.push_str(DRAG_REGION_SCRIPT);
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(&source),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            );
            content.addUserScript(&script);
            content.addScriptMessageHandler_name(
                ProtocolObject::from_ref(&**host_message_handler),
                &NSString::from_str("webuiHost"),
            );
        }
        #[cfg(feature = "application-ipc")]
        if let Some(ipc) = handlers.ipc {
            let handler =
                super::ipc_message::DesktopIpcMessageHandler::new(mtm, std::rc::Rc::clone(ipc));
            content.addScriptMessageHandlerWithReply_contentWorld_name(
                ProtocolObject::from_ref(&*handler),
                &objc2_web_kit::WKContentWorld::pageWorld(mtm),
                &NSString::from_str("webuiDesktopIpc"),
            );
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(if ipc.is_local() {
                    #[cfg(feature = "local-server")]
                    {
                        crate::ipc_assets::LOCAL_NATIVE_BOOTSTRAP_SCRIPT
                    }
                    #[cfg(not(feature = "local-server"))]
                    {
                        crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT
                    }
                } else {
                    crate::ipc_assets::NATIVE_BOOTSTRAP_SCRIPT
                }),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            );
            content.addUserScript(&script);
            #[cfg(feature = "local-server")]
            if ipc.is_local() {
                let data = super::ipc_data_message::DesktopIpcDataHandler::new(
                    mtm,
                    std::rc::Rc::clone(ipc),
                );
                content.addScriptMessageHandlerWithReply_contentWorld_name(
                    ProtocolObject::from_ref(&*data),
                    &objc2_web_kit::WKContentWorld::pageWorld(mtm),
                    &NSString::from_str("webuiDesktopIpcData"),
                );
            }
        }
        let webview = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config);
        if options.devtools || devtools_enabled_by_env() {
            // SAFETY: `setInspectable:` is a WebKit setter on a live WKWebView
            // created on the main thread. It only enables Safari Web Inspector
            // for this development webview.
            webview.setInspectable(true);
        }
        webview.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        webview
    }
}

fn load_startup_url(webview: &WKWebView, local_url: Option<&str>) {
    let startup = local_url.map_or_else(startup_url, str::to_string);
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(&startup)) {
        let request = NSURLRequest::requestWithURL(&url);
        // SAFETY: The request URL uses the registered custom scheme.
        unsafe {
            let _ = webview.loadRequest(&request);
        }
    }
}
