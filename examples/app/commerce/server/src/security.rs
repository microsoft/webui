// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::middleware::DefaultHeaders;
use std::sync::atomic::{AtomicU64, Ordering};

/// Returns a [`DefaultHeaders`] middleware that sets standard security headers
/// on every response.  The `Content-Security-Policy` header is **not** included
/// here because it contains a per-request nonce — use [`csp_header`] instead.
pub(crate) fn security_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add(("X-Content-Type-Options", "nosniff"))
        .add(("X-Frame-Options", "DENY"))
        .add(("Referrer-Policy", "strict-origin-when-cross-origin"))
        .add((
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=()",
        ))
}

/// Generate a unique CSP nonce for a single request.
///
/// Uses a monotonic counter mixed with the process ID to produce a
/// hex-encoded value that is unique per request within this process.
/// This avoids pulling in a full CSPRNG crate for a demo server.
pub(crate) fn generate_nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid:08x}{count:016x}")
}

/// Build a CSP header value that allows inline scripts matching `nonce`.
#[must_use]
pub(crate) fn csp_header(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self'; \
         font-src 'self'; \
         frame-ancestors 'none'"
    )
}

/// Extract the client IP address for rate-limiting purposes.
///
/// When deployed behind a reverse proxy (Docker, CDN), `peer_addr()` returns
/// the proxy's IP.  This helper checks `X-Forwarded-For` first and falls back
/// to the TCP peer address.
///
/// **Trust model:** Only the *first* entry in `X-Forwarded-For` is used, which
/// is the client-facing IP set by the outermost trusted proxy.  If the server
/// is directly internet-facing without a proxy, clients can spoof this header
/// — but the worst outcome is per-IP rate-limit evasion, which is acceptable
/// for a demo.
pub(crate) fn client_ip(req: &actix_web::HttpRequest) -> Option<std::net::IpAddr> {
    if let Some(forwarded) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        let first = forwarded.split(',').next().unwrap_or(forwarded).trim();
        if let Ok(ip) = first.parse::<std::net::IpAddr>() {
            return Some(ip);
        }
    }
    req.peer_addr().map(|a| a.ip())
}

/// Checks the `Origin` (or `Referer`) header on a request to verify it
/// originated from the same site.  Returns `true` when the request is
/// safe to process.
///
/// Policy (per OWASP double-submit recommendations):
/// - If `Origin` is present it **must** match the `Host` header.
///   If `Host` is missing, reject because validation is impossible.
/// - If `Origin` is absent, fall back to `Referer` and compare its host.
///   If `Host` is missing, reject for the same reason.
/// - If neither `Origin` nor `Referer` is present, allow the request.
///   Same-origin form submissions may omit both in some browsers/proxies
///   and test harnesses.
pub(crate) fn passes_csrf_check(req: &actix_web::HttpRequest) -> bool {
    // HTTP/2 uses the `:authority` pseudo-header instead of `Host`.
    // `connection_info().host()` handles both transparently.
    let info = req.connection_info();
    let host = info.host();
    if host.is_empty() {
        // No host available — can't validate Origin/Referer.
        // Allow only when neither is present (same-origin or non-browser).
        return req.headers().get("origin").is_none() && req.headers().get("referer").is_none();
    }
    let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
    let referer = req.headers().get("referer").and_then(|v| v.to_str().ok());

    if let Some(origin) = origin {
        return origin_matches_host(origin, host);
    }

    if let Some(referer) = referer {
        return referer_matches_host(referer, host);
    }

    // Neither Origin nor Referer → same-origin or non-browser client.
    true
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    // Origin is typically "https://host:port" or "http://host:port"
    let stripped = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"));
    match stripped {
        Some(origin_host) => origin_host == host,
        // Bare origin without scheme — compare directly
        None => origin == host,
    }
}

fn referer_matches_host(referer: &str, host: &str) -> bool {
    let stripped = referer
        .strip_prefix("https://")
        .or_else(|| referer.strip_prefix("http://"));
    match stripped {
        Some(rest) => {
            // rest is "host:port/path?query", extract just host:port
            let referer_host = rest.split('/').next().unwrap_or(rest);
            referer_host == host
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        client_ip, csp_header, generate_nonce, origin_matches_host, passes_csrf_check,
        referer_matches_host,
    };
    use actix_web::test::TestRequest;

    #[test]
    fn origin_matches_with_scheme() {
        assert!(origin_matches_host(
            "https://localhost:3004",
            "localhost:3004"
        ));
        assert!(origin_matches_host(
            "http://localhost:3004",
            "localhost:3004"
        ));
        assert!(!origin_matches_host("https://evil.com", "localhost:3004"));
    }

    #[test]
    fn referer_matches_host_part() {
        assert!(referer_matches_host(
            "https://localhost:3004/search?q=test",
            "localhost:3004"
        ));
        assert!(!referer_matches_host(
            "https://evil.com/path",
            "localhost:3004"
        ));
    }

    #[test]
    fn csrf_allows_same_origin() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("host", "localhost:3004"))
            .insert_header(("origin", "http://localhost:3004"))
            .to_http_request();
        assert!(passes_csrf_check(&req));
    }

    #[test]
    fn csrf_rejects_cross_origin() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("host", "localhost:3004"))
            .insert_header(("origin", "https://evil.com"))
            .to_http_request();
        assert!(!passes_csrf_check(&req));
    }

    #[test]
    fn csrf_allows_missing_origin_and_referer() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("host", "localhost:3004"))
            .to_http_request();
        assert!(passes_csrf_check(&req));
    }

    #[test]
    fn csrf_rejects_cross_site_referer_when_no_origin() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("host", "localhost:3004"))
            .insert_header(("referer", "https://evil.com/attack"))
            .to_http_request();
        assert!(!passes_csrf_check(&req));
    }

    #[test]
    fn csrf_allows_same_site_referer_when_no_origin() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("host", "localhost:3004"))
            .insert_header(("referer", "http://localhost:3004/product/acme-t-shirt"))
            .to_http_request();
        assert!(passes_csrf_check(&req));
    }

    #[test]
    fn csrf_rejects_origin_when_host_missing() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("origin", "https://evil.com"))
            .to_http_request();
        assert!(!passes_csrf_check(&req));
    }

    #[test]
    fn csrf_rejects_referer_when_host_missing() {
        let req = TestRequest::post()
            .uri("/cart/add")
            .insert_header(("referer", "https://evil.com/attack"))
            .to_http_request();
        assert!(!passes_csrf_check(&req));
    }

    #[test]
    fn csrf_allows_when_no_headers_at_all() {
        let req = TestRequest::post().uri("/cart/add").to_http_request();
        assert!(passes_csrf_check(&req));
    }

    #[test]
    fn nonce_values_are_unique() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn csp_header_includes_nonce() {
        let nonce = "abc123";
        let csp = csp_header(nonce);
        assert!(csp.contains("'nonce-abc123'"));
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "script-src must use nonce, not unsafe-inline"
        );
    }

    #[test]
    fn client_ip_returns_none_without_peer_or_header() {
        let req = TestRequest::get().uri("/").to_http_request();
        // TestRequest has no peer_addr and no X-Forwarded-For
        assert!(client_ip(&req).is_none());
    }

    #[test]
    fn client_ip_parses_x_forwarded_for() {
        let req = TestRequest::get()
            .uri("/")
            .insert_header(("x-forwarded-for", "203.0.113.50, 70.41.3.18"))
            .to_http_request();
        let ip = client_ip(&req);
        assert_eq!(
            ip,
            Some(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
                203, 0, 113, 50
            )))
        );
    }
}
