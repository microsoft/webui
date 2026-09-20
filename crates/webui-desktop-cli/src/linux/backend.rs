// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::RefCell;
use std::sync::Arc;

use anyhow::Result;
use gtk4::{gdk, gio, glib, prelude::*, Application, ApplicationWindow, HeaderBar};
use webkit6::{
    prelude::*, LoadEvent, PolicyDecisionType, UserContentInjectedFrames, UserContentManager,
    UserScript, UserScriptInjectionTime, WebContext, WebView,
};
use webui_desktop::{
    DesktopEvent, DesktopHostMessage, DesktopRuntime, DisplayBounds, EventResponse, Rgba,
    TitlebarStyle, WindowCommand, WindowEffect, WindowHandle, WindowId, WindowState,
    WindowStateStore, DRAG_REGION_SCRIPT,
};

use super::protocol::{handle_scheme_request, startup_url};
use crate::DesktopFrame;

const WINDOW_ID: WindowId = WindowId::PRIMARY;
const SCRIPT_HANDLER: &str = "webuiHost";

/// Host bridge exposed to web content.
///
/// WebKitGTK exposes a registered handler as
/// `window.webkit.messageHandlers.<name>`, so [`DRAG_REGION_SCRIPT`] cannot
/// reach it until this alias is defined. Without the alias every drag,
/// minimize, maximize, and close message from page JavaScript is silently
/// dropped.
const HOST_BRIDGE_SCRIPT: &str = "(()=>{if(!window.webkit?.messageHandlers?.webuiHost)return;\
window.webuiHostPostMessage=m=>window.webkit.messageHandlers.webuiHost.postMessage(String(m));})();";

/// Run a packaged WebUI desktop app on Linux using GTK4 and WebKitGTK 6.
///
/// # Errors
///
/// Returns an error when packaged resources cannot be located or GTK cannot run.
pub fn run_packaged_app() -> Result<()> {
    crate::run_packaged_app()
}

/// Run a prebuilt desktop runtime in a GTK4/WebKitGTK 6 window.
///
/// # Errors
///
/// Returns an error if GTK cannot initialize.
pub fn run_runtime(
    runtime: Arc<DesktopRuntime>,
    window: webui_desktop::WindowOptions,
) -> Result<()> {
    run_frame(DesktopFrame::new(runtime, window))
}

pub(crate) fn run_frame(frame: DesktopFrame) -> Result<()> {
    let app = Application::builder()
        .application_id("com.microsoft.webui.desktop")
        .build();
    let events = frame.events.clone();
    app.connect_activate(move |app| build_window(app, frame.clone()));
    app.run();
    // The window and its `WebView` are already destroyed once `app.run()`
    // returns, so there is no page left to mirror this event into; only the
    // native handler registry can still observe it.
    let _ = events.dispatch(&DesktopEvent::Exiting);
    Ok(())
}

