// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::error::{Error, Result};

pub(crate) fn document_title(title: &str, site_title: &str) -> String {
    if title.is_empty() || title == site_title {
        return site_title.to_string();
    }
    if site_title.is_empty() {
        return title.to_string();
    }
    let mut result = String::with_capacity(title.len() + site_title.len() + 3);
    result.push_str(title);
    result.push_str(" | ");
    result.push_str(site_title);
    result
}

pub(crate) fn document_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len() + 1);
    result.push_str(path);
    if !path.ends_with('/') {
        result.push('/');
    }
    result
}

pub(crate) fn site_origin(url: Option<&str>) -> Result<Option<&str>> {
    let Some(url) = url else {
        return Ok(None);
    };
    let uri = url
        .parse::<actix_web::http::Uri>()
        .map_err(|_| invalid_site_url())?;
    let valid_scheme = matches!(uri.scheme_str(), Some("http" | "https"));
    let valid_authority = uri.authority().is_some_and(|authority| {
        let raw = authority.as_str();
        let port = raw
            .rsplit_once(':')
            .filter(|(_, suffix)| !suffix.ends_with(']'));
        !authority.host().is_empty()
            && !raw.contains('@')
            && port.is_none_or(|(_, value)| value.parse::<u16>().is_ok())
    });
    if !valid_scheme
        || !valid_authority
        || uri.path() != "/"
        || uri.query().is_some()
        || url.contains(['#', '\\'])
    {
        return Err(invalid_site_url());
    }
    Ok(Some(url.trim_end_matches('/')))
}

pub(crate) fn canonical_url(origin: Option<&str>, path: &str) -> Option<String> {
    let origin = origin?;
    let mut result = String::with_capacity(origin.len() + path.len());
    result.push_str(origin);
    result.push_str(path);
    Some(result)
}

#[cold]
#[inline(never)]
fn invalid_site_url() -> Error {
    Error::Build(
        "site.url must be an absolute HTTP(S) origin without credentials, a path, query, or fragment.\nhelp: Use an origin such as https://example.com and configure the deployment path with basePath."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_titles_avoid_repeating_the_site_name() {
        assert_eq!(document_title("Docs", "Docs"), "Docs");
        assert_eq!(document_title("", "Docs"), "Docs");
        assert_eq!(document_title("API", "Docs"), "API | Docs");
        assert_eq!(document_title("API", ""), "API");
    }

    #[test]
    fn document_paths_are_base_prefixed_directory_urls() {
        for (path, expected) in [
            ("/", "/"),
            ("/webui/", "/webui/"),
            ("/nested/docs/guide", "/nested/docs/guide/"),
            ("/nested/docs/playground/", "/nested/docs/playground/"),
        ] {
            assert_eq!(document_path(path), expected);
        }
    }

    #[test]
    fn origins_allow_http_https_and_ports_but_not_page_urls() -> Result<()> {
        assert_eq!(site_origin(None)?, None);
        for origin in [
            "https://example.com",
            "https://example.com/",
            "http://localhost:4000/",
            "https://[::1]:8443",
        ] {
            assert_eq!(
                site_origin(Some(origin))?,
                Some(origin.trim_end_matches('/'))
            );
        }
        for invalid in [
            "",
            "/docs/",
            "//example.com",
            "file:///docs",
            "https://example.com/docs/",
            "https://example.com/?query",
            "https://example.com/#fragment",
            "https://user:password@example.com",
            "https://example.com\\docs",
            "https://example.com:invalid",
            "https://example.com:99999",
        ] {
            let error = site_origin(Some(invalid)).expect_err(invalid).to_string();
            assert!(error.contains("site.url"));
            assert!(error.contains("help:"));
        }
        Ok(())
    }

    #[test]
    fn canonical_urls_keep_the_base_path_exactly_once() {
        assert_eq!(canonical_url(None, "/docs/guide/"), None);
        assert_eq!(
            canonical_url(Some("https://example.com"), "/nested/docs/guide/").as_deref(),
            Some("https://example.com/nested/docs/guide/")
        );
        assert_eq!(
            canonical_url(Some("https://example.com"), "/").as_deref(),
            Some("https://example.com/")
        );
    }
}
