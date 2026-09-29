// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Direct HTTP WebKitGTK window. Never uses the bundled scheme or host-control
//! handlers; optional owned IPC gets a separate top-frame isolated manager.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use gtk4::{gdk, gio, glib, prelude::*, Application, ApplicationWindow};
#[cfg(feature = "application-ipc")]
use webkit6::UserContentManager;
use webkit6::{prelude::*, LoadEvent, PolicyDecisionType, WebContext, WebView};

use crate::local_server::{HostCloseError, HostCloseRegistration};
use crate::{
    DesktopEvent, EventResponse, LocalServerFrame, WindowCommand, WindowId, WindowStateStore,
};

use super::backend::{
    configure_constraints, persist_state, restore_state, run_application, safe_dimension,
    to_gdk_rgba,
};

const WINDOW_ID: WindowId = WindowId::PRIMARY;

// The native objects must never cross threads. Both wakes capture only Send
// data and hop to the default GTK main context before touching this slot.
thread_local! {
    static LOCAL_WINDOW: RefCell<Option<LocalWindow>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct LocalWindow {
    window: ApplicationWindow,
    webview: WebView,
    commands: crate::WindowHandle,
    events: crate::EventRegistry,
    lifetime: crate::HostLifetime,
}

/// UI-thread launch state shared by activation, native callbacks and teardown.
struct LocalLaunch {
    state_store: Option<WindowStateStore>,
    running: Arc<AtomicBool>,
    registration: RefCell<Option<HostCloseRegistration>>,
    startup_error: Rc<RefCell<Option<anyhow::Error>>>,
    closed: Rc<Cell<bool>>,
    #[cfg(feature = "application-ipc")]
    ipc: Rc<RefCell<Option<Rc<super::local_ipc::GtkLocalIpc>>>>,
}

/// Run the already-bound loopback server as the main WebKitGTK document.
///
/// The host retains the listener and lifetime owner until `WindowClosed`.
/// A close wake is independent of the bounded window command queue.
pub(crate) fn run_local_server_frame(frame: LocalServerFrame) -> Result<()> {
    frame.lifetime().require_active()?;

    let state_store =
        WindowStateStore::for_window(frame.window.remember_state, frame.app_id.as_deref())?;
    let builder = match frame
        .app_id
        .as_deref()
        .filter(|id| gio::Application::id_is_valid(id))
    {
        Some(id) => Application::builder().application_id(id),
        None => Application::builder().flags(gio::ApplicationFlags::NON_UNIQUE),
    };
    let frame = Rc::new(frame);
    let app = builder.build();
    let launch = Rc::new(LocalLaunch {
        state_store,
        running: Arc::new(AtomicBool::new(false)),
        registration: RefCell::new(None),
        startup_error: Rc::new(RefCell::new(None)),
        closed: Rc::new(Cell::new(false)),
        #[cfg(feature = "application-ipc")]
        ipc: Rc::new(RefCell::new(None)),
    });
    let activation = {
        let frame = Rc::clone(&frame);
        let launch = Rc::clone(&launch);
        app.connect_activate(move |app| {
            // A second GApplication activation cannot replace the close
            // registration/window generation while one is still running.
            if launch.running.load(Ordering::Acquire) {
                if let Some(window) = LOCAL_WINDOW.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .and_then(|local| local.lifetime.is_active().then(|| local.window.clone()))
                }) {
                    window.present();
                }
                return;
            }
            if let Err(error) = build_window(app, &frame, &launch) {
                launch.running.store(false, Ordering::Release);
                *launch.startup_error.borrow_mut() = Some(error);
                app.quit();
            }
        })
    };
    let exit = run_application(&app);
    // Retain native state for an actionable error, but never synthesize a
    // missing window-removed event during teardown.
    let native_window =
        LOCAL_WINDOW.with(|slot| slot.borrow().as_ref().map(|local| local.window.clone()));
    launch.running.store(false, Ordering::Release);
    let remaining_windows = app.windows().len();
    let native_state = native_window
        .as_ref()
        .map(|window| (window.is_visible(), window.is_realized()));
    app.disconnect(activation);
    #[cfg(feature = "application-ipc")]
    if let Some(ipc) = launch.ipc.borrow_mut().take() {
        ipc.close();
    }
    // Drop the registration before the UI slot so a concurrent owner revoke
    // cannot schedule a close against a subsequent window generation.
    launch.registration.borrow_mut().take();
    LOCAL_WINDOW.with(|slot| {
        slot.borrow_mut().take();
    });
    let _ = frame.events.dispatch(&DesktopEvent::Exiting);
    if let Some(error) = launch.startup_error.borrow_mut().take() {
        return Err(error);
    }
    if exit != glib::ExitCode::SUCCESS {
        return Err(anyhow!(
            "GTK application exited with {exit:?}; check native startup diagnostics"
        ));
    }
    if remaining_windows != 0 || !launch.closed.get() {
        return Err(anyhow!(
            "GTK local-server event loop exited without a verified native window removal \
             (registered_windows={remaining_windows}, window_removed={}, \
              local_visible_realized={native_state:?})",
            launch.closed.get()
        ));
    }
    Ok(())
}

