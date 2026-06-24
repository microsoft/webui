// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::Arc;

use anyhow::{Context, Result};
use gtk4::{gio, glib, prelude::*, Application, ApplicationWindow};
use webkit6::{prelude::*, URISchemeRequest, URISchemeResponse, WebContext, WebView};
use webui_desktop::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_ASSET_BYTES,
};

/// Run a packaged WebUI desktop app on Linux using GTK4 and WebKitGTK 6.
///
/// # Errors
///
/// Returns an error when packaged resources cannot be located or GTK cannot run.
pub fn run_packaged_app() -> Result<()> {
    let resources = packaged_resources_dir()?;
    let manifest =
        webui_desktop::DesktopBundleManifest::load(&resources.join("manifest.webui-desktop.json"))
            .with_context(|| "Failed to read packaged desktop manifest")?;
    let runtime = Arc::new(DesktopRuntime::from_bundle(resources)?);
    run_runtime(runtime, manifest.window)
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
    let app = Application::builder()
        .application_id("com.microsoft.webui.desktop")
        .build();
    app.connect_activate(move |app| {
        let context = WebContext::new();
        let handler_runtime = Arc::clone(&runtime);
        context.register_uri_scheme("webui", move |request| {
            handle_scheme_request(request, &handler_runtime);
        });

        let webview = WebView::builder().web_context(&context).build();
        let window_widget = ApplicationWindow::builder()
            .application(app)
            .title(&window.title)
            .default_width(i32::try_from(window.width).unwrap_or(1200))
            .default_height(i32::try_from(window.height).unwrap_or(800))
            .child(&webview)
            .build();
        if window.maximized {
            window_widget.maximize();
        }
        window_widget.present();
        webview.load_uri(&startup_url());
    });
    app.run();
    Ok(())
}

fn packaged_resources_dir() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("Failed to locate webui-desktop executable")?;
    let root = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to locate desktop executable directory"))?;
    Ok(root.join("resources").join("webui"))
}

fn startup_url() -> String {
    let path = std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| "/".to_string());
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity("webui://app".len() + path.len());
    url.push_str("webui://app");
    url.push_str(&path);
    url
}

fn handle_scheme_request(request: &URISchemeRequest, runtime: &DesktopRuntime) {
    let path = request
        .path()
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/".to_string());
    let method = request
        .http_method()
        .map(|value| DesktopHttpMethod::parse(value.as_str()))
        .unwrap_or(DesktopHttpMethod::Get);
    let wants_json = request
        .http_headers()
        .and_then(|headers| headers.one("Accept"))
        .is_some_and(|accept| {
            let accept = accept.as_str();
            accept.contains("json") || accept.contains("ndjson")
        });
    let body = match read_body(request) {
        Ok(body) => body,
        Err(response) => {
            finish_scheme_request(request, response);
            return;
        }
    };
    let desktop_request = DesktopProtocolRequest {
        method,
        path: &path,
        body: &body,
        wants_json,
    };
    let response = runtime
        .handle_request(&desktop_request)
        .unwrap_or_else(|err| DesktopProtocolResponse::text(500, err.chain_message()));
    finish_scheme_request(request, response);
}

fn read_body(request: &URISchemeRequest) -> std::result::Result<Vec<u8>, DesktopProtocolResponse> {
    let Some(stream) = request.http_body() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let max = usize::try_from(DEFAULT_MAX_ASSET_BYTES).unwrap_or(usize::MAX);
    loop {
        match stream.read_bytes(16 * 1024, None::<&gio::Cancellable>) {
            Ok(bytes) if bytes.is_empty() => break,
            Ok(bytes) => {
                if out.len().saturating_add(bytes.len()) > max {
                    return Err(DesktopProtocolResponse::text(
                        413,
                        "desktop request body exceeds the configured size limit",
                    ));
                }
                out.extend_from_slice(bytes.as_ref());
            }
            Err(_) => break,
        }
    }
    Ok(out)
}

fn finish_scheme_request(request: &URISchemeRequest, response: DesktopProtocolResponse) {
    let bytes = glib::Bytes::from_owned(response.body);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let length = i64::try_from(bytes.len()).unwrap_or(-1);
    let scheme_response = URISchemeResponse::new(&stream, length);
    scheme_response.set_content_type(&response.content_type);
    scheme_response.set_status(u32::from(response.status), None);
    request.finish_with_response(&scheme_response);
}
