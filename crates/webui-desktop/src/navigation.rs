// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared navigation allowlist for every desktop backend.
//!
//! Each platform loads app content from a different origin: macOS and Linux
//! serve a custom `webui://app` scheme, while Windows maps a virtual host
//! because WebView2 cannot register a custom scheme. The *policy* must not vary
//! with that detail, so the decision lives here and every backend defers to it
//! rather than reimplementing string matching per platform.
//!
//! The policy is deny-by-default: a navigation is permitted only when it targets
//! the app's own origin exactly, targets a path beneath it, or is the empty
//! `about:blank` document that webviews commit before the first real load.

/// The empty document webviews commit before the first real navigation.
const ABOUT_BLANK: &str = "about:blank";

/// Return whether `url` may be navigated to for an app served from `origin`.
///
/// `origin` is the scheme-and-authority prefix with no trailing slash, such as
/// `webui://app` or `https://app.webui.localhost`.
///
/// Matching is deliberately exact rather than a bare `starts_with`, so hostile
/// lookalikes such as `webui://app.evil/` and `webui://application/` are
/// rejected: anything past the origin must begin with a `/` path separator.
#[must_use]
pub fn is_allowed_navigation_url(url: &str, origin: &str) -> bool {
    if url == ABOUT_BLANK || url == origin {
        return true;
    }
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    const WEBUI: &str = "webui://app";
    const HOSTED: &str = "https://app.webui.localhost";

    #[test]
    fn allows_the_app_origin_and_its_paths() {
        assert!(is_allowed_navigation_url("webui://app", WEBUI));
        assert!(is_allowed_navigation_url("webui://app/", WEBUI));
        assert!(is_allowed_navigation_url("webui://app/settings", WEBUI));
        assert!(is_allowed_navigation_url(HOSTED, HOSTED));
        assert!(is_allowed_navigation_url(
            "https://app.webui.localhost/index.html",
            HOSTED
        ));
    }

    #[test]
    fn allows_only_the_empty_document_from_the_about_scheme() {
        // macOS previously allowed the whole `about:` scheme, which is broader
        // than the other backends and needlessly reachable from web content.
        assert!(is_allowed_navigation_url(ABOUT_BLANK, WEBUI));
        assert!(!is_allowed_navigation_url("about:srcdoc", WEBUI));
        assert!(!is_allowed_navigation_url("about:config", WEBUI));
        assert!(!is_allowed_navigation_url("about:blank#x", WEBUI));
    }

    #[test]
    fn rejects_origin_lookalikes() {
        assert!(!is_allowed_navigation_url("webui://app.evil/", WEBUI));
        assert!(!is_allowed_navigation_url("webui://application/", WEBUI));
        assert!(!is_allowed_navigation_url("webui://appevil", WEBUI));
        assert!(!is_allowed_navigation_url(
            "https://app.webui.localhost.evil/",
            HOSTED
        ));
    }

    #[test]
    fn rejects_external_and_embedded_origins() {
        assert!(!is_allowed_navigation_url("https://example.com/", WEBUI));
        assert!(!is_allowed_navigation_url("https://webui://app/", WEBUI));
        assert!(!is_allowed_navigation_url("javascript:alert(1)", WEBUI));
        assert!(!is_allowed_navigation_url("file:///etc/passwd", WEBUI));
        assert!(!is_allowed_navigation_url("", WEBUI));
    }
}