fn build_window(app: &Application, frame: &LocalServerFrame, launch: &LocalLaunch) -> Result<()> {
    frame.lifetime().require_active()?;
    // The default path has no content manager, script or native handler.
    // The isolated manager exists only for a separately proven owned listener.
    let context = WebContext::new();
    #[cfg(feature = "application-ipc")]
    let manager = frame.ipc_owner.as_ref().map(|_| UserContentManager::new());
    #[cfg(feature = "application-ipc")]
    if let Some(manager) = manager.as_ref() {
        super::local_ipc::GtkLocalIpc::prepare(manager)?;
    }
    let builder = WebView::builder().web_context(&context);
    #[cfg(feature = "application-ipc")]
    let builder = if let Some(manager) = manager.as_ref() {
        builder.user_content_manager(manager)
    } else {
        builder
    };
    let webview = builder.build();
    #[cfg(feature = "application-ipc")]
    if let Some(manager) = manager.as_ref() {
        *launch.ipc.borrow_mut() = Some(super::local_ipc::GtkLocalIpc::install(
            frame, &webview, manager,
        )?);
    }
    // A host may queue a background change before the native loop starts.
    // Reflect that latest native color on the first WebView paint.
    if let Some(color) = frame.live_background.current().or(frame.window.background) {
        webview.set_background_color(&to_gdk_rgba(color));
    }
    let window = ApplicationWindow::builder()
        .application(app)
        .title(&frame.window.title)
        .default_width(safe_dimension(frame.window.width, 1200))
        .default_height(safe_dimension(frame.window.height, 800))
        .resizable(frame.window.resizable)
        .build();
    configure_constraints(&window, &frame.window);
    window.set_child(Some(&webview));
    restore_state(&window, launch.state_store.as_ref());

    let local = LocalWindow {
        window: window.clone(),
        webview: webview.clone(),
        commands: frame.window_handle.clone(),
        events: frame.events.clone(),
        lifetime: frame.lifetime().clone(),
    };
    install_navigation(&local, frame, app, &launch.startup_error);
    install_events(&local, launch.state_store.clone());
    let target = window.downgrade();
    let closed = Rc::clone(&launch.closed);
    let events = frame.events.clone();
    #[cfg(feature = "application-ipc")]
    let ipc = Rc::clone(&launch.ipc);
    app.connect_window_removed(move |_, removed| {
        // A WeakRef names this exact window generation without extending its
        // lifetime. Neither close-request nor widget destroy alone certifies
        // that GtkApplication removed the native window.
        let this_window = target
            .upgrade()
            .is_some_and(|window| removed.as_ptr().cast::<()>() == window.as_ptr().cast::<()>());
        if this_window && !closed.replace(true) {
            #[cfg(feature = "application-ipc")]
            if let Some(state) = ipc.borrow_mut().take() {
                state.close();
            }
            let _ = events.dispatch(&DesktopEvent::WindowClosed {
                window_id: WINDOW_ID,
            });
        }
    });
    LOCAL_WINDOW.with(|slot| {
        *slot.borrow_mut() = Some(local);
    });
    let active = Arc::clone(&launch.running);
    let close_registration = frame.lifetime().register_close_fallible(Arc::new(move || {
        if !active.load(Ordering::Acquire) {
            return Err(HostCloseError::WakeFailed {
                message: "GTK local-server window is no longer running".to_string(),
            });
        }
        // Not the bounded WindowHandle queue: owner loss always has its own
        // source, even when page/host command producers have filled that queue.
        // SourceId wraps NonZeroU32: GLib has attached this independent
        // source to its default main context when this call returns.
        let _source = glib::idle_add_once(|| {
            let local = LOCAL_WINDOW.with(|slot| slot.borrow().clone());
            if let Some(local) = local {
                if !local.lifetime.is_active() {
                    local.webview.stop_loading();
                    // destroy() cannot be vetoed by WindowCloseRequested.
                    local.window.destroy();
                }
            }
        });
        Ok(())
    }))?;
    *launch.registration.borrow_mut() = Some(close_registration);
    launch.running.store(true, Ordering::Release);

    let handle = frame.window_handle.clone();
    handle.set_wakeup(|| {
        glib::idle_add_once(|| {
            let local = LOCAL_WINDOW.with(|slot| slot.borrow().clone());
            if let Some(local) = local {
                execute_commands(&local);
            }
        });
    });
    if frame.window.maximized {
        window.maximize();
    }
    if frame.window.fullscreen {
        window.fullscreen();
    }
    if frame.window.always_on_top {
        eprintln!("WebUI: always_on_top is unavailable on Wayland and best-effort on X11");
    }
    if frame.window.center {
        eprintln!("WebUI: centering is compositor-controlled on Wayland");
    }
    if !frame.lifetime().is_active() {
        window.destroy();
        return Err(crate::local_server::retired_host().into());
    }
    window.present();
    if !frame.lifetime().is_active() {
        window.destroy();
        return Err(crate::local_server::retired_host().into());
    }
    // This is a real browser load, not a custom scheme/proxy or buffered
    // response. WebKit owns HTTP redirects, Range, cookies, CSP and streaming.
    webview.load_uri(&frame.options.url());
    if !frame.lifetime().is_active() {
        webview.stop_loading();
        window.destroy();
        return Err(crate::local_server::retired_host().into());
    }
    let _ = frame.events.dispatch(&DesktopEvent::Ready);
    if !frame.lifetime().is_active() {
        webview.stop_loading();
        window.destroy();
        return Err(crate::local_server::retired_host().into());
    }
    Ok(())
}

