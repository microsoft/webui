// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded host-only delivery for a single local-server window.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::{mpsc, Arc};

#[cfg(target_os = "macos")]
use super::HostLifetime;
use crate::WindowId;

/// Maximum UTF-8 bytes in one incoming URL.
pub const MAX_URL_ACTIVATION_BYTES: usize = 2048;
/// Maximum URLs accepted in one AppKit delivery and before window readiness.
pub const MAX_URL_ACTIVATIONS_PER_BATCH: usize = 8;

/// Validated incoming URL for the primary local-server window.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UrlActivation {
    /// The window this activation belongs to.
    pub window_id: WindowId,
    /// The incoming custom-scheme URL. Treat its path and query as untrusted.
    pub url: String,
}

/// Failure to install the single incoming URL callback.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum UrlActivationRegistrationError {
    /// Scheme is not a bounded lower-case custom-scheme identifier.
    #[error("invalid URL activation scheme; help: use 2-32 lower-case ASCII letters, digits or hyphens, starting with a letter, not a reserved scheme")]
    InvalidScheme,
    /// This frame already registered a callback.
    #[error("URL activation handler already registered; help: register only once before running the frame")]
    AlreadyRegistered,
    /// This target has no incoming URL adapter.
    #[error("incoming URL activation is unsupported on this platform; help: use a macOS local-server frame")]
    Unsupported,
    /// The owning server has already retired.
    #[error(
        "URL activation host has retired; help: create a new local-server frame before registering"
    )]
    Closed,
    /// The registration or delivery worker could not be started.
    #[error("URL activation worker unavailable; help: retry with a new local-server frame")]
    Unavailable,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rejection {
    TooMany,
    TooLong,
    Control,
    InvalidUrl,
    WrongScheme,
    Retired,
    Full,
}

#[cfg(target_os = "macos")]
impl Rejection {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::TooMany => "batch-limit",
            Self::TooLong => "url-length",
            Self::Control => "url-control",
            Self::InvalidUrl => "invalid-url",
            Self::WrongScheme => "wrong-scheme",
            Self::Retired => "retired-host",
            Self::Full => "delivery-capacity",
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn reject(reason: Rejection) {
    eprintln!(
        "WebUI: incoming URL activation rejected ({})",
        reason.reason()
    );
}

#[cfg(target_os = "macos")]
pub(crate) fn valid_scheme(scheme: &str) -> bool {
    (2..=32).contains(&scheme.len())
        && scheme.as_bytes()[0].is_ascii_lowercase()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !matches!(scheme, "http" | "https" | "file" | "webui")
}

#[cfg(target_os = "macos")]
fn encoded_control(bytes: &[u8]) -> bool {
    bytes.windows(3).any(|triplet| {
        if triplet[0] != b'%' {
            return false;
        }
        let (Some(hi), Some(lo)) = (
            (triplet[1] as char).to_digit(16),
            (triplet[2] as char).to_digit(16),
        ) else {
            return false;
        };
        let code = hi * 16 + lo;
        code < 32 || code == 127
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn validate(url: &str, scheme: &str) -> Result<UrlActivation, Rejection> {
    if url.len() > MAX_URL_ACTIVATION_BYTES {
        return Err(Rejection::TooLong);
    }
    if url.bytes().any(|byte| byte.is_ascii_control()) || encoded_control(url.as_bytes()) {
        return Err(Rejection::Control);
    }
    let parsed = url::Url::parse(url).map_err(|_| Rejection::InvalidUrl)?;
    if parsed.scheme() != scheme {
        return Err(Rejection::WrongScheme);
    }
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Rejection::InvalidUrl);
    }
    Ok(UrlActivation {
        window_id: WindowId::PRIMARY,
        url: url.to_string(),
    })
}

#[cfg(target_os = "macos")]
pub(crate) struct ActivationSender {
    scheme: String,
    sender: mpsc::SyncSender<UrlActivation>,
    closed: Arc<AtomicBool>,
    lifetime: HostLifetime,
}

#[cfg(target_os = "macos")]
impl ActivationSender {
    pub(crate) fn register(
        scheme: &str,
        lifetime: HostLifetime,
        handler: impl Fn(UrlActivation) + Send + 'static,
    ) -> Result<Self, UrlActivationRegistrationError> {
        if !valid_scheme(scheme) {
            return Err(UrlActivationRegistrationError::InvalidScheme);
        }
        if !lifetime.is_active() {
            return Err(UrlActivationRegistrationError::Closed);
        }
        let (sender, receiver) = mpsc::sync_channel(MAX_URL_ACTIVATIONS_PER_BATCH);
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = Arc::clone(&closed);
        let worker_lifetime = lifetime.clone();
        std::thread::Builder::new()
            .name("webui-url-activation".to_string())
            .spawn(move || {
                for activation in receiver {
                    if !worker_closed.load(Ordering::Acquire) && worker_lifetime.is_active() {
                        handler(activation);
                    }
                }
            })
            .map_err(|_| UrlActivationRegistrationError::Unavailable)?;
        Ok(Self {
            scheme: scheme.to_string(),
            sender,
            closed,
            lifetime,
        })
    }

    pub(crate) fn accept(&self, url: &str) -> Result<UrlActivation, Rejection> {
        if self.closed.load(Ordering::Acquire) || !self.lifetime.is_active() {
            return Err(Rejection::Retired);
        }
        validate(url, &self.scheme)
    }

    pub(crate) fn send(&self, activation: UrlActivation) {
        if self.closed.load(Ordering::Acquire) || !self.lifetime.is_active() {
            reject(Rejection::Retired);
        } else if self.sender.try_send(activation).is_err() {
            reject(Rejection::Full);
        }
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn scheme_and_url_validation_reject_unbounded_or_ambiguous_input() {
        assert!(valid_scheme("testapp"));
        for scheme in [
            "",
            "g",
            "HTTPS",
            "http",
            "webui",
            "testapp_",
            "g".repeat(33).as_str(),
        ] {
            assert!(!valid_scheme(scheme));
        }
        assert_eq!(
            validate("testapp://open/path?q=1", "testapp").unwrap(),
            UrlActivation {
                window_id: WindowId::PRIMARY,
                url: "testapp://open/path?q=1".into()
            }
        );
        for url in [
            "https://open/path",
            "testapp://",
            "testapp://user@open/path",
            "testapp://open/#fragment",
            "testapp://open/\n",
            "testapp://open/%0A",
            "testapp://open/%7f",
        ] {
            assert!(validate(url, "testapp").is_err());
        }
        assert!(matches!(
            validate(&format!("testapp://open/{}", "x".repeat(2048)), "testapp"),
            Err(Rejection::TooLong)
        ));
    }

    #[test]
    fn owner_retirement_blocks_delivery_even_with_a_registered_worker() {
        let (owner, lifetime) = HostLifetime::new();
        let sender = ActivationSender::register("testapp", lifetime.clone(), |_| {}).unwrap();
        assert!(sender.accept("testapp://open/before").is_ok());
        owner.revoke().unwrap();
        assert!(matches!(
            sender.accept("testapp://open/after"),
            Err(Rejection::Retired)
        ));
        assert!(matches!(
            ActivationSender::register("testapp", lifetime, |_| {}),
            Err(UrlActivationRegistrationError::Closed)
        ));
        sender.close();
    }
}
