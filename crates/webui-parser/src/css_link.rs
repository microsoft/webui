// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Link-mode CSS filename resolution.
//!
//! Emitted file names are also the hrefs stored in the protocol and client
//! payloads: they are paths relative to the asset base, which maps to the
//! build output directory. The base itself is applied when rendering.

use crate::{AssetFileNameTemplate, ParserError, Result, DEFAULT_ASSET_FILE_NAME_TEMPLATE};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Default Link-mode CSS filename template.
pub const DEFAULT_CSS_FILE_NAME_TEMPLATE: &str = DEFAULT_ASSET_FILE_NAME_TEMPLATE;

/// Configures Link-mode CSS filenames.
#[derive(Debug, Clone)]
pub struct CssLinkOptions {
    file_name_template: AssetFileNameTemplate,
    cache: Arc<Mutex<HashMap<String, CssLinkHref>>>,
}

/// Resolved Link-mode CSS asset path data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssLinkHref {
    /// File name emitted in the build output directory. Protocol hrefs and
    /// client payloads use it as a path relative to the asset base.
    pub filename: String,
}

impl CssLinkOptions {
    /// Create validated Link-mode CSS options.
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Css`] when the template contains unknown tokens
    /// or unsafe path bytes.
    pub fn try_new(file_name_template: String) -> Result<Self> {
        let file_name_template =
            AssetFileNameTemplate::try_new(file_name_template, "css_file_name_template")
                .map_err(|error| ParserError::Css(error.to_string()))?;
        Ok(Self {
            file_name_template,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Resolve the emitted filename for a component stylesheet.
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

        let resolved = CssLinkHref {
            filename: self
                .file_name_template
                .resolve(tag_name, "css", css_content.as_bytes()),
        };

        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(tag_name.to_string())
            .or_insert_with(|| resolved.clone())
            .clone()
    }

    /// Resolve the emitted filename for a bundled CSS chunk.
    ///
    /// Chunk names live in a build-owned namespace rather than the component
    /// registry, so this deliberately bypasses the per-tag cache: a chunk and a
    /// component that happen to share a name must never resolve to each other's
    /// content hash.
    #[must_use]
    pub fn resolve_chunk(&self, chunk_name: &str, css_content: &str) -> CssLinkHref {
        CssLinkHref {
            filename: self
                .file_name_template
                .resolve(chunk_name, "css", css_content.as_bytes()),
        }
    }

    /// File name template used for Link-mode CSS files.
    #[must_use]
    pub fn file_name_template(&self) -> &str {
        self.file_name_template.file_name_template()
    }
}

impl Default for CssLinkOptions {
    fn default() -> Self {
        Self {
            file_name_template: AssetFileNameTemplate::new_unchecked(
                DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            ),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PartialEq for CssLinkOptions {
    fn eq(&self, other: &Self) -> bool {
        self.file_name_template == other.file_name_template
    }
}

impl Eq for CssLinkOptions {}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn default_template_does_not_require_hash() {
        let options = CssLinkOptions::default();
        assert!(!options.file_name_template.uses_hash());
        let resolved = options.resolve("my-card", ".card { color: red; }");
        assert_eq!(resolved.filename, "my-card.css");
    }

    #[test]
    fn hash_template_uses_content_hash() {
        let options = CssLinkOptions::try_new("[name]-[hash].[ext]".to_string()).unwrap();
        assert!(options.file_name_template.uses_hash());
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
            let result = CssLinkOptions::try_new(template.to_string());
            assert!(result.is_err(), "template should be rejected: {template}");
        }
    }
}
