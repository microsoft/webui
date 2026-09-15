// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Startup-validated generic headers and request-local document CSP.

use actix_web::http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_SECURITY_POLICY};
use actix_web::middleware::DefaultHeaders;
use actix_web::HttpResponse;
use anyhow::Result;

const NONCE_MARKER: &str = "{nonce}";
const NONCE_BYTES: usize = 16;
const OWNED_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "content-type",
    "content-encoding",
    "cache-control",
    "etag",
    "last-modified",
    "vary",
];

#[derive(Debug, Default)]
pub(super) struct ResponsePolicy {
    headers: HeaderMap,
    csp: Option<String>,
}

impl ResponsePolicy {
    pub(super) fn new(headers: &[String], csp: Option<&str>) -> Result<Self> {
        let mut parsed = HeaderMap::with_capacity(headers.len());
        for header in headers {
            let (name, value) = parse_header(header)?;
            if parsed.contains_key(&name) {
                return Err(policy_error(
                    "duplicate response header",
                    "Provide each --header name only once (names are case-insensitive).",
                ));
            }
            parsed.insert(name, value);
        }
        if let Some(policy) = csp {
            validate_csp(policy, &parsed)?;
        }
        Ok(Self {
            headers: parsed,
            csp: csp.map(str::to_owned),
        })
    }

    /// Apply configured defaults without replacing endpoint-owned headers.
    #[must_use]
    pub(super) fn default_headers(&self) -> DefaultHeaders {
        self.headers
            .iter()
            .fold(DefaultHeaders::new(), |middleware, (name, value)| {
                middleware.add((name.clone(), value.clone()))
            })
    }

    #[must_use]
    pub(super) fn is_csp_enabled(&self) -> bool {
        self.csp.is_some()
    }

    /// Generate fresh OS entropy for one document, never for JSON or SSE.
    pub(super) fn nonce(&self) -> Result<Option<String>> {
        self.nonce_with(getrandom::fill)
    }

    fn nonce_with(
        &self,
        fill: impl FnOnce(&mut [u8]) -> std::result::Result<(), getrandom::Error>,
    ) -> Result<Option<String>> {
        if !self.is_csp_enabled() {
            return Ok(None);
        }
        let mut bytes = [0; NONCE_BYTES];
        fill(&mut bytes).map_err(entropy_error)?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut nonce = String::with_capacity(NONCE_BYTES * 2);
        for byte in bytes {
            nonce.push(char::from(HEX[usize::from(byte >> 4)]));
            nonce.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Ok(Some(nonce))
    }

    /// Set the nonce policy on an HTML response after rendering with that nonce.
    ///
    /// An enabled policy requires a nonempty CSP-safe nonce. Non-HTML responses
    /// are unchanged; their original document's nonce remains authoritative.
    pub(super) fn apply_document(
        &self,
        response: &mut HttpResponse,
        nonce: Option<&str>,
    ) -> Result<()> {
        let Some(policy) = &self.csp else {
            return Ok(());
        };
        let nonce = nonce.filter(|value| {
            !value.is_empty()
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'_' | b'-' | b'=')
                })
        });
        let Some(nonce) = nonce else {
            return Err(policy_error(
                "missing or invalid document nonce",
                "Generate a nonce with ResponsePolicy::nonce and use it for both rendering and CSP.",
            ));
        };
        if !is_html(response) {
            return Ok(());
        }
        let value = HeaderValue::try_from(policy.replace(NONCE_MARKER, nonce)).map_err(|_| {
            policy_error(
                "invalid document CSP header",
                "Use an HTTP-safe --csp value and the generated document nonce.",
            )
        })?;
        response
            .headers_mut()
            .insert(CONTENT_SECURITY_POLICY, value);
        Ok(())
    }
}

fn parse_header(header: &str) -> Result<(HeaderName, HeaderValue)> {
    reject_newlines(header)?;
    let (name, value) = header.split_once(':').ok_or_else(|| {
        policy_error(
            "invalid response header",
            "Format --header as \"Name: value\".",
        )
    })?;
    let name = HeaderName::try_from(name).map_err(|_| {
        policy_error(
            "invalid response header name",
            "Use an HTTP token for the --header name, without whitespace.",
        )
    })?;
    if OWNED_HEADERS.contains(&name.as_str()) {
        return Err(policy_error(
            "server-owned response header",
            "Remove this --header; WebUI owns transport, content, cache, and validator headers.",
        ));
    }
    if value.contains(NONCE_MARKER) {
        return Err(policy_error(
            "nonce placeholder in generic response header",
            "Use --csp for nonce substitution instead of --header.",
        ));
    }
    let value = HeaderValue::try_from(value.trim_matches([' ', '\t'])).map_err(|_| {
        policy_error(
            "invalid response header value",
            "Use an HTTP-safe --header value without control characters.",
        )
    })?;
    Ok((name, value))
}

