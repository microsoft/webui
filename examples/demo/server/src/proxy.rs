// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{
    body::{BodySize, BodyStream, MessageBody},
    http::{
        header::{self, HeaderMap, HeaderName},
        StatusCode,
    },
    web, HttpRequest, HttpResponse, HttpResponseBuilder,
};
use awc::Client;
use std::{
    collections::HashMap,
    error::Error as StdError,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

// Maximum response body size (16 MB) — generous for HTML/JSON/assets.
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

struct LimitedBody<B> {
    body: B,
    bytes_forwarded: usize,
    limit: usize,
    finished: bool,
}

impl<B> LimitedBody<B> {
    fn new(body: B, limit: usize) -> Self {
        Self {
            body,
            bytes_forwarded: 0,
            limit,
            finished: false,
        }
    }
}

impl<B> MessageBody for LimitedBody<B>
where
    B: MessageBody + Unpin,
{
    type Error = Box<dyn StdError>;

    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<web::Bytes, Self::Error>>> {
        if self.finished {
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.body).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => match self.bytes_forwarded.checked_add(chunk.len()) {
                Some(total) if total <= self.limit => {
                    self.bytes_forwarded = total;
                    Poll::Ready(Some(Ok(chunk)))
                }
                _ => {
                    self.finished = true;
                    Poll::Ready(Some(Err(Box::new(ResponseBodyTooLarge {
                        limit: self.limit,
                    }))))
                }
            },
            Poll::Ready(Some(Err(error))) => {
                self.finished = true;
                Poll::Ready(Some(Err(error.into())))
            }
            Poll::Ready(None) => {
                self.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
struct ResponseBodyTooLarge {
    limit: usize,
}

impl fmt::Display for ResponseBodyTooLarge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "proxied response body exceeded {} byte limit",
            self.limit
        )
    }
}

impl StdError for ResponseBodyTooLarge {}

/// Shared state mapping app slugs to their internal ports.
pub(crate) struct ProxyState {
    pub routes: HashMap<String, u16>,
}

/// Reverse proxy handler: strips the `/{slug}/` prefix and forwards
/// the request to the app's internal port.
pub(crate) async fn proxy_handler(
    req: HttpRequest,
    body: web::Bytes,
    path: web::Path<(String, String)>,
    state: web::Data<ProxyState>,
    client: web::Data<Client>,
) -> HttpResponse {
    let (slug, tail) = path.into_inner();

    let port = match state.routes.get(&slug) {
        Some(p) => *p,
        None => return HttpResponse::NotFound().body(format!("Unknown app: {slug}")),
    };

    let forward_path = if tail.is_empty() {
        "/".to_string()
    } else {
        format!("/{tail}")
    };

    // Preserve query string
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    let target_url = format!("http://127.0.0.1:{port}{forward_path}{query}");

    // Build the forwarded request
    // Preserve upstream content codings byte-for-byte. The proxy streams the
    // encoded body and matching Content-Encoding header instead of risking a
    // decoder pass-through for an unsupported or stacked encoding.
    let mut forwarded = client
        .request(req.method().clone(), &target_url)
        .no_decompress();

    // Copy relevant headers (skip Host — we set our own)
    for (name, value) in req.headers() {
        if name != "host" {
            forwarded = forwarded.insert_header((name.clone(), value.clone()));
        }
    }

    // Add forwarding headers
    if let Some(peer) = req.peer_addr() {
        forwarded = forwarded.insert_header(("X-Forwarded-For", peer.ip().to_string()));
    }
    if let Some(host) = req.headers().get("host") {
        forwarded =
            forwarded.insert_header(("X-Forwarded-Host", host.to_str().unwrap_or_default()));
    }
    forwarded = forwarded.insert_header(("X-Forwarded-Proto", req.connection_info().scheme()));

    // Send the request
    let response = match forwarded.send_body(body).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Proxy error for {slug} → {target_url}: {e}");
            return HttpResponse::BadGateway().body(format!("App '{slug}' is unavailable: {e}"));
        }
    };

    if known_content_length_exceeds(response.headers(), MAX_BODY_SIZE) {
        log::error!(
            "Proxy response for {slug} exceeds the {MAX_BODY_SIZE} byte limit declared by Content-Length"
        );
        return HttpResponse::BadGateway().body(format!(
            "App '{slug}' returned a response larger than the {MAX_BODY_SIZE} byte proxy limit"
        ));
    }

    // Build the response back to the client.
    let status = response.status();
    let mut builder = proxy_response_builder(status, response.headers());
    builder.body(LimitedBody::new(BodyStream::new(response), MAX_BODY_SIZE))
}

