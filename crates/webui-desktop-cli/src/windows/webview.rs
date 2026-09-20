// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebView2 environment, controller, navigation policy, and script bridges.

use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::{Context, Result};
use webui_desktop::{
    DesktopEvent, DesktopHostMessage, EventRegistry, EventResponse, Rgba, WindowEffect,
    DRAG_REGION_SCRIPT,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Controller2, ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
    ICoreWebView2NavigationCompletedEventHandler, ICoreWebView2NavigationStartingEventHandler,
    ICoreWebView2WebMessageReceivedEventArgs, ICoreWebView2_3, COREWEBVIEW2_COLOR,
    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY,
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
};
use windows::core::{Error as WindowsError, Interface, Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE,
};

use super::command::execute_host_message;
use super::message::system_dark;
use super::protocol::read_pwstr;
use super::{APP_HOST, APP_ORIGIN, WINDOW_ID};

/// Byte size of the 32-bit values passed to `DwmSetWindowAttribute`.
const DWM_ATTRIBUTE_SIZE: u32 = 4;

/// Host bridge exposed to web content.
///
/// `DRAG_REGION_SCRIPT` calls `window.webuiHostPostMessage` with an already
/// serialized JSON string. WebView2 wraps that string once more when it is read
/// back through `WebMessageAsJson`, which [`handle_host_message`] unwraps.
const HOST_BRIDGE_SCRIPT: &str = "(()=>{if(!window.chrome?.webview)return;\
window.webuiHostPostMessage=m=>window.chrome.webview.postMessage(String(m));})();";

/// Create the shared WebView2 environment backed by a per-app data folder.
pub(super) fn create_environment() -> Result<ICoreWebView2Environment> {
    let user_data_folder = webview_user_data_folder()?;
    let user_data_folder = user_data_folder.to_string_lossy();
    let user_data_folder = CoTaskMemPWSTR::from(user_data_folder.as_ref());
    let options = webview_environment_options();
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: The folder buffer and options outlive this call, and the
            // handler is owned by the WebView2 loader for the async operation.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    *user_data_folder.as_ref().as_pcwstr(),
                    &options,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(
            move |error_code, environment: Option<ICoreWebView2Environment>| {
                error_code?;
                tx.send(environment.ok_or_else(|| WindowsError::from(E_FAIL)))
                    .map_err(|_| WindowsError::from(E_FAIL))?;
                Ok(())
            },
        ),
    )?;
    rx.recv()
        .map_err(|_| anyhow::anyhow!("WebView2 environment creation was cancelled"))?
        .map_err(Into::into)
}

/// Resolve the first URL loaded into the window.
fn startup_url(use_packaged_index: bool) -> String {
    let default_path = if use_packaged_index {
        "/index.html"
    } else {
        "/"
    };
    let path =
        std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| default_path.to_string());
    startup_url_for_path(&path)
}

/// Build an app-origin URL for the supplied path.
fn startup_url_for_path(path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity(APP_ORIGIN.len() + path.len());
    url.push_str(APP_ORIGIN);
    url.push_str(&path);
    url
}

/// Default WebView2 environment options.
fn webview_environment_options() -> ICoreWebView2EnvironmentOptions {
    let options = CoreWebView2EnvironmentOptions::default();
    options.into()
}

/// Locate (and create) the per-app WebView2 user data folder.
fn webview_user_data_folder() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let app_name = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_os_string()))
        .and_then(|stem| stem.into_string().ok())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "webui-desktop".to_string());
    let dir = base
        .join("Microsoft")
        .join("WebUI")
        .join("Desktop")
        .join(app_name)
        .join("WebView2");
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create WebView2 user data folder {}",
            dir.display()
        )
    })?;
    Ok(dir)
}