fn install_navigation(
    local: &LocalWindow,
    frame: &LocalServerFrame,
    app: &Application,
    startup_error: &Rc<RefCell<Option<anyhow::Error>>>,
) {
    let origin = frame.origin().clone();
    let lifetime = frame.lifetime().clone();
    let events = local.events.clone();
    let denied = Rc::new(RefCell::new(None::<String>));
    let denied_in_policy = Rc::clone(&denied);
    webkit6::prelude::WebViewExt::connect_decide_policy(
        &local.webview,
        move |_, decision, kind| {
            match kind {
                PolicyDecisionType::NewWindowAction => {
                    decision.ignore();
                }
                PolicyDecisionType::NavigationAction => {
                    let url = decision
                        .downcast_ref::<webkit6::NavigationPolicyDecision>()
                        .and_then(|navigation| navigation.navigation_action())
                        .and_then(|action| action.request())
                        .and_then(|request| request.uri());
                    let allowed = url.as_ref().is_some_and(|url| {
                        lifetime.allows_navigation(&origin, url.as_str())
                        && events.dispatch(&DesktopEvent::NavigationRequested {
                            window_id: WINDOW_ID,
                            url: url.to_string(),
                        }) == EventResponse::Continue
                        // Event handlers can revoke the owner synchronously.
                        && lifetime.allows_navigation(&origin, url.as_str())
                    });
                    if allowed {
                        decision.use_();
                    } else {
                        *denied_in_policy.borrow_mut() = url.map(|url| url.to_string());
                        decision.ignore();
                    }
                }
                PolicyDecisionType::Response => {
                    let response = decision.downcast_ref::<webkit6::ResponsePolicyDecision>();
                    let url = response
                        .and_then(|response| response.response())
                        .and_then(|response| response.uri());
                    let allowed = response
                        .is_some_and(|response| response.is_main_frame_main_resource())
                        && url
                            .as_ref()
                            .is_some_and(|url| lifetime.allows_navigation(&origin, url.as_str()));
                    if allowed {
                        decision.use_();
                    } else {
                        *denied_in_policy.borrow_mut() = url.map(|url| url.to_string());
                        decision.ignore();
                    }
                }
                _ => decision.ignore(),
            }
            true
        },
    );
    // No WebView is created for target=_blank/window.open, including an
    // about:blank popup. Response policy rejects subframe DOCUMENT responses;
    // WebKitGTK does not expose source-frame identity at action time.
    local.webview.connect_create(|_, _| None);
    local.webview.connect_permission_request(|_, request| {
        request.deny();
        true
    });
    let lifetime = frame.lifetime().clone();
    let window = local.window.clone();
    let events = local.events.clone();
    let origin = frame.origin().clone();
    local.webview.connect_load_changed(move |webview, state| {
        if !lifetime.is_active() {
            webview.stop_loading();
            window.destroy();
            return;
        }
        if state == LoadEvent::Committed
            && !webview
                .uri()
                .is_some_and(|url| lifetime.allows_navigation(&origin, url.as_str()))
        {
            webview.stop_loading();
            window.destroy();
            return;
        }
        if state == LoadEvent::Finished {
            if let Some(url) = webview.uri() {
                if !lifetime.allows_navigation(&origin, url.as_str()) {
                    webview.stop_loading();
                    window.destroy();
                    return;
                }
                let _ = events.dispatch(&DesktopEvent::NavigationCompleted {
                    window_id: WINDOW_ID,
                    url: url.to_string(),
                });
                if !lifetime.is_active() {
                    webview.stop_loading();
                    window.destroy();
                }
            }
        }
    });
    let lifetime = frame.lifetime().clone();
    let origin = frame.origin().clone();
    let error_slot = Rc::clone(startup_error);
    let window = local.window.clone();
    let app_for_failure = app.clone();
    let denied = Rc::clone(&denied);
    local.webview.connect_load_failed(move |_, _, url, error| {
        if denied.borrow_mut().take().as_deref() == Some(url) {
            return true;
        }
        if lifetime.allows_navigation(&origin, url) {
            *error_slot.borrow_mut() = Some(anyhow!("local HTTP document {url} failed: {error}"));
            window.destroy();
            app_for_failure.quit();
        }
        // Never substitute a WebKit-generated error document for the server.
        true
    });
    let error_slot = Rc::clone(startup_error);
    let window = local.window.clone();
    let app_for_failure = app.clone();
    local
        .webview
        .connect_web_process_terminated(move |_, reason| {
            *error_slot.borrow_mut() =
                Some(anyhow!("local HTTP WebKit process terminated: {reason:?}"));
            window.destroy();
            app_for_failure.quit();
        });
}

