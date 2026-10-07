// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-owned native appearance. No renderer transport or persisted preference.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::NativeServiceError;

/// Appearance requested by the trusted Rust host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeMode {
    /// Use the platform's light appearance.
    Light,
    /// Use the platform's dark appearance.
    Dark,
    /// Inherit the current OS appearance and follow future changes.
    System,
}

/// The requested mode and the native window's current effective appearance.
/// The host is responsible for persisting its preference and rendering CSS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemeState {
    /// Most recently applied host preference.
    pub mode: ThemeMode,
    /// Whether this window currently has a dark effective appearance.
    pub dark: bool,
}

#[cfg(target_os = "macos")]
#[path = "macos/theme_adapter.rs"]
#[allow(unsafe_code)]
pub(crate) mod platform;

/// Awaitable native UI-thread acknowledgement, not merely queue admission.
#[must_use = "await the native appearance result; queue admission is not completion"]
pub struct ThemeRequest {
    #[cfg(target_os = "macos")]
    pub(crate) inner: platform::Request,
}

impl Future for ThemeRequest {
    type Output = Result<ThemeState, NativeServiceError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(target_os = "macos")]
        {
            Pin::new(&mut self.get_mut().inner).poll(cx)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (self, cx);
            Poll::Ready(Err(NativeServiceError::ThemeUnsupported))
        }
    }
}