fn build_window(app: &Application, frame: DesktopFrame) {
    let context = WebContext::new();
    let runtime = Arc::clone(&frame.runtime);
    context.register_uri_scheme("webui", move |request| {
        handle_scheme_request(request, &runtime);
    });

    let manager = UserContentManager::new();
    let mut source = String::with_capacity(HOST_BRIDGE_SCRIPT.len() + DRAG_REGION_SCRIPT.len());
    source.push_str(HOST_BRIDGE_SCRIPT);
    source.push_str(DRAG_REGION_SCRIPT);
    let script = UserScript::new(
        &source,
        UserContentInjectedFrames::AllFrames,
        UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    manager.add_script(&script);
    let _ = manager.register_script_message_handler(SCRIPT_HANDLER, None);

    let webview = WebView::builder()
        .web_context(&context)
        .user_content_manager(&manager)
        .build();
    if let Some(color) = frame.window.background {
        webview.set_background_color(&to_gdk_rgba(color));
    } else if frame.window.effect != WindowEffect::None {
        webview.set_background_color(&to_gdk_rgba(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }));
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title(&frame.window.title)
        .default_width(safe_dimension(frame.window.width, 1200))
        .default_height(safe_dimension(frame.window.height, 800))
        .resizable(frame.window.resizable)
        .build();
    configure_constraints(&window, &frame.window);
    configure_titlebar(&window, &webview, &frame.window.titlebar);
    if matches!(frame.window.titlebar, TitlebarStyle::Native) {
        window.set_child(Some(&webview));
    }

    let state_store = frame
        .window
        .remember_state
        .then(|| WindowStateStore::for_app_id("com.microsoft.webui.desktop"));
    restore_state(&window, state_store.as_ref());
    install_events(&window, &webview, &manager, &frame, state_store.clone());
    install_wakeup(&window, &webview, &frame);

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
    report_effect_limit(frame.window.effect);
    window.present();
    webview.load_uri(&startup_url());
    let ready = DesktopEvent::Ready;
    dispatch_event(&frame.events, &webview, &ready);
}

fn configure_constraints(window: &ApplicationWindow, options: &webui_desktop::WindowOptions) {
    let min_width = options
        .min_width
        .map_or(-1, |value| safe_dimension(value, -1));
    let min_height = options
        .min_height
        .map_or(-1, |value| safe_dimension(value, -1));
    if min_width >= 0 || min_height >= 0 {
        window.set_size_request(min_width, min_height);
    }
    if options.max_width.is_some() || options.max_height.is_some() {
        eprintln!(
            "WebUI: GTK4 has no portable hard maximum window size; max dimensions are ignored"
        );
    }
}

fn configure_titlebar(window: &ApplicationWindow, webview: &WebView, style: &TitlebarStyle) {
    match style {
        TitlebarStyle::Native => {}
        TitlebarStyle::HiddenInset | TitlebarStyle::Overlay { .. } => {
            window.set_decorated(false);
            let header = HeaderBar::new();
            header.set_show_title_buttons(true);
            if let TitlebarStyle::Overlay { height } = style {
                header.set_size_request(-1, safe_dimension(*height, 28));
            }
            window.set_titlebar(Some(&header));
            window.set_child(Some(webview));
        }
        TitlebarStyle::None => {
            window.set_decorated(false);
            window.set_child(Some(webview));
        }
    }
}

/// UI-thread handles used to run queued window commands.
///
/// `WindowHandle::set_wakeup` requires a `Send + Sync` callback, but GTK
/// objects are neither. The handles therefore stay in thread-local storage
/// owned by the UI thread, and the wakeup callback captures nothing but a
/// request to visit it there.
#[derive(Clone)]
struct CommandContext {
    window: ApplicationWindow,
    webview: WebView,
    handle: WindowHandle,
    events: webui_desktop::EventRegistry,
}

thread_local! {
    static COMMAND_CONTEXT: RefCell<Option<CommandContext>> = const { RefCell::new(None) };
}

fn install_wakeup(window: &ApplicationWindow, webview: &WebView, frame: &DesktopFrame) {
    COMMAND_CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(CommandContext {
            window: window.clone(),
            webview: webview.clone(),
            handle: frame.window_handle.clone(),
            events: frame.events.clone(),
        });
    });
    frame.window_handle.set_wakeup(|| {
        // Captures nothing, so it stays `Send + Sync`. `idle_add_once` hops to
        // the default main context, which the UI thread owns, so the
        // thread-local context below is always the one installed above.
        glib::idle_add_once(|| {
            let context = COMMAND_CONTEXT.with(|slot| slot.borrow().clone());
            if let Some(context) = context {
                execute_commands(
                    &context.window,
                    &context.webview,
                    &context.handle,
                    &context.events,
                );
            }
        });
    });
}

