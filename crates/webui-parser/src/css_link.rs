// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Link-mode CSS filename and public href resolution.

use crate::{ParserError, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

/// Default Link-mode CSS filename template.
pub const DEFAULT_CSS_FILE_NAME_TEMPLATE: &str = "[name].[ext]";

/// Configures Link-mode CSS filenames and public hrefs.
#[derive(Debug, Clone)]
pub struct CssLinkOptions {
    file_name_template: String,
    public_base: Option<String>,
    uses_hash: bool,
    cache: Arc<Mutex<HashMap<String, CssLinkHref>>>,
}

/// Resolved Link-mode CSS asset path data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssLinkHref {
    /// Local filename emitted by build output.
    pub filename: String,
    /// Public href used in parser/handler `<link>` tags.
    pub href: String,
}

impl CssLinkOptions {
    /// Create validated Link-mode CSS options.
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Css`] when the template contains unknown tokens,
    /// unsafe path bytes, or an unsafe public base.
    pub fn try_new(file_name_template: String, public_base: Option<String>) -> Result<Self> {
        validate_css_link_template(&file_name_template)?;
        if let Some(base) = public_base.as_deref() {
            validate_css_public_base(base)?;
        }
        Ok(Self {
            uses_hash: file_name_template.contains("[hash]"),
            file_name_template,
            public_base,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Resolve the local filename and public href for a component stylesheet.
    ///
    /// Results are cached by component tag. A `CssLinkOptions` value should be
    /// scoped to one build/component registry where each tag maps to exactly
    /// one stylesheet.
    #[must_use]
    pub fn resolve(&self, tag_name: &str, css_content: &str) -> CssLinkHref {
        {
            let cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(cached) = cache.get(tag_name) {
                return cached.clone();
            }
        }

        let safe_tag = tag_name.replace(['/', '\\'], "-");
        let hash = if self.uses_hash {
            short_sha256_hex(css_content.as_bytes())
        } else {
            String::new()
        };
        let filename = format_css_link_filename(&self.file_name_template, &safe_tag, &hash);
        let href = match self.public_base.as_deref() {
            Some(base) => join_css_public_base(base, &filename),
            None => filename.clone(),
        };
        let resolved = CssLinkHref { filename, href };

        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(tag_name.to_string())
            .or_insert_with(|| resolved.clone())
            .clone()
    }

    /// File name template used for Link-mode CSS files.
    #[must_use]
    pub fn file_name_template(&self) -> &str {
        &self.file_name_template
    }

    /// Optional public base prepended to Link-mode CSS hrefs.
    #[must_use]
    pub fn public_base(&self) -> Option<&str> {
        self.public_base.as_deref()
    }
}

impl Default for CssLinkOptions {
    fn default() -> Self {
        Self {
            file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            public_base: None,
            uses_hash: false,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PartialEq for CssLinkOptions {
    fn eq(&self, other: &Self) -> bool {
        self.file_name_template == other.file_name_template
            && self.public_base == other.public_base
            && self.uses_hash == other.uses_hash
    }
}

impl Eq for CssLinkOptions {}

fn validate_css_link_template(template: &str) -> Result<()> {
    if template.is_empty() {
        return Err(ParserError::Css(
            "css_file_name_template cannot be empty".to_string(),
        ));
    }
    if contains_invalid_filename_template_byte(template) {
        return Err(ParserError::Css(
            "css_file_name_template must be ASCII and cannot contain path separators, Windows-reserved filename characters, control characters, or whitespace".to_string(),
        ));
    }
    if template.contains("..") {
        return Err(ParserError::Css(
            "css_file_name_template cannot contain '..'".to_string(),
        ));
    }
    let bytes = template.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b']' {
                j += 1;
            }
            if j >= bytes.len() {
                return Err(ParserError::Css(format!(
                    "Invalid css_file_name_template '{template}': missing closing ']'"
                )));
            }
            let token = &template[start..j];
            match token {
                "name" | "hash" | "ext" => {}
                _ => {
                    return Err(ParserError::Css(format!(
                        "Invalid css_file_name_template '{template}': unknown token '[{token}]'. Allowed tokens: [name], [hash], [ext]"
                    )));
                }
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }

    Ok(())
}

fn validate_css_public_base(base: &str) -> Result<()> {
    if base.is_empty() {
        return Err(ParserError::Css(
            "css_public_base cannot be empty".to_string(),
        ));
    }
    if contains_invalid_href_byte(base) {
        return Err(ParserError::Css(
            "css_public_base cannot contain quotes, angle brackets, backslashes, or whitespace"
                .to_string(),
        ));
    }
    Ok(())
}

fn format_css_link_filename(template: &str, name: &str, hash: &str) -> String {
    debug_assert!(template.is_ascii());
    let mut out = String::with_capacity(template.len() + 16);
    let bytes = template.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b']' {
                j += 1;
            }
            let token = &template[start..j];
            match token {
                "name" => out.push_str(name),
                "hash" => out.push_str(hash),
                "ext" => out.push_str("css"),
                _ => {}
            }
            i = j + 1;
            continue;
        }

        out.push(char::from(bytes[i]));
        i += 1;
    }
    out
}

fn short_sha256_hex(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut out = String::with_capacity(8);
    for b in &digest[..4] {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn join_css_public_base(base: &str, css_filename: &str) -> String {
    let mut href = String::with_capacity(base.len() + css_filename.len() + 1);
    href.push_str(base);
    if !base.ends_with('/') {
        href.push('/');
    }
    href.push_str(css_filename);
    href
}

fn contains_invalid_href_byte(value: &str) -> bool {
    value
        .bytes()
        .any(|b| matches!(b, b'"' | b'\'' | b'<' | b'>' | b'\\') || b.is_ascii_whitespace())
}

fn contains_invalid_filename_template_byte(value: &str) -> bool {
    !value.is_ascii()
        || value.bytes().any(|b| {
            matches!(
                b,
                0x00..=0x1F | b'"' | b'*' | b'/' | b':' | b'<' | b'>' | b'?' | b'\\' | b'|'
            ) || b.is_ascii_whitespace()
        })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn default_template_does_not_require_hash() {
        let options = CssLinkOptions::default();
        assert!(!options.uses_hash);
        let resolved = options.resolve("my-card", ".card { color: red; }");
        assert_eq!(resolved.filename, "my-card.css");
    }

    #[test]
    fn hash_template_uses_content_hash() {
        let options = CssLinkOptions::try_new("[name]-[hash].[ext]".to_string(), None).unwrap();
        assert!(options.uses_hash);
        let first = options.resolve("my-card", ".card { color: red; }");
        let second = options.resolve("other-card", ".card { color: blue; }");

        assert!(first.filename.starts_with("my-card-"));
        assert!(second.filename.starts_with("other-card-"));
        assert_ne!(
            first.filename.rsplit_once('-').map(|(_, hash)| hash),
            second.filename.rsplit_once('-').map(|(_, hash)| hash)
        );
    }

    #[test]
    fn filename_template_rejects_filesystem_unsafe_characters() {
        for template in [
            "[name]:[hash].[ext]",
            "[name]*[hash].[ext]",
            "[name]?[hash].[ext]",
            "[name]|[hash].[ext]",
            "résumé-[hash].[ext]",
        ] {
            let result = CssLinkOptions::try_new(template.to_string(), None);
            assert!(result.is_err(), "template should be rejected: {template}");
        }
    }
}