fn validate_csp(policy: &str, headers: &HeaderMap) -> Result<()> {
    reject_newlines(policy)?;
    if headers.contains_key(CONTENT_SECURITY_POLICY) {
        return Err(policy_error(
            "conflicting Content-Security-Policy options",
            "Use either --csp or --header Content-Security-Policy, not both.",
        ));
    }
    if !policy.contains(NONCE_MARKER) {
        return Err(policy_error(
            "CSP nonce placeholder is required",
            "Include {nonce} in --csp, for example \"script-src 'nonce-{nonce}'\".",
        ));
    }
    HeaderValue::from_str(policy).map_err(|_| {
        policy_error(
            "invalid Content-Security-Policy value",
            "Use an HTTP-safe --csp value without control characters.",
        )
    })?;
    Ok(())
}

fn reject_newlines(value: &str) -> Result<()> {
    if value.contains(['\r', '\n']) {
        return Err(policy_error(
            "response header contains a newline",
            "Remove CR/LF characters from --header and --csp values.",
        ));
    }
    Ok(())
}

fn is_html(response: &HttpResponse) -> bool {
    response
        .headers()
        .get(actix_web::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let mime = value.split(';').next().unwrap_or_default().trim();
            mime.eq_ignore_ascii_case("text/html")
                || mime.eq_ignore_ascii_case("application/xhtml+xml")
        })
}

#[cold]
#[inline(never)]
fn policy_error(message: &str, help: &str) -> anyhow::Error {
    anyhow::anyhow!("{message}\nhelp: {help}")
}

