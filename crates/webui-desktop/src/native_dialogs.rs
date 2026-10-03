// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded host-authored copy and OS-independent button policy.

// The dialog API is uniform on every target so hosts compile unchanged, but only
// macOS and Windows have a native backend that reads the copy and admission
// machinery; elsewhere `NativeServices` answers `DialogError::Unsupported`.
#![cfg_attr(
    not(any(target_os = "macos", windows)),
    allow(dead_code, unused_imports, unused_variables)
)]

#[path = "native_dialogs/state.rs"]
pub(crate) mod state;

pub use state::DialogRequest;
pub(crate) use state::{DialogState, Signal};

pub const MAX_DIALOG_TITLE_BYTES: usize = 120;
pub const MAX_DIALOG_MESSAGE_BYTES: usize = 500;
pub const MAX_DIALOG_LABEL_BYTES: usize = 40;

#[derive(Clone, Debug)]
pub struct ErrorDialog {
    title: String,
    message: String,
    acknowledgement: String,
}

#[derive(Clone, Debug)]
pub struct ConfirmDialog {
    title: String,
    message: String,
    confirm: String,
    cancel: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogOutcome {
    Acknowledged,
    Confirmed,
    Cancelled,
}

#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DialogError {
    #[error("invalid dialog {field}; supply bounded single-line, host-authored text")]
    Invalid { field: &'static str },
    #[error("a native modal dialog is already active for this window")]
    Busy,
    #[error("navigation superseded the native dialog")]
    Navigated,
    #[error("the native dialog window or verified host closed")]
    Closed,
    #[error("native dialog did not acknowledge within ten seconds")]
    Timeout,
    #[error("native dialog {operation} failed with OS code {code}")]
    Os { operation: &'static str, code: i32 },
    #[error("native dialog dispatch is unavailable")]
    Unavailable,
    #[error("native dialogs are unsupported on this platform")]
    Unsupported,
}

#[cold]
#[inline(never)]
fn validate<'a>(value: &'a str, field: &'static str, max: usize) -> Result<&'a str, DialogError> {
    if value.is_empty()
        || value.trim().is_empty()
        || value.len() > max
        || value.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}'
                        | '\u{2066}'..='\u{206f}'
                )
        })
    {
        return Err(DialogError::Invalid { field });
    }
    Ok(value)
}

impl ErrorDialog {
    /// Explicit user-safe text, never implicitly converted from an error.
    pub fn new(title: &str, message: &str, acknowledgement: &str) -> Result<Self, DialogError> {
        Ok(Self {
            title: validate(title, "title", MAX_DIALOG_TITLE_BYTES)?.into(),
            message: validate(message, "message", MAX_DIALOG_MESSAGE_BYTES)?.into(),
            acknowledgement: validate(acknowledgement, "acknowledgement", MAX_DIALOG_LABEL_BYTES)?
                .into(),
        })
    }
}