fn execute_commands(
    window: &ApplicationWindow,
    webview: &WebView,
    handle: &webui_desktop::WindowHandle,
    events: &webui_desktop::EventRegistry,
) {
    for command in handle.drain_commands() {
        match command {
            WindowCommand::SetTitle(title) => window.set_title(Some(&title)),
            WindowCommand::SetSize { width, height } => {
                window.set_default_size(safe_dimension(width, 1200), safe_dimension(height, 800))
            }
            WindowCommand::Minimize => minimize_window(window),
            WindowCommand::Maximize => window.maximize(),
            WindowCommand::Unmaximize => window.unmaximize(),
            WindowCommand::SetFullscreen(value) => {
                if value {
                    window.fullscreen();
                } else {
                    window.unfullscreen();
                }
            }
            WindowCommand::Center => {
                eprintln!("WebUI: centering is compositor-controlled on Wayland")
            }
            WindowCommand::Focus => window.present(),
            WindowCommand::Close => window.close(),
            WindowCommand::StartDrag => begin_native_drag(window),
            WindowCommand::SetAlwaysOnTop(value) => {
                if value {
                    eprintln!("WebUI: always_on_top is not portable in GTK4/Wayland");
                }
            }
        }
    }
    let _ = webview;
    let _ = events;
}

fn install_events(
    window: &ApplicationWindow,
    webview: &WebView,
    manager: &UserContentManager,
    frame: &DesktopFrame,
    state_store: Option<WindowStateStore>,
) {
    let events = frame.events.clone();
    let webview_for_close = webview.clone();
    window.connect_close_request(move |_| {
        let event = DesktopEvent::WindowCloseRequested {
            window_id: WINDOW_ID,
        };
        let response = dispatch_event(&events, &webview_for_close, &event);
        if response == EventResponse::PreventDefault {
            glib::Propagation::Stop
        } else {
            let closed = DesktopEvent::WindowClosed {
                window_id: WINDOW_ID,
            };
            dispatch_event(&events, &webview_for_close, &closed);
            glib::Propagation::Proceed
        }
    });

    let events = frame.events.clone();
    let webview_for_size = webview.clone();
    let width_state = state_store.clone();
    window.connect_notify_local(Some("width"), move |window, _| {
        dispatch_size_event(window, &webview_for_size, &events, width_state.as_ref());
    });
    let events = frame.events.clone();
    let webview_for_size = webview.clone();
    let height_state = state_store.clone();
    window.connect_notify_local(Some("height"), move |window, _| {
        dispatch_size_event(window, &webview_for_size, &events, height_state.as_ref());
    });

    connect_window_state_events(window, webview, &frame.events, state_store);
    super::state::connect_toplevel_state_events(window, webview, &frame.events);
    connect_webview_events(webview, &frame.events);
    connect_theme_events(webview, &frame.events);
    connect_message_handler(manager, window, webview, &frame.events);
}

fn connect_window_state_events(
    window: &ApplicationWindow,
    webview: &WebView,
    events: &webui_desktop::EventRegistry,
    state_store: Option<WindowStateStore>,
) {
    // `DesktopEvent::WindowMaximized`/`WindowUnmaximized` and the fullscreen
    // pair are emitted from a single consolidated path in
    // `state::connect_toplevel_state_events`, driven by `gdk::Toplevel`'s
    // `state` property; this handler only persists window state, which the
    // GTK-level `maximized` property notification tracks independently of
    // the GDK toplevel state bits.
    window.connect_maximized_notify(move |window| {
        persist_state(window, state_store.as_ref());
    });
    let events_focus = events.clone();
    let webview_focus = webview.clone();
    window.connect_is_active_notify(move |window| {
        let event = if window.is_active() {
            DesktopEvent::WindowFocused {
                window_id: WINDOW_ID,
            }
        } else {
            DesktopEvent::WindowBlurred {
                window_id: WINDOW_ID,
            }
        };
        dispatch_event(&events_focus, &webview_focus, &event);
    });
    let events_scale = events.clone();
    let webview_scale = webview.clone();
    window.connect_scale_factor_notify(move |window| {
        let event = DesktopEvent::ScaleFactorChanged {
            scale: f64::from(window.scale_factor()),
        };
        dispatch_event(&events_scale, &webview_scale, &event);
    });
}

