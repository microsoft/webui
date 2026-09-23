// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DesktopError, Result};
use crate::{DesktopResponseContent, DesktopResponseFile};

/// Reserved custom-protocol path for protobuf desktop IPC.
#[cfg(feature = "application-ipc")]
pub const IPC_ENDPOINT: &str = "/_webui/ipc";

/// Default maximum file length delivered by a custom-protocol response.
pub const DEFAULT_MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;

/// Maximum ordinary native API request body size. Typed IPC has its own limits.
pub const DEFAULT_MAX_REQUEST_BYTES: u64 = 1024 * 1024;

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
#[derive(Debug)]
pub struct DesktopProtocolResponse {
    /// HTTP status code.
    pub status: u16,
    /// Content type header value.
    pub content_type: String,
    /// Owned bytes or an already-opened file for bounded streaming delivery.
    pub body: DesktopResponseContent,
}

/// Owned response bytes and any resource reservation backing their lifetime.
///
/// Native adapters must retain the complete body, or both values returned by
/// [`Self::into_parts`], until the native consumer releases the bytes.
pub struct DesktopResponseBody {
    bytes: Vec<u8>,
    lease: Option<DesktopResponseLease>,
}

/// Opaque response-lifetime reservation released when the native body is freed.
pub struct DesktopResponseLease {
    _owner: Box<dyn Send + Sync>,
}

impl DesktopResponseBody {
    /// Attach a reservation to an owned response buffer without copying it.
    #[must_use]
    pub fn with_guard<G: Send + Sync + 'static>(bytes: Vec<u8>, guard: G) -> Self {
        Self {
            bytes,
            lease: Some(DesktopResponseLease {
                _owner: Box::new(guard),
            }),
        }
    }

    /// Borrow response bytes while retaining their reservation.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Transfer bytes and reservation to a native ownership container.
    ///
    /// The returned lease must not be dropped before the last native view of
    /// the corresponding buffer is released.
    #[must_use]
    pub fn into_parts(self) -> (Vec<u8>, Option<DesktopResponseLease>) {
        (self.bytes, self.lease)
    }
}

impl From<Vec<u8>> for DesktopResponseBody {
    fn from(bytes: Vec<u8>) -> Self {
        Self { bytes, lease: None }
    }
}

impl AsRef<[u8]> for DesktopResponseBody {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::ops::Deref for DesktopResponseBody {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl std::fmt::Debug for DesktopResponseBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopResponseBody")
            .field("length", &self.bytes.len())
            .field("reserved", &self.lease.is_some())
            .finish()
    }
}

impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for DesktopResponseBody {
    fn eq(&self, other: &T) -> bool {
        self.bytes.as_slice() == other.as_ref()
    }
}

impl Eq for DesktopResponseBody {}

impl DesktopProtocolResponse {
    /// Create a response.
    #[must_use]
    pub fn new(
        status: u16,
        content_type: impl Into<String>,
        body: impl Into<DesktopResponseContent>,
    ) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body: body.into(),
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
    pub fn protobuf(body: impl Into<DesktopResponseBody>) -> Self {
        Self::new(200, "application/x-protobuf", body.into())
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

    let content_type = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .to_string();

    read_known_asset_response(asset_root, &canonical, &content_type, size, max_asset_bytes)
        .map(Some)
}

pub(crate) fn read_known_asset_response(
    asset_root: &Path,
    asset_path: &Path,
    content_type: &str,
    size_bytes: u64,
    max_asset_bytes: u64,
) -> Result<DesktopProtocolResponse> {
    if size_bytes > max_asset_bytes {
        return Err(DesktopError::AssetTooLarge {
            path: asset_path.to_path_buf(),
            size: size_bytes,
            max_bytes: max_asset_bytes,
        });
    }

    let file = crate::asset_file::open(asset_root, asset_path)?;
    let metadata = file.metadata().map_err(|source| DesktopError::Io {
        context: format!("checking opened desktop asset {}", asset_path.display()),
        source,
    })?;
    if !metadata.is_file() {
        return Err(DesktopError::InvalidAssetPath {
            path: asset_path.display().to_string(),
        });
    }
    if metadata.len() > max_asset_bytes {
        return Err(DesktopError::AssetTooLarge {
            path: asset_path.to_path_buf(),
            size: metadata.len(),
            max_bytes: max_asset_bytes,
        });
    }

    Ok(DesktopProtocolResponse::new(
        200,
        content_type.to_string(),
        DesktopResponseContent::File(DesktopResponseFile::new(file, metadata.len())),
    ))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod response_body_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct ReleaseCount(Arc<AtomicUsize>);

    impl Drop for ReleaseCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn response_body_retains_its_reservation_until_drop() {
        let released = Arc::new(AtomicUsize::new(0));
        let bytes = vec![1, 2, 3];
        let address = bytes.as_ptr();
        let body = DesktopResponseBody::with_guard(bytes, ReleaseCount(Arc::clone(&released)));
        assert_eq!(body.as_slice(), &[1, 2, 3]);
        assert_eq!(body.as_ptr(), address);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(body);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_transfer_preserves_reservation_and_allocation() {
        let released = Arc::new(AtomicUsize::new(0));
        let bytes = vec![4, 5, 6];
        let address = bytes.as_ptr();
        let response = DesktopProtocolResponse::protobuf(DesktopResponseBody::with_guard(
            bytes,
            ReleaseCount(Arc::clone(&released)),
        ));
        let (bytes, lease) = response.body.into_bytes().unwrap().into_parts();
        assert_eq!(bytes.as_ptr(), address);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(bytes);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(lease);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ordinary_responses_do_not_need_a_reservation() {
        let response = DesktopProtocolResponse::new(201, "text/plain", b"created".to_vec());
        assert_eq!(response.body, b"created");
        let (bytes, lease) = response.body.into_bytes().unwrap().into_parts();
        assert_eq!(bytes, b"created");
        assert!(lease.is_none());
    }
}