fn known_content_length_exceeds(headers: &HeaderMap, limit: usize) -> bool {
    let Some(length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };

    match usize::try_from(length) {
        Ok(length) => length > limit,
        Err(_) => true,
    }
}

fn proxy_response_builder(status: StatusCode, headers: &HeaderMap) -> HttpResponseBuilder {
    let mut builder = HttpResponse::build(status);

    for (name, value) in headers {
        // Content-Length is unsafe because the streaming byte limit can
        // terminate delivery early. Content-Encoding remains valid because
        // the upstream request explicitly disables AWC decompression.
        if should_forward_response_header(name, headers) {
            builder.insert_header((name.clone(), value.clone()));
        }
    }

    // Set SAMEORIGIN for all proxied responses.
    builder.insert_header(("X-Frame-Options", "SAMEORIGIN"));
    builder
}

fn should_forward_response_header(name: &HeaderName, headers: &HeaderMap) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "content-length"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    ) && !headers
        .get_all(header::CONNECTION)
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|connection_header| connection_header.trim().eq_ignore_ascii_case(name.as_str()))
}

/// Handler for the root of an app slug (e.g., `/calculator` without trailing slash).
/// Redirects to `/{slug}/` so relative paths resolve correctly.
pub(crate) async fn slug_redirect(
    path: web::Path<String>,
    state: web::Data<ProxyState>,
) -> HttpResponse {
    let slug = path.into_inner();
    if state.routes.contains_key(&slug) {
        HttpResponse::MovedPermanently()
            .insert_header(("Location", format!("/{slug}/")))
            .finish()
    } else {
        HttpResponse::NotFound().body(format!("Unknown app: {slug}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::HeaderValue;
    use std::{cell::Cell, collections::VecDeque, future::poll_fn, rc::Rc};

    fn proxy_response<B>(
        status: StatusCode,
        headers: &HeaderMap,
        body: B,
        max_body_size: usize,
    ) -> HttpResponse
    where
        B: MessageBody + Unpin + 'static,
    {
        proxy_response_builder(status, headers).body(LimitedBody::new(body, max_body_size))
    }

    #[derive(Debug)]
    struct TestBodyError(&'static str);

    impl fmt::Display for TestBodyError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl StdError for TestBodyError {}

    struct TestBody {
        chunks: VecDeque<Result<web::Bytes, TestBodyError>>,
        polls: Rc<Cell<usize>>,
    }

    impl TestBody {
        fn new(
            chunks: impl IntoIterator<Item = Result<web::Bytes, TestBodyError>>,
            polls: Rc<Cell<usize>>,
        ) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
                polls,
            }
        }
    }

    impl MessageBody for TestBody {
        type Error = TestBodyError;

        fn size(&self) -> BodySize {
            BodySize::Stream
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<web::Bytes, Self::Error>>> {
            self.polls.set(self.polls.get() + 1);
            Poll::Ready(self.chunks.pop_front())
        }
    }

    struct GatedBody {
        first: Option<web::Bytes>,
        second: Option<web::Bytes>,
        can_finish: Rc<Cell<bool>>,
        finished: Rc<Cell<bool>>,
        polls: Rc<Cell<usize>>,
    }

    impl MessageBody for GatedBody {
        type Error = TestBodyError;

        fn size(&self) -> BodySize {
            BodySize::Stream
        }

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<web::Bytes, Self::Error>>> {
            self.polls.set(self.polls.get() + 1);

            if let Some(first) = self.first.take() {
                return Poll::Ready(Some(Ok(first)));
            }

            if !self.can_finish.get() {
                return Poll::Pending;
            }

            if let Some(second) = self.second.take() {
                return Poll::Ready(Some(Ok(second)));
            }

            self.finished.set(true);
            Poll::Ready(None)
        }
    }

    async fn next_chunk<B: MessageBody + Unpin>(
        body: &mut B,
    ) -> Option<Result<web::Bytes, B::Error>> {
        poll_fn(|cx| Pin::new(&mut *body).poll_next(cx)).await
    }

    #[actix_web::test]
    async fn forwards_multiple_chunks_before_upstream_finishes() {
        let can_finish = Rc::new(Cell::new(false));
        let finished = Rc::new(Cell::new(false));
        let polls = Rc::new(Cell::new(0));
        let body = GatedBody {
            first: Some(web::Bytes::from_static(b"shell")),
            second: Some(web::Bytes::from_static(b"hydration")),
            can_finish: can_finish.clone(),
            finished: finished.clone(),
            polls: polls.clone(),
        };
        let response = proxy_response(StatusCode::OK, &HeaderMap::new(), body, 1024);
        let mut body = response.into_body();

        let first = next_chunk(&mut body).await.unwrap().unwrap();
        assert_eq!(first, web::Bytes::from_static(b"shell"));
        assert!(!finished.get(), "upstream unexpectedly finished first");
        assert_eq!(polls.get(), 1, "upstream was polled without demand");

        can_finish.set(true);
        let second = next_chunk(&mut body).await.unwrap().unwrap();
        assert_eq!(second, web::Bytes::from_static(b"hydration"));
        assert!(next_chunk(&mut body).await.is_none());
        assert!(finished.get());
    }

    #[actix_web::test]
    async fn cumulative_limit_breach_terminates_stream() {
        let polls = Rc::new(Cell::new(0));
        let body = TestBody::new(
            [
                Ok(web::Bytes::from_static(b"1234")),
                Ok(web::Bytes::from_static(b"567")),
                Ok(web::Bytes::from_static(b"must not be polled")),
            ],
            polls.clone(),
        );
        let mut body = LimitedBody::new(body, 6);

        assert_eq!(
            next_chunk(&mut body).await.unwrap().unwrap(),
            web::Bytes::from_static(b"1234")
        );
        let error = next_chunk(&mut body).await.unwrap().unwrap_err();
        assert_eq!(
            error.to_string(),
            "proxied response body exceeded 6 byte limit"
        );
        assert!(next_chunk(&mut body).await.is_none());
        assert_eq!(polls.get(), 2);
    }

    #[actix_web::test]
    async fn upstream_chunk_error_terminates_stream() {
        let polls = Rc::new(Cell::new(0));
        let body = TestBody::new(
            [
                Ok(web::Bytes::from_static(b"first")),
                Err(TestBodyError("upstream chunk failed")),
                Ok(web::Bytes::from_static(b"must not be polled")),
            ],
            polls.clone(),
        );
        let mut body = LimitedBody::new(body, 1024);

        assert!(next_chunk(&mut body).await.unwrap().is_ok());
        let error = next_chunk(&mut body).await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), "upstream chunk failed");
        assert!(next_chunk(&mut body).await.is_none());
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn preserves_status_and_safe_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            HeaderName::from_static("x-demo-header"),
            HeaderValue::from_static("preserved"),
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("dcb"));
        let body = TestBody::new([], Rc::new(Cell::new(0)));

        let response = proxy_response(StatusCode::PARTIAL_CONTENT, &headers, body, 1024);

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(
            response.headers().get("x-demo-header").unwrap(),
            "preserved"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_ENCODING).unwrap(),
            "dcb"
        );
        assert_eq!(
            response.headers().get(header::X_FRAME_OPTIONS).unwrap(),
            "SAMEORIGIN"
        );
    }

    #[test]
    fn strips_hop_by_hop_and_body_framing_headers() {
        let mut headers = HeaderMap::new();
        for name in [
            "connection",
            "content-length",
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
        ] {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static("forbidden"),
            );
        }
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-connection-only"),
        );
        headers.insert(
            HeaderName::from_static("x-connection-only"),
            HeaderValue::from_static("forbidden"),
        );
        let body = TestBody::new([], Rc::new(Cell::new(0)));

        let response = proxy_response(StatusCode::OK, &headers, body, 1024);

        for name in [
            "connection",
            "content-length",
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "x-connection-only",
        ] {
            assert!(
                !response.headers().contains_key(name),
                "{name} was forwarded"
            );
        }
    }

    #[test]
    fn rejects_only_known_oversized_content_lengths_early() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1025"));
        assert!(known_content_length_exceeds(&headers, 1024));

        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1024"));
        assert!(!known_content_length_exceeds(&headers, 1024));

        // Unknown or malformed lengths still use the streaming limiter.
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("invalid"));
        assert!(!known_content_length_exceeds(&headers, 1024));
        headers.remove(header::CONTENT_LENGTH);
        assert!(!known_content_length_exceeds(&headers, 1024));
    }
}