#[cold]
#[inline(never)]
fn entropy_error(error: getrandom::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "cannot obtain OS randomness for document CSP: {error}\n\
         help: Restore operating-system random source access and retry; no document was authorized."
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use actix_web::{test as actix_test, web, App};

    use super::*;

    #[test]
    fn parses_first_colon_and_trims_only_value_whitespace() {
        let policy =
            ResponsePolicy::new(&["X-Target:\t https://localhost:3000/path \t".into()], None)
                .unwrap();
        assert_eq!(
            policy.headers.get("x-target").unwrap(),
            "https://localhost:3000/path"
        );
        assert!(ResponsePolicy::new(&["X-Empty:".into()], None).is_ok());
    }

    #[test]
    fn rejects_invalid_duplicate_and_owned_headers() {
        for header in [
            "missing-colon",
            ": missing-name",
            " X-Name: value",
            "X-Name : value",
            "X-Bad Name: value",
            "X-Name: a\r\nX-Injected: b",
            "X-Name: a\n",
            "X-Name: a\r",
            "X-Name: \0",
            "X-Name: \u{7f}",
            "X-Name: {nonce}",
        ] {
            let error = ResponsePolicy::new(&[header.into()], None).unwrap_err();
            assert!(error.to_string().contains("help:"), "{header:?}");
        }
        assert!(ResponsePolicy::new(&["X-Name: a".into(), "x-name: b".into()], None).is_err());
        for name in OWNED_HEADERS {
            assert!(
                ResponsePolicy::new(&[format!("{}: value", name.to_ascii_uppercase())], None)
                    .is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn validates_csp_and_literal_header_conflicts() {
        for csp in [
            "",
            "default-src 'self'",
            "script-src 'nonce-{nonce}'\r\nX-Injected: value",
            "script-src 'nonce-{nonce}'\0",
        ] {
            assert!(ResponsePolicy::new(&[], Some(csp)).is_err(), "{csp:?}");
        }
        let literal = ["cOnTeNt-SeCuRiTy-PoLiCy: default-src 'self'".into()];
        let policy = ResponsePolicy::new(&literal, None).unwrap();
        assert!(!policy.is_csp_enabled());
        assert!(policy.nonce().unwrap().is_none());
        assert!(ResponsePolicy::new(&literal, Some("script-src 'nonce-{nonce}'")).is_err());
    }

    #[test]
    fn nonce_uses_all_entropy_and_is_fresh_per_call() {
        let policy = ResponsePolicy::new(&[], Some("script-src 'nonce-{nonce}'")).unwrap();
        let deterministic = policy
            .nonce_with(|bytes| {
                bytes.copy_from_slice(&[
                    0x00, 0x01, 0x0f, 0x10, 0x7f, 0x80, 0xab, 0xff, 0x01, 0x23, 0x45, 0x67, 0x89,
                    0xab, 0xcd, 0xef,
                ]);
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(deterministic, "00010f107f80abff0123456789abcdef");
        let first = policy.nonce().unwrap().unwrap();
        let second = policy.nonce().unwrap().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn entropy_failure_is_explicit_and_disabled_policy_does_not_read_entropy() {
        let policy = ResponsePolicy::new(&[], Some("script-src 'nonce-{nonce}'")).unwrap();
        let error = policy
            .nonce_with(|_| Err(getrandom::Error::UNSUPPORTED))
            .unwrap_err();
        assert!(error.to_string().contains("OS randomness"));
        assert!(error.to_string().contains("help:"));
        assert!(ResponsePolicy::default()
            .nonce_with(|_| panic!("disabled CSP must not request entropy"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn document_policy_substitutes_every_marker_without_changing_body() {
        let policy = ResponsePolicy::new(
            &[],
            Some("script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'"),
        )
        .unwrap();
        let mut response = HttpResponse::InternalServerError()
            .content_type("text/html; charset=utf-8")
            .body("<script nonce=\"abc123\">run()</script>");
        policy
            .apply_document(&mut response, Some("abc123"))
            .unwrap();
        assert_eq!(
            response.headers().get(CONTENT_SECURITY_POLICY).unwrap(),
            "script-src 'nonce-abc123'; style-src 'nonce-abc123'"
        );
        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let body = actix_web::body::MessageBody::try_into_bytes(response.into_body()).unwrap();
        assert_eq!(body, "<script nonce=\"abc123\">run()</script>");
    }

    #[test]
    fn missing_or_unsafe_nonce_cannot_silently_authorize_document() {
        let policy = ResponsePolicy::new(&[], Some("script-src 'nonce-{nonce}'")).unwrap();
        let mut response = HttpResponse::Ok().content_type("text/html").finish();
        for nonce in [
            None,
            Some(""),
            Some("x' 'unsafe-inline"),
            Some("x\r\nInjected: y"),
        ] {
            assert!(policy.apply_document(&mut response, nonce).is_err());
            assert!(!response.headers().contains_key(CONTENT_SECURITY_POLICY));
        }
        ResponsePolicy::default()
            .apply_document(&mut response, None)
            .unwrap();
    }

    #[actix_web::test]
    async fn generic_middleware_covers_success_errors_json_sse_and_missing_routes() {
        let policy = ResponsePolicy::new(
            &["X-Content-Type-Options: nosniff".into()],
            Some("script-src 'nonce-{nonce}'"),
        )
        .unwrap();
        let app = actix_test::init_service(
            App::new()
                .wrap(policy.default_headers())
                .default_service(web::to(|| async { HttpResponse::NotFound().finish() }))
                .route(
                    "/json",
                    web::get().to(|| async {
                        HttpResponse::Ok()
                            .content_type("application/json")
                            .body("{}")
                    }),
                )
                .route(
                    "/sse",
                    web::get().to(|| async {
                        HttpResponse::Ok()
                            .content_type("text/event-stream")
                            .insert_header(("cache-control", "no-cache"))
                            .body(": connected\n\n")
                    }),
                )
                .route(
                    "/error",
                    web::get().to(|| async {
                        HttpResponse::InternalServerError()
                            .insert_header(("X-Content-Type-Options", "endpoint-value"))
                            .finish()
                    }),
                ),
        )
        .await;
        for path in ["/json", "/sse", "/error", "/missing"] {
            let response = actix_test::call_service(
                &app,
                actix_test::TestRequest::get().uri(path).to_request(),
            )
            .await;
            let expected = if path == "/error" {
                "endpoint-value"
            } else {
                "nosniff"
            };
            assert_eq!(
                response.headers().get("x-content-type-options").unwrap(),
                expected
            );
            assert!(!response.headers().contains_key(CONTENT_SECURITY_POLICY));
            if path == "/sse" {
                assert_eq!(response.headers().get("cache-control").unwrap(), "no-cache");
            }
        }
    }

    #[test]
    fn document_policy_does_not_apply_to_json_sse_or_assets() {
        let policy = ResponsePolicy::new(&[], Some("script-src 'nonce-{nonce}'")).unwrap();
        for content_type in [
            "application/json",
            "text/event-stream",
            "text/javascript",
            "text/css",
        ] {
            let mut response = HttpResponse::Ok().content_type(content_type).finish();
            policy
                .apply_document(&mut response, Some("abc123"))
                .unwrap();
            assert!(!response.headers().contains_key(CONTENT_SECURITY_POLICY));
        }
    }
}
