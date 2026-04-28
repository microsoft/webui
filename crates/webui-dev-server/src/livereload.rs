// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Server-Sent Events live-reload broadcaster.
//!
//! [`LiveReload`] holds a `tokio::sync::broadcast` channel and an actix
//! handler that streams events to connected browsers. Cheap to clone — the
//! underlying `Sender` is itself an `Arc`-like type.
//!
//! ## Wire protocol
//!
//! The browser opens an `EventSource` connection and listens for two
//! named events:
//!
//! - `reload` — successful rebuild; the page should refresh.
//! - `reload-error` — rebuild failed; the page logs the message and waits
//!   for the next event without reloading.
//!
//! A 30-second `:heartbeat` comment keeps the connection alive through
//! intermediate proxies that drop idle TCP streams.

use std::time::Duration;

use actix_web::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use actix_web::web::Bytes;
use actix_web::{web, HttpResponse};
use futures_util::stream::Stream;
use tokio::sync::broadcast;

use crate::inject::inject_before_body_close;

/// Broadcast channel capacity. Live reload only emits one event per build
/// — 16 is plenty for a dev server, and lagging subscribers drop old
/// events rather than delaying the publisher.
const RELOAD_CHANNEL_CAPACITY: usize = 16;

/// Heartbeat interval for SSE streams so intermediaries don't drop idle
/// connections.
const SSE_HEARTBEAT: Duration = Duration::from_secs(30);

/// A live-reload event. Keep this enum stable; both the wire format and
/// the broadcast subscribers depend on it.
#[derive(Clone, Debug)]
pub enum ReloadEvent {
    /// Successful rebuild — clients should reload.
    Reload,
    /// Build failed — clients log the message but do not reload.
    Error(String),
}

/// SSE live-reload broadcaster. Cheap to clone; share across handlers.
#[derive(Clone)]
pub struct LiveReload {
    endpoint: String,
    tx: broadcast::Sender<ReloadEvent>,
    client_script: String,
}

impl LiveReload {
    /// Create a new broadcaster.
    ///
    /// `endpoint` should be a root-relative URL path (e.g.
    /// `"/__webui/livereload"`). Use a root-relative path so the script
    /// works when the served pages set `<base href>` for sub-path
    /// deployments.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        let (tx, _rx) = broadcast::channel::<ReloadEvent>(RELOAD_CHANNEL_CAPACITY);
        let client_script = build_client_script(&endpoint);
        Self {
            endpoint,
            tx,
            client_script,
        }
    }

    /// The endpoint URL where the SSE stream is served.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The inline `<script>...</script>` that subscribes to this endpoint.
    /// Inject this into served HTML.
    #[must_use]
    pub fn client_script(&self) -> &str {
        &self.client_script
    }

    /// Inject [`Self::client_script`] immediately before `</body>` in
    /// `html`. Appends to the end if no closing tag is found.
    #[must_use]
    pub fn inject(&self, html: &str) -> String {
        inject_before_body_close(html, &self.client_script)
    }

    /// Broadcast a `Reload` event to all connected clients. A failed send
    /// (no subscribers) is ignored — that's a normal state when no browser
    /// is currently connected.
    pub fn broadcast_reload(&self) {
        let _ = self.tx.send(ReloadEvent::Reload);
    }

    /// Broadcast a `reload-error` event. Newlines in the message are
    /// replaced with spaces to keep the SSE frame on one line.
    pub fn broadcast_error(&self, msg: impl Into<String>) {
        let _ = self.tx.send(ReloadEvent::Error(msg.into()));
    }

    /// Direct access to the underlying broadcaster, e.g. to count
    /// subscribers (`sender().receiver_count()`).
    #[must_use]
    pub fn sender(&self) -> &broadcast::Sender<ReloadEvent> {
        &self.tx
    }
}

/// Actix handler for the SSE endpoint.
///
/// Wire it up in your `App` with:
///
/// ```ignore
/// use actix_web::{web, App};
/// use webui_dev_server::{livereload, LiveReload};
///
/// let lr = LiveReload::new("/__webui/livereload");
/// let app = App::new()
///     .app_data(web::Data::new(lr.clone()))
///     .route(lr.endpoint(), web::get().to(livereload::sse_handler));
/// ```
pub async fn sse_handler(lr: web::Data<LiveReload>) -> HttpResponse {
    let rx = lr.tx.subscribe();
    HttpResponse::Ok()
        .insert_header((CONTENT_TYPE, "text/event-stream"))
        .insert_header((CACHE_CONTROL, "no-cache"))
        // Disable nginx-style proxy buffering if anyone proxies dev.
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(sse_stream(rx))
}

