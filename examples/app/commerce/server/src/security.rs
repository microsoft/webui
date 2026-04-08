// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::middleware::DefaultHeaders;

/// Returns a [`DefaultHeaders`] middleware that sets standard security headers
/// on every response.
pub(crate) fn security_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add(("X-Content-Type-Options", "nosniff"))
        .add(("X-Frame-Options", "DENY"))
        .add(("Referrer-Policy", "strict-origin-when-cross-origin"))
        .add((
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=()",
        ))
        .add((
            "Content-Security-Policy",
            "default-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' https://cdn.shopify.com; \
             font-src 'self'; \
             frame-ancestors 'none'",
        ))
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
    let host = req.headers().get("host").and_then(|v| v.to_str().ok());
    let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
    let referer = req.headers().get("referer").and_then(|v| v.to_str().ok());

    if let Some(origin) = origin {
        // Origin present → must match, and Host must be present to validate.
        return host.is_some_and(|host| origin_matches_host(origin, host));
    }

    if let Some(referer) = referer {
        // Referer present → must match, and Host must be present to validate.
        return host.is_some_and(|host| referer_matches_host(referer, host));
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
    use super::{origin_matches_host, passes_csrf_check, referer_matches_host};
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
}
