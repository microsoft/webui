// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::wire::WireError;

macro_rules! error_codes {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// Stable machine-readable failure category.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum IpcErrorCode { $(#[doc = $name] $variant),+ }
        impl IpcErrorCode {
            /// Wire spelling.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
            /// Unknown peer codes fail as a transport error.
            pub fn from_wire(value: &str) -> Self {
                match value { $($name => Self::$variant,)+ _ => Self::Transport }
            }
        }
    }
}
error_codes! {
    InvalidFrame => "invalid-frame", InvalidPayload => "invalid-payload",
    PayloadTooLarge => "payload-too-large", UnsupportedVersion => "unsupported-version",
    SchemaMismatch => "schema-mismatch", PermissionDenied => "permission-denied",
    UnknownMethod => "unknown-method", ReceiverUnavailable => "receiver-unavailable",
    NotReady => "not-ready", Overloaded => "overloaded", Cancelled => "cancelled",
    DeadlineExceeded => "deadline-exceeded", Navigated => "navigated", Closed => "closed",
    Transport => "transport", Handler => "handler"
}

/// Bounded plain-text diagnostic. Application stacks never cross the transport.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}; help: {help}")]
pub struct IpcError {
    /// Stable category.
    pub code: IpcErrorCode,
    /// Human-readable description.
    pub message: String,
    /// Actionable recovery guidance.
    pub help: String,
    /// Optional application-specific code.
    pub application_code: Option<String>,
}

impl IpcError {
    /// Build a diagnostic; wire conversion additionally enforces configured bounds.
    #[cold]
    #[inline(never)]
    pub fn new(code: IpcErrorCode, message: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            help: help.into(),
            application_code: None,
        }
    }

    pub(super) fn wire(self, budget: usize) -> WireError {
        let mut remaining = budget.saturating_sub(self.code.as_str().len());
        WireError {
            code: self.code.as_str().into(),
            message: bounded(self.message, &mut remaining),
            help: bounded(self.help, &mut remaining),
            application_code: bounded(self.application_code.unwrap_or_default(), &mut remaining),
        }
    }

    pub(super) fn from_wire(value: WireError, budget: usize) -> Self {
        let code = IpcErrorCode::from_wire(&value.code);
        let mut remaining = budget.saturating_sub(code.as_str().len());
        let message = bounded(value.message, &mut remaining);
        let help = bounded(value.help, &mut remaining);
        let application_code = bounded(value.application_code, &mut remaining);
        Self {
            code,
            message,
            help,
            application_code: (!application_code.is_empty()).then_some(application_code),
        }
    }
}

fn bounded(mut text: String, remaining: &mut usize) -> String {
    let mut end = text.len().min(*remaining);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    *remaining -= end;
    text
}

#[cold]
#[inline(never)]
pub(super) fn fail(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        code.as_str(),
        match code {
            IpcErrorCode::Overloaded => {
                "reduce concurrent IPC work and wait for capacity before sending again"
            }
            IpcErrorCode::SchemaMismatch | IpcErrorCode::UnsupportedVersion => {
                "regenerate and deploy matching host and renderer bindings together"
            }
            IpcErrorCode::PermissionDenied => {
                "use the admitted main document and explicitly grant this method on the host"
            }
            IpcErrorCode::InvalidFrame | IpcErrorCode::InvalidPayload => {
                "send the generated protobuf type and wire-v2 envelope"
            }
            IpcErrorCode::PayloadTooLarge => {
                "send a smaller message within the configured frame and collection limits"
            }
            IpcErrorCode::NotReady | IpcErrorCode::Navigated => {
                "wait for a newly admitted document and acquire its session"
            }
            IpcErrorCode::UnknownMethod | IpcErrorCode::ReceiverUnavailable => {
                "install the generated receiver before sending this method"
            }
            IpcErrorCode::DeadlineExceeded => {
                "use cancellable asynchronous work and an appropriate bounded timeout"
            }
            IpcErrorCode::Cancelled => {
                "start a new call only if the application operation can safely be repeated"
            }
            IpcErrorCode::Closed => "create a new frame; closed sessions cannot reconnect",
            IpcErrorCode::Transport => "check the native IPC adapter and platform support",
            IpcErrorCode::Handler => {
                "handle the application failure without sending private exception details"
            }
        },
    )
}