/// Create the WebView2 controller hosted inside the native window.
pub(super) fn create_controller(
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
) -> Result<ICoreWebView2Controller> {
    let (tx, rx) = mpsc::channel();
    let environment = environment.clone();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: `hwnd` is a live window owned by this thread and the
            // environment reference stays alive for the async operation.
            unsafe {
                environment
                    .CreateCoreWebView2Controller(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(
            move |error_code, controller: Option<ICoreWebView2Controller>| {
                error_code?;
                tx.send(controller.ok_or_else(|| WindowsError::from(E_FAIL)))
                    .map_err(|_| WindowsError::from(E_FAIL))?;
                Ok(())
            },
        ),
    )?;
    rx.recv()
        .map_err(|_| anyhow::anyhow!("WebView2 controller creation was cancelled"))?
        .map_err(Into::into)
}

/// Apply the developer-tools policy to the WebView2 settings.
pub(super) fn configure_settings(webview: &ICoreWebView2, devtools: bool) -> Result<()> {
    // SAFETY: `webview` is a live COM interface owned by the caller.
    unsafe {
        let settings = webview.Settings()?;
        settings.SetAreDevToolsEnabled(devtools)?;
        settings.SetAreDefaultContextMenusEnabled(devtools)?;
    }
    Ok(())
}

/// Route navigations through the app event registry while keeping the
/// app-origin allowlist as the final, non-overridable authority.
pub(super) fn register_navigation_guard(
    webview: &ICoreWebView2,
    events: EventRegistry,
) -> Result<ICoreWebView2NavigationStartingEventHandler> {
    let webview_for_events = webview.clone();
    let handler = NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            // SAFETY: WebView2 passes a live args interface for the callback's
            // duration; the URL is copied out before the callback returns.
            let uri = read_pwstr(|out| unsafe { args.Uri(out) })?;
            let event = DesktopEvent::NavigationRequested {
                window_id: WINDOW_ID,
                url: uri.clone(),
            };
            let prevented = events.dispatch(&event) == EventResponse::PreventDefault;
            mirror_event(&webview_for_events, &event);
            if prevented || !is_allowed_navigation_url(&uri) {
                // SAFETY: Same live args interface as above.
                unsafe { args.SetCancel(true)? };
            }
        }
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: `webview` is live and the handler is kept alive by the caller,
    // which stores it in the window state for the lifetime of the window.
    unsafe { webview.add_NavigationStarting(&handler, &mut token)? };
    Ok(handler)
}

/// Apply the requested backdrop effect, degrading silently on older Windows.
pub(super) fn configure_window_effect(hwnd: HWND, effect: WindowEffect) {
    // `Vibrancy` and `Tabbed` have no Windows equivalent and degrade to no effect.
    let backdrop = match effect {
        WindowEffect::Acrylic => DWMSBT_TRANSIENTWINDOW.0,
        WindowEffect::Mica => DWMSBT_MAINWINDOW.0,
        WindowEffect::None | WindowEffect::Vibrancy | WindowEffect::Tabbed => return,
    };
    // SAFETY: `hwnd` is a live top-level window and the pointer refers to an
    // initialized `i32` whose length matches `DWM_ATTRIBUTE_SIZE`. DWM reports
    // an error on Windows versions without backdrop support, which is ignored
    // so the window degrades to an opaque frame instead of failing to start.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            std::ptr::from_ref(&backdrop).cast(),
            DWM_ATTRIBUTE_SIZE,
        )
    };
    let dark = i32::from(system_dark());
    // SAFETY: Same live window; the pointer refers to an initialized `i32`
    // holding a `BOOL`-shaped value of the declared length.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            DWM_ATTRIBUTE_SIZE,
        )
    };
}

