// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::response::finish_scheme_request;
use crate::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopRuntime,
    DEFAULT_MAX_REQUEST_BYTES,
};
use gtk4::gio;
use std::sync::Arc;
use webkit6::{prelude::*, URISchemeRequest};

pub(super) fn startup_url() -> String {
    let path = std::env::var("WEBUI_DESKTOP_START_PATH").unwrap_or_else(|_| "/".to_string());
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let mut url = String::with_capacity(super::APP_ORIGIN.len() + path.len());
    url.push_str(super::APP_ORIGIN);
    url.push_str(&path);
    url
}

pub(super) fn handle_scheme_request(
    request: &URISchemeRequest,
    runtime: &Arc<DesktopRuntime>,
    executor: &Arc<crate::execution::ApplicationExecutor>,
) {
    if request
        .uri()
        .is_none_or(|uri| !crate::is_allowed_navigation_url(uri.as_str(), super::APP_ORIGIN))
    {
        finish_scheme_request(
            request,
            DesktopProtocolResponse::text(403, "Untrusted desktop origin"),
        );
        return;
    }
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
            accept.as_str().contains("json") || accept.as_str().contains("ndjson")
        });
    let body = match read_body(request) {
        Ok(body) => body,
        Err(response) => {
            finish_scheme_request(request, response);
            return;
        }
    };
    let runtime = Arc::clone(runtime);
    let work = executor.submit(move || {
        runtime
            .handle_request(&DesktopProtocolRequest {
                method,
                path: &path,
                body: &body,
                wants_json,
            })
            .unwrap_or_else(|error| DesktopProtocolResponse::text(500, error.chain_message()))
    });
    let Ok(work) = work else {
        finish_scheme_request(
            request,
            DesktopProtocolResponse::text(503, "Desktop application executor is unavailable"),
        );
        return;
    };
    let request = request.clone();
    gtk4::glib::MainContext::default().spawn_local(async move {
        let response = work.await.unwrap_or_else(|_| {
            DesktopProtocolResponse::text(503, "Desktop application executor closed")
        });
        finish_scheme_request(&request, response);
    });
}

fn read_body(request: &URISchemeRequest) -> std::result::Result<Vec<u8>, DesktopProtocolResponse> {
    let Some(stream) = request.http_body() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let max = usize::try_from(DEFAULT_MAX_REQUEST_BYTES).unwrap_or(usize::MAX);
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
            Err(_) => {
                return Err(DesktopProtocolResponse::text(
                    400,
                    "desktop request body could not be read",
                ))
            }
        }
    }
    Ok(out)
}