fn sse_stream(
    mut rx: broadcast::Receiver<ReloadEvent>,
) -> impl Stream<Item = std::result::Result<Bytes, actix_web::Error>> {
    async_stream::stream! {
        // Hello frame so EventSource immediately marks the connection open.
        yield Ok::<_, actix_web::Error>(Bytes::from_static(b": connected\n\n"));

        let mut heartbeat = tokio::time::interval(SSE_HEARTBEAT);
        heartbeat.tick().await; // skip immediate fire

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    yield Ok(Bytes::from_static(b": heartbeat\n\n"));
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(ReloadEvent::Reload) => {
                            yield Ok(Bytes::from_static(b"event: reload\ndata: ok\n\n"));
                        }
                        Ok(ReloadEvent::Error(e)) => {
                            let payload = format!(
                                "event: reload-error\ndata: {}\n\n",
                                e.replace('\n', " ")
                            );
                            yield Ok(Bytes::from(payload));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    }
}

fn build_client_script(endpoint: &str) -> String {
    // `EventSource` reconnects automatically; we only need to handle
    // named events. The endpoint is embedded as a JSON-quoted string so
    // any unusual characters survive (we don't expect them, but it's
    // cheap insurance).
    let endpoint_js = json_string_literal(endpoint);
    format!(
        "<script>(function(){{try{{var s=new EventSource({endpoint_js});\
s.addEventListener(\"reload\",function(){{location.reload();}});\
s.addEventListener(\"reload-error\",function(e){{console.error(\"[webui-dev] rebuild failed:\",e.data);}});\
}}catch(e){{console.warn(\"[webui-dev] live reload unavailable:\",e);}}}})();</script>"
    )
}

/// Minimal JSON string literal escaper for embedding a URL path in a
/// JavaScript string. Handles the only chars the URL grammar permits to
/// also be JSON-significant: `"` and `\`.
fn json_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(&mut out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn live_reload_endpoint_round_trips() {
        let lr = LiveReload::new("/__webui/livereload");
        assert_eq!(lr.endpoint(), "/__webui/livereload");
    }

    #[test]
    fn client_script_includes_endpoint() {
        let lr = LiveReload::new("/__webui/livereload");
        assert!(lr.client_script().contains("/__webui/livereload"));
        assert!(lr.client_script().contains("EventSource"));
        assert!(lr.client_script().contains("reload"));
    }

    #[test]
    fn inject_places_script_before_close_body() {
        let lr = LiveReload::new("/__webui/lr");
        let html = "<html><body>x</body></html>";
        let injected = lr.inject(html);
        let script_idx = injected.find("EventSource").unwrap();
        let close_idx = injected.find("</body>").unwrap();
        assert!(script_idx < close_idx);
    }

    #[test]
    fn broadcast_reload_does_not_panic_with_no_subscribers() {
        let lr = LiveReload::new("/x");
        lr.broadcast_reload();
        lr.broadcast_error("oops");
    }

    #[test]
    fn broadcast_reload_reaches_subscriber() {
        let lr = LiveReload::new("/x");
        let mut rx = lr.sender().subscribe();
        lr.broadcast_reload();
        let evt = rx.try_recv().unwrap();
        assert!(matches!(evt, ReloadEvent::Reload));
    }

    #[test]
    fn broadcast_error_carries_message() {
        let lr = LiveReload::new("/x");
        let mut rx = lr.sender().subscribe();
        lr.broadcast_error("build failed");
        let evt = rx.try_recv().unwrap();
        match evt {
            ReloadEvent::Error(msg) => assert_eq!(msg, "build failed"),
            _ => panic!("expected Error event"),
        }
    }

    #[test]
    fn json_string_literal_escapes_quote_and_backslash() {
        assert_eq!(json_string_literal("/foo"), "\"/foo\"");
        assert_eq!(json_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string_literal("a\\b"), "\"a\\\\b\"");
    }
}