fn connect_theme_events(webview: &WebView, events: &webui_desktop::EventRegistry) {
    let Some(settings) = gtk4::Settings::default() else {
        return;
    };
    let webview = webview.clone();
    let events = events.clone();
    settings.connect_gtk_application_prefer_dark_theme_notify(move |settings| {
        let event = DesktopEvent::ThemeChanged {
            dark: settings.is_gtk_application_prefer_dark_theme(),
        };
        dispatch_event(&events, &webview, &event);
    });
}

fn connect_webview_events(webview: &WebView, events: &webui_desktop::EventRegistry) {
    let load_events = events.clone();
    webview.connect_load_changed(move |webview, load_event| {
        if load_event == LoadEvent::Finished {
            if let Some(uri) = webview.uri() {
                let event = DesktopEvent::NavigationCompleted {
                    window_id: WINDOW_ID,
                    url: uri.to_string(),
                };
                dispatch_event(&load_events, webview, &event);
            }
        }
    });
    let events = events.clone();
    webview.connect_decide_policy(move |webview, decision, decision_type| {
        if decision_type != PolicyDecisionType::NavigationAction {
            return false;
        }
        let Some(action) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
            decision.ignore();
            return true;
        };
        let Some(request) = action
            .navigation_action()
            .and_then(|action| action.request())
        else {
            decision.ignore();
            return true;
        };
        let Some(uri) = request.uri() else {
            decision.ignore();
            return true;
        };
        let event = DesktopEvent::NavigationRequested {
            window_id: WINDOW_ID,
            url: uri.to_string(),
        };
        let allowed = is_allowed_navigation_url(uri.as_str())
            && dispatch_event(&events, webview, &event) == EventResponse::Continue;
        if allowed {
            decision.use_();
        } else {
            decision.ignore();
        }
        true
    });
}

fn connect_message_handler(
    manager: &UserContentManager,
    window: &ApplicationWindow,
    webview: &WebView,
    events: &webui_desktop::EventRegistry,
) {
    let window = window.clone();
    let webview = webview.clone();
    let events = events.clone();
    manager.connect_script_message_received(Some(SCRIPT_HANDLER), move |_, value| {
        let message = value.to_str();
        let Ok(message) = DesktopHostMessage::from_json(message.as_str()) else {
            return;
        };
        match message {
            DesktopHostMessage::StartDrag => begin_native_drag(&window),
            DesktopHostMessage::Minimize => minimize_window(&window),
            DesktopHostMessage::ToggleMaximize => {
                if window.is_maximized() {
                    window.unmaximize();
                } else {
                    window.maximize();
                }
            }
            DesktopHostMessage::Close => {
                let event = DesktopEvent::WindowCloseRequested {
                    window_id: WINDOW_ID,
                };
                if dispatch_event(&events, &webview, &event) == EventResponse::Continue {
                    window.close();
                }
            }
        }
    });
}

fn begin_native_drag(window: &ApplicationWindow) {
    let Some(surface) = window.surface() else {
        return;
    };
    let display = surface.display();
    let Some(device) = display.default_seat().and_then(|seat| seat.pointer()) else {
        return;
    };
    let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() else {
        return;
    };
    let (x, y, _) =
        surface
            .device_position(&device)
            .unwrap_or((0.0, 0.0, gdk::ModifierType::empty()));
    toplevel.begin_move(&device, 1, x, y, 0);
}

fn minimize_window(window: &ApplicationWindow) {
    if let Some(surface) = window.surface() {
        if let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() {
            let _ = toplevel.minimize();
        }
    }
}

