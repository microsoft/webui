// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DesktopError, Result};

/// Reserved custom-protocol path for protobuf desktop IPC.
pub const IPC_ENDPOINT: &str = "/_webui/ipc";

/// Default maximum asset size read into one custom-protocol response.
pub const DEFAULT_MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;

/// HTTP method for a desktop custom-protocol request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopHttpMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// Any other method.
    Other(String),
}

impl DesktopHttpMethod {
    /// Parse an HTTP method name.
    #[must_use]
    pub fn parse(method: &str) -> Self {
        match method {
            "GET" | "get" => Self::Get,
            "POST" | "post" => Self::Post,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Runtime-neutral representation of a custom-protocol request.
pub struct DesktopProtocolRequest<'a> {
    /// Request method.
    pub method: DesktopHttpMethod,
    /// URL path, including a leading slash.
    pub path: &'a str,
    /// Request body bytes.
    pub body: &'a [u8],
    /// Whether the request asks for a router JSON/NDJSON response.
    pub wants_json: bool,
}

impl<'a> DesktopProtocolRequest<'a> {
    /// Create a GET request.
    #[must_use]
    pub fn get(path: &'a str) -> Self {
        Self {
            method: DesktopHttpMethod::Get,
            path,
            body: &[],
            wants_json: false,
        }
    }

    /// Create a POST request.
    #[must_use]
    pub fn post(path: &'a str, body: &'a [u8]) -> Self {
        Self {
            method: DesktopHttpMethod::Post,
            path,
            body,
            wants_json: false,
        }
    }
}

/// Runtime-neutral representation of a custom-protocol response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopProtocolResponse {
    /// HTTP status code.
    pub status: u16,
    /// Content type header value.
    pub content_type: String,
    /// Response body bytes.
    pub body: Vec<u8>,
}

impl DesktopProtocolResponse {
    /// Create a response.
    #[must_use]
    pub fn new(status: u16, content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body,
        }
    }

    /// Create a plain-text response.
    #[must_use]
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    /// Create a protobuf response.
    #[must_use]
    pub fn protobuf(body: Vec<u8>) -> Self {
        Self::new(200, "application/x-protobuf", body)
    }

    /// Create an HTML response.
    #[must_use]
    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self::new(200, "text/html; charset=utf-8", body.into())
    }
}

pub(crate) fn read_asset_response(
    asset_root: &Path,
    asset_path: PathBuf,
    max_asset_bytes: u64,
) -> Result<Option<DesktopProtocolResponse>> {
    let canonical = match asset_path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(DesktopError::Io {
                context: format!("resolving desktop asset {}", asset_path.display()),
                source,
            })
        }
    };

    if !canonical.starts_with(asset_root) {
        return Err(DesktopError::InvalidAssetPath {
            path: canonical.display().to_string(),
        });
    }

    let metadata = fs::metadata(&canonical).map_err(|source| DesktopError::Io {
        context: format!("reading metadata for desktop asset {}", canonical.display()),
        source,
    })?;

    if !metadata.is_file() {
        return Ok(None);
    }

    let size = metadata.len();
    if size > max_asset_bytes {
        return Err(DesktopError::AssetTooLarge {
            path: canonical,
            size,
            max_bytes: max_asset_bytes,
        });
    }

    let body = fs::read(&canonical).map_err(|source| DesktopError::Io {
        context: format!("reading desktop asset {}", canonical.display()),
        source,
    })?;
    let content_type = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .to_string();

    Ok(Some(DesktopProtocolResponse::new(200, content_type, body)))
}