fn install_events(local: &LocalWindow, store: Option<WindowStateStore>) {
    let events = local.events.clone();
    let lifetime = local.lifetime.clone();
    local.window.connect_close_request(move |_| {
        let response = events.dispatch(&DesktopEvent::WindowCloseRequested {
            window_id: WINDOW_ID,
        });
        if response == EventResponse::PreventDefault && lifetime.is_active() {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    for property in ["width", "height"] {
        let events = local.events.clone();
        let store = store.clone();
        local
            .window
            .connect_notify_local(Some(property), move |window, _| {
                let _ = events.dispatch(&DesktopEvent::WindowResized {
                    window_id: WINDOW_ID,
                    width: u32::try_from(window.width()).unwrap_or(0),
                    height: u32::try_from(window.height()).unwrap_or(0),
                });
                persist_state(window, store.as_ref());
            });
    }
    let events = local.events.clone();
    local.window.connect_is_active_notify(move |window| {
        let _ = events.dispatch(&if window.is_active() {
            DesktopEvent::WindowFocused {
                window_id: WINDOW_ID,
            }
        } else {
            DesktopEvent::WindowBlurred {
                window_id: WINDOW_ID,
            }
        });
    });
    let events = local.events.clone();
    local.window.connect_scale_factor_notify(move |window| {
        let _ = events.dispatch(&DesktopEvent::ScaleFactorChanged {
            scale: f64::from(window.scale_factor()),
        });
    });
    let events = local.events.clone();
    local.window.connect_maximized_notify(move |window| {
        persist_state(window, store.as_ref());
        let _ = events.dispatch(&if window.is_maximized() {
            DesktopEvent::WindowMaximized {
                window_id: WINDOW_ID,
            }
        } else {
            DesktopEvent::WindowUnmaximized {
                window_id: WINDOW_ID,
            }
        });
    });
    if let Some(settings) = gtk4::Settings::default() {
        let events = local.events.clone();
        settings.connect_gtk_application_prefer_dark_theme_notify(move |settings| {
            let _ = events.dispatch(&DesktopEvent::ThemeChanged {
                dark: settings.is_gtk_application_prefer_dark_theme(),
            });
        });
    }
    let events = local.events.clone();
    let previous = Rc::new(Cell::new(None));
    local.window.connect_realize(move |window| {
        let Some(surface) = window.surface() else {
            return;
        };
        let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() else {
            return;
        };
        if previous.get().is_some() {
            return;
        }
        let before = state_flags(toplevel.state());
        previous.set(Some(before));
        let events = events.clone();
        let previous = Rc::clone(&previous);
        toplevel.connect_state_notify(move |toplevel| {
            let current = state_flags(toplevel.state());
            let before = previous.replace(Some(current)).unwrap_or_default();
            // Avoid the bundled state helper: it mirrors native events into
            // page JavaScript, which is not part of a local HTTP document.
            if before.0 != current.0 {
                let _ = events.dispatch(&if current.0 {
                    DesktopEvent::WindowMinimized {
                        window_id: WINDOW_ID,
                    }
                } else {
                    DesktopEvent::WindowRestored {
                        window_id: WINDOW_ID,
                    }
                });
            }
            if before.2 != current.2 {
                let _ = events.dispatch(&if current.2 {
                    DesktopEvent::WindowEnteredFullscreen {
                        window_id: WINDOW_ID,
                    }
                } else {
                    DesktopEvent::WindowLeftFullscreen {
                        window_id: WINDOW_ID,
                    }
                });
            }
        });
    });
}

fn state_flags(state: gdk::ToplevelState) -> (bool, bool, bool) {
    (
        state.contains(gdk::ToplevelState::MINIMIZED),
        state.contains(gdk::ToplevelState::MAXIMIZED),
        state.contains(gdk::ToplevelState::FULLSCREEN),
    )
}

fn execute_commands(local: &LocalWindow) {
    for command in local.commands.drain_commands() {
        match command {
            WindowCommand::SetTitle(title) => local.window.set_title(Some(&title)),
            WindowCommand::SetBackground(color) => {
                local.webview.set_background_color(&to_gdk_rgba(color));
            }
            WindowCommand::SetSize { width, height } => local
                .window
                .set_default_size(safe_dimension(width, 1200), safe_dimension(height, 800)),
            WindowCommand::Minimize => {
                if let Some(surface) = local.window.surface() {
                    if let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() {
                        let _ = toplevel.minimize();
                    }
                }
            }
            WindowCommand::Maximize => local.window.maximize(),
            WindowCommand::Unmaximize => local.window.unmaximize(),
            WindowCommand::SetFullscreen(true) => local.window.fullscreen(),
            WindowCommand::SetFullscreen(false) => local.window.unfullscreen(),
            WindowCommand::Center => {
                eprintln!("WebUI: centering is compositor-controlled on Wayland");
            }
            WindowCommand::Focus => local.window.present(),
            WindowCommand::Close => local.window.close(),
            WindowCommand::StartDrag => {
                eprintln!("WebUI: drag requires a native pointer gesture on GTK");
            }
            WindowCommand::SetAlwaysOnTop(true) => {
                eprintln!("WebUI: always_on_top is not portable in GTK4/Wayland");
            }
            WindowCommand::SetAlwaysOnTop(false) => {}
        }
    }
}