fn dispatch_size_event(
    window: &ApplicationWindow,
    webview: &WebView,
    events: &webui_desktop::EventRegistry,
    state_store: Option<&WindowStateStore>,
) {
    let width = u32::try_from(window.width()).unwrap_or(0);
    let height = u32::try_from(window.height()).unwrap_or(0);
    let event = DesktopEvent::WindowResized {
        window_id: WINDOW_ID,
        width,
        height,
    };
    dispatch_event(events, webview, &event);
    persist_state(window, state_store);
}

pub(super) fn dispatch_event(
    events: &webui_desktop::EventRegistry,
    webview: &WebView,
    event: &DesktopEvent,
) -> EventResponse {
    let response = events.dispatch(event);
    match event.to_javascript() {
        Ok(script) => {
            webview.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {})
        }
        Err(error) => eprintln!("WebUI: failed to mirror desktop event: {error}"),
    }
    response
}

fn restore_state(window: &ApplicationWindow, store: Option<&WindowStateStore>) {
    let Some(store) = store else {
        return;
    };
    let displays = display_bounds(window);
    match store.load_valid(&displays) {
        Ok(Some(state)) => {
            window.set_default_size(
                safe_dimension(state.width, 1200),
                safe_dimension(state.height, 800),
            );
            if state.maximized {
                window.maximize();
            }
            if state.x != 0 || state.y != 0 {
                eprintln!("WebUI: GTK4 cannot restore window position on Wayland");
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("WebUI: failed to restore window state: {error}"),
    }
}

fn persist_state(window: &ApplicationWindow, store: Option<&WindowStateStore>) {
    let Some(store) = store else {
        return;
    };
    let state = WindowState {
        x: 0,
        y: 0,
        width: u32::try_from(window.width()).unwrap_or(0),
        height: u32::try_from(window.height()).unwrap_or(0),
        maximized: window.is_maximized(),
    };
    if let Err(error) = store.save(&state) {
        eprintln!("WebUI: failed to persist window state: {error}");
    }
}

fn display_bounds(window: &ApplicationWindow) -> Vec<DisplayBounds> {
    // Both `RootExt` and `WidgetExt` expose `display()`, so name the trait.
    let display = gtk4::prelude::WidgetExt::display(window);
    let monitors = display.monitors();
    let mut bounds = Vec::with_capacity(monitors.n_items() as usize);
    for index in 0..monitors.n_items() {
        let Some(monitor) = monitors
            .item(index)
            .and_then(|object| object.downcast::<gdk::Monitor>().ok())
        else {
            continue;
        };
        let geometry = monitor.geometry();
        bounds.push(DisplayBounds {
            x: geometry.x(),
            y: geometry.y(),
            width: u32::try_from(geometry.width()).unwrap_or(0),
            height: u32::try_from(geometry.height()).unwrap_or(0),
        });
    }
    bounds
}

fn to_gdk_rgba(color: Rgba) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    )
}

fn report_effect_limit(effect: WindowEffect) {
    if effect != WindowEffect::None {
        eprintln!("WebUI: Linux has no portable blur-behind effect; requested {effect:?} is degraded to the configured background");
    }
}

fn safe_dimension(value: u32, fallback: i32) -> i32 {
    i32::try_from(value).unwrap_or(fallback).max(1)
}

/// Delegate to the shared cross-backend navigation policy.
fn is_allowed_navigation_url(url: &str) -> bool {
    webui_desktop::is_allowed_navigation_url(url, super::APP_ORIGIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_allowlist_rejects_external_origins() {
        assert!(is_allowed_navigation_url("webui://app"));
        assert!(is_allowed_navigation_url("webui://app/settings"));
        assert!(!is_allowed_navigation_url("https://webui://app/"));
        assert!(!is_allowed_navigation_url("webui://application/"));
        assert!(!is_allowed_navigation_url("webui://app.evil/"));
    }

    #[test]
    fn dimensions_are_positive_and_bounded_to_native_int() {
        assert_eq!(safe_dimension(0, 1200), 1);
        assert_eq!(safe_dimension(u32::MAX, 1200), 1200);
        assert_eq!(safe_dimension(640, 1200), 640);
    }
}