/// Paint the WebView2 surface with the manifest background so the window does
/// not flash white before the first frame is rendered.
pub(super) fn configure_controller_background(
    controller: &ICoreWebView2Controller,
    background: Option<Rgba>,
) -> Result<()> {
    let Some(color) = background else {
        return Ok(());
    };
    // `ICoreWebView2Controller2` is unavailable on very old runtimes; a failed
    // cast simply leaves the default background in place.
    let Ok(controller) = controller.cast::<ICoreWebView2Controller2>() else {
        return Ok(());
    };
    let color = COREWEBVIEW2_COLOR {
        A: color.a,
        R: color.r,
        G: color.g,
        B: color.b,
    };
    // SAFETY: The controller is a live COM interface and `color` is passed by
    // value as an initialized plain-old-data struct.
    unsafe {
        controller.SetDefaultBackgroundColor(color)?;
    }
    Ok(())
}

/// Install the host bridge and drag-region helper script on every document.
pub(super) fn inject_drag_script(webview: &ICoreWebView2) -> Result<()> {
    add_document_script(webview, HOST_BRIDGE_SCRIPT)?;
    add_document_script(webview, DRAG_REGION_SCRIPT)
}

/// Register a script that runs before any page script on every document.
fn add_document_script(webview: &ICoreWebView2, source: &str) -> Result<()> {
    let webview = webview.clone();
    let script = CoTaskMemPWSTR::from(source);
    AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: `webview` is live and the script buffer outlives the call.
            unsafe {
                webview
                    .AddScriptToExecuteOnDocumentCreated(*script.as_ref().as_pcwstr(), &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(|result, _| {
            result?;
            Ok(())
        }),
    )?;
    Ok(())
}

/// Mirror a native event into web content as a DOM event.
pub(super) fn mirror_event(webview: &ICoreWebView2, event: &DesktopEvent) {
    let Ok(script) = event.to_javascript() else {
        return;
    };
    let script = CoTaskMemPWSTR::from(script.as_str());
    let handler = ExecuteScriptCompletedHandler::create(Box::new(|_result, _| Ok(())));
    // SAFETY: `webview` is a live COM interface and the script buffer stays
    // alive for the duration of this call. The completion handler is reference
    // counted by WebView2, so it remains valid until the script finishes.
    let _ = unsafe { webview.ExecuteScript(*script.as_ref().as_pcwstr(), &handler) };
}

/// Handle a window-control message posted by web content.
///
/// Returns `true` when the message was a host command and must not be forwarded
/// to the fetch bridge.
pub(super) fn handle_host_message(
    hwnd: HWND,
    args: &ICoreWebView2WebMessageReceivedEventArgs,
) -> WindowsResult<bool> {
    // SAFETY: WebView2 passes a live args interface for the callback's
    // duration; the JSON is copied out before the callback returns.
    let raw = read_pwstr(|out| unsafe { args.WebMessageAsJson(out) })?;
    let Some(message) = decode_host_message(&raw) else {
        return Ok(false);
    };
    // Host commands arrive on the UI thread, so they run directly instead of
    // being queued through the command channel.
    execute_host_message(hwnd, message);
    Ok(true)
}

/// Decode a `WebMessageAsJson` payload into a host command.
///
/// Host commands are posted as strings, so WebView2 reports them as a JSON
/// string wrapping the message's own JSON text. Fetch-bridge messages are
/// objects and fail the first decode, which hands them back to the bridge.
fn decode_host_message(raw: &str) -> Option<DesktopHostMessage> {
    let payload = serde_json::from_str::<String>(raw).ok()?;
    DesktopHostMessage::from_json(&payload).ok()
}

/// Report completed navigations to app handlers and web content.
pub(super) fn register_navigation_completed(
    webview: &ICoreWebView2,
    events: EventRegistry,
) -> Result<ICoreWebView2NavigationCompletedEventHandler> {
    let webview_for_uri = webview.clone();
    let handler = NavigationCompletedEventHandler::create(Box::new(move |_sender, _args| {
        // SAFETY: The cloned interface is live for as long as the handler is
        // registered, and the source URL is copied out immediately.
        let url = read_pwstr(|out| unsafe { webview_for_uri.Source(out) })?;
        let event = DesktopEvent::NavigationCompleted {
            window_id: WINDOW_ID,
            url,
        };
        events.dispatch(&event);
        mirror_event(&webview_for_uri, &event);
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: `webview` is live and the handler is kept alive by the caller.
    unsafe { webview.add_NavigationCompleted(&handler, &mut token)? };
    Ok(handler)
}

/// Report whether a URL belongs to the app origin allowlist.
/// Delegate to the shared cross-backend navigation policy.
fn is_allowed_navigation_url(url: &str) -> bool {
    webui_desktop::is_allowed_navigation_url(url, APP_ORIGIN)
}

/// Map packaged static assets onto the app host when they exist.
pub(super) fn register_virtual_host_assets(webview: &ICoreWebView2) -> Result<bool> {
    let Some(resources) = crate::find_packaged_resources_dir() else {
        return Ok(false);
    };
    let assets = resources.join("assets");
    if !assets.is_dir() {
        return Ok(false);
    }

    let webview: ICoreWebView2_3 = webview.cast()?;
    let host = CoTaskMemPWSTR::from(APP_HOST);
    let assets = assets.to_string_lossy();
    let assets = CoTaskMemPWSTR::from(assets.as_ref());
    // SAFETY: `webview` is a live COM interface and both string buffers outlive
    // this call.
    unsafe {
        webview.SetVirtualHostNameToFolderMapping(
            *host.as_ref().as_pcwstr(),
            *assets.as_ref().as_pcwstr(),
            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY,
        )?;
    }
    Ok(true)
}

/// Navigate the window to its configured startup URL.
pub(super) fn navigate_to_startup_url(
    webview: &ICoreWebView2,
    use_packaged_index: bool,
) -> Result<()> {
    let url = startup_url(use_packaged_index);
    let url = CoTaskMemPWSTR::from(url.as_str());
    // SAFETY: `webview` is live and the URL buffer outlives this call.
    unsafe { webview.Navigate(*url.as_ref().as_pcwstr())? };
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn startup_url_normalizes_paths() {
        assert_eq!(
            startup_url_for_path("/contacts"),
            "https://app.webui.localhost/contacts"
        );
        assert_eq!(
            startup_url_for_path("contacts"),
            "https://app.webui.localhost/contacts"
        );
    }

    #[test]
    fn navigation_allowlist_accepts_only_the_app_origin() {
        assert!(is_allowed_navigation_url("about:blank"));
        assert!(is_allowed_navigation_url("https://app.webui.localhost"));
        assert!(is_allowed_navigation_url(
            "https://app.webui.localhost/contacts?view=all"
        ));
        assert!(!is_allowed_navigation_url("https://example.com"));
        assert!(!is_allowed_navigation_url(
            "https://app.webui.localhost.evil.com/"
        ));
    }

    #[test]
    fn host_messages_decode_through_the_webview_json_wrapper() {
        // `webuiHostPostMessage` posts the already-serialized JSON text, which
        // WebView2 reports wrapped in one more layer of JSON string quoting.
        assert_eq!(
            decode_host_message("\"\\\"start-drag\\\"\""),
            Some(DesktopHostMessage::StartDrag)
        );
        assert_eq!(
            decode_host_message("\"\\\"toggle-maximize\\\"\""),
            Some(DesktopHostMessage::ToggleMaximize)
        );
        // Fetch-bridge payloads are objects and must fall through to the bridge.
        assert_eq!(
            decode_host_message("{\"kind\":\"webui-desktop-fetch\"}"),
            None
        );
        assert_eq!(decode_host_message("\"\\\"nope\\\"\""), None);
    }

    #[test]
    fn host_bridge_script_defines_the_documented_global() {
        assert!(HOST_BRIDGE_SCRIPT.contains("window.webuiHostPostMessage"));
        assert!(DRAG_REGION_SCRIPT.contains("window.webuiHostPostMessage"));
    }
}
