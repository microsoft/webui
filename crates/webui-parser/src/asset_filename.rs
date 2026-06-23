// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared filename-template handling for emitted build assets.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use thiserror::Error;

/// Default build-asset filename template.
pub const DEFAULT_ASSET_FILE_NAME_TEMPLATE: &str = "[name].[ext]";

/// Error returned when an asset filename template is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct AssetFileNameTemplateError {
    message: String,
}

impl AssetFileNameTemplateError {
    fn new(message: String) -> Self {
        Self { message }
    }
}

/// Validated filename template for emitted build assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFileNameTemplate {
    file_name_template: String,
    uses_hash: bool,
}

impl AssetFileNameTemplate {
    /// Create a validated asset filename template.
    ///
    /// Supported tokens are `[name]`, `[hash]`, and `[ext]`.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFileNameTemplateError`] when the template is empty,
    /// contains unsafe filename characters, or uses an unknown token.
    pub fn try_new(
        file_name_template: String,
        option_name: &str,
    ) -> Result<Self, AssetFileNameTemplateError> {
        validate_template(&file_name_template, option_name)?;
        Ok(Self {
            uses_hash: file_name_template.contains("[hash]"),
            file_name_template,
        })
    }

    pub(crate) fn new_unchecked(file_name_template: String) -> Self {
        Self {
            uses_hash: file_name_template.contains("[hash]"),
            file_name_template,
        }
    }

    /// Resolve a concrete filename for an emitted asset.
    ///
    /// `[hash]` is the SHA-256 content hash truncated to 8 hex characters.
    #[must_use]
    pub fn resolve(&self, name: &str, ext: &str, content: &[u8]) -> String {
        let safe_name = name.replace(['/', '\\'], "-");
        let hash = if self.uses_hash {
            short_sha256_hex(content)
        } else {
            String::new()
        };
        format_asset_filename(&self.file_name_template, &safe_name, ext, &hash)
    }

    /// Filename template text.
    #[must_use]
    pub fn file_name_template(&self) -> &str {
        &self.file_name_template
    }

    /// Whether the template includes `[hash]`.
    #[must_use]
    pub fn uses_hash(&self) -> bool {
        self.uses_hash
    }
}

fn validate_template(template: &str, option_name: &str) -> Result<(), AssetFileNameTemplateError> {
    if template.is_empty() {
        return Err(AssetFileNameTemplateError::new(format!(
            "{option_name} cannot be empty"
        )));
    }
    if contains_invalid_filename_template_byte(template) {
        return Err(AssetFileNameTemplateError::new(format!(
            "{option_name} must be ASCII and cannot contain path separators, Windows-reserved filename characters, control characters, or whitespace"
        )));
    }
    if template.contains("..") {
        return Err(AssetFileNameTemplateError::new(format!(
            "{option_name} cannot contain '..'"
        )));
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
                return Err(AssetFileNameTemplateError::new(format!(
                    "Invalid {option_name} '{template}': missing closing ']'"
                )));
            }
            let token = &template[start..j];
            match token {
                "name" | "hash" | "ext" => {}
                _ => {
                    return Err(AssetFileNameTemplateError::new(format!(
                        "Invalid {option_name} '{template}': unknown token '[{token}]'. Allowed tokens: [name], [hash], [ext]"
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

fn format_asset_filename(template: &str, name: &str, ext: &str, hash: &str) -> String {
    debug_assert!(template.is_ascii());
    let mut out = String::with_capacity(template.len() + name.len() + ext.len() + 16);
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
                "ext" => out.push_str(ext),
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
    fn hash_template_uses_content_hash() {
        let template =
            AssetFileNameTemplate::try_new("[name]-[hash].[ext]".to_string(), "asset").unwrap();
        let first = template.resolve("my-card", "webui.js", b"one");
        let second = template.resolve("my-card", "webui.js", b"two");

        assert!(first.starts_with("my-card-"));
        assert!(first.ends_with(".webui.js"));
        assert_ne!(first, second);
    }

    #[test]
    fn rejects_unsafe_characters() {
        for template in [
            "[name]:[hash].[ext]",
            "[name]*[hash].[ext]",
            "[name]?[hash].[ext]",
            "[name]|[hash].[ext]",
            "resume [hash].[ext]",
            "résumé-[hash].[ext]",
        ] {
            let result = AssetFileNameTemplate::try_new(template.to_string(), "asset");
            assert!(result.is_err(), "template should be rejected: {template}");
        }
    }
}