impl ConfirmDialog {
    /// A host-authored question and two bounded button labels.
    pub fn new(
        title: &str,
        message: &str,
        confirm: &str,
        cancel: &str,
    ) -> Result<Self, DialogError> {
        Ok(Self {
            title: validate(title, "title", MAX_DIALOG_TITLE_BYTES)?.into(),
            message: validate(message, "message", MAX_DIALOG_MESSAGE_BYTES)?.into(),
            confirm: validate(confirm, "confirm", MAX_DIALOG_LABEL_BYTES)?.into(),
            cancel: validate(cancel, "cancel", MAX_DIALOG_LABEL_BYTES)?.into(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DialogCopy {
    Error(ErrorDialog),
    Confirm(ConfirmDialog),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DialogButton {
    Affirmative,
    Cancel,
}

impl DialogButton {
    #[cfg(any(windows, test))]
    pub(crate) fn native_id(self, affirmative: i32, cancel: i32) -> i32 {
        match self {
            Self::Affirmative => affirmative,
            Self::Cancel => cancel,
        }
    }
}

impl DialogCopy {
    pub(crate) fn title(&self) -> &str {
        match self {
            Self::Error(options) => &options.title,
            Self::Confirm(options) => &options.title,
        }
    }
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Error(options) => &options.message,
            Self::Confirm(options) => &options.message,
        }
    }
    pub(crate) fn label(&self, button: DialogButton) -> Option<&str> {
        match (self, button) {
            (Self::Error(options), DialogButton::Affirmative) => Some(&options.acknowledgement),
            (Self::Confirm(options), DialogButton::Affirmative) => Some(&options.confirm),
            (Self::Error(_), DialogButton::Cancel) => None,
            (Self::Confirm(options), DialogButton::Cancel) => Some(&options.cancel),
        }
    }
    #[cfg(any(windows, test))]
    pub(crate) fn default_button(&self) -> DialogButton {
        match self {
            Self::Error(_) => DialogButton::Affirmative,
            Self::Confirm(_) => DialogButton::Cancel,
        }
    }
    // The first NSAlert button is its Return-key default.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn mac_buttons(&self) -> &'static [DialogButton] {
        const ERROR: [DialogButton; 1] = [DialogButton::Affirmative];
        const CONFIRM: [DialogButton; 2] = [DialogButton::Cancel, DialogButton::Affirmative];
        match self {
            Self::Error(_) => &ERROR,
            Self::Confirm(_) => &CONFIRM,
        }
    }
    pub(crate) fn outcome(&self, button: DialogButton) -> DialogOutcome {
        match (self, button) {
            (_, DialogButton::Cancel) => DialogOutcome::Cancelled,
            (Self::Error(_), DialogButton::Affirmative) => DialogOutcome::Acknowledged,
            (Self::Confirm(_), DialogButton::Affirmative) => DialogOutcome::Confirmed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn bounded_host_copy_rejects_controls_bidi_and_unbounded_labels() {
        assert!(ErrorDialog::new("Unable to continue", "Try again.", "OK").is_ok());
        assert!(
            ConfirmDialog::new("Delete?", "This cannot be undone.", "Delete", "Cancel").is_ok()
        );
        assert!(matches!(
            ErrorDialog::new("Oops\nsecret", "Try again", "OK"),
            Err(DialogError::Invalid { field: "title" })
        ));
        assert!(ConfirmDialog::new("Continue?", "Yes", "A".repeat(41).as_str(), "No").is_err());
        assert!(ConfirmDialog::new("Continue?", "A\u{202e}B", "Yes", "No").is_err());
        assert!(ConfirmDialog::new("Continue?", "A\u{2028}B", "Yes", "No").is_err());
    }

    #[test]
    fn native_default_button_and_response_mapping_fail_closed_for_confirmation() {
        let confirm = DialogCopy::Confirm(
            ConfirmDialog::new(
                "Discard changes?",
                "Changes will be lost.",
                "Discard",
                "Keep editing",
            )
            .unwrap(),
        );
        assert_eq!(confirm.default_button(), DialogButton::Cancel);
        assert_eq!(confirm.default_button().native_id(100, 2), 2);
        assert_eq!(
            confirm.mac_buttons(),
            &[DialogButton::Cancel, DialogButton::Affirmative]
        );
        assert_eq!(
            confirm.label(confirm.mac_buttons()[0]),
            Some("Keep editing")
        );
        assert_eq!(
            confirm.outcome(confirm.mac_buttons()[0]),
            DialogOutcome::Cancelled
        );
        assert_eq!(
            confirm.outcome(confirm.mac_buttons()[1]),
            DialogOutcome::Confirmed
        );
        let error = DialogCopy::Error(ErrorDialog::new("Failed", "Try again.", "OK").unwrap());
        assert_eq!(error.default_button(), DialogButton::Affirmative);
        assert_eq!(error.default_button().native_id(100, 2), 100);
        assert_eq!(error.mac_buttons(), &[DialogButton::Affirmative]);
        assert_eq!(
            error.outcome(error.mac_buttons()[0]),
            DialogOutcome::Acknowledged
        );
        assert_eq!(error.label(DialogButton::Cancel), None);
        assert_eq!(
            error.outcome(DialogButton::Cancel),
            DialogOutcome::Cancelled
        );
    }
}
