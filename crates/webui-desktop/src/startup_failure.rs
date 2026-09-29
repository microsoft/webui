// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit host-owned failure presentation before any desktop frame exists.

/// Maximum UTF-8 byte length of the alert title.
pub const MAX_STARTUP_TITLE_BYTES: usize = 120;
/// Maximum UTF-8 byte length of the problem summary.
pub const MAX_STARTUP_SUMMARY_BYTES: usize = 500;
/// Maximum UTF-8 byte length of the actionable recovery help.
pub const MAX_STARTUP_HELP_BYTES: usize = 500;

/// Checked, host-authored text for one pre-frame startup failure.
///
/// Construct this from deliberately selected user-safe copy, not `Error::to_string`
/// or an error chain (which can expose paths, connection strings, or secrets).
/// The host remains responsible for durable diagnostics and error propagation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    title: String,
    summary: String,
    help: String,
}

/// Invalid text supplied to [`StartupFailure::new`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartupFailureError {
    /// A field is empty or contains only whitespace.
    #[error("{field} is empty; help: provide a short user-facing {field}")]
    Empty {
        /// The rejected field.
        field: &'static str,
    },
    /// A field exceeds its UTF-8 byte budget.
    #[error("{field} exceeds {max_bytes} UTF-8 bytes; help: shorten the {field}")]
    TooLong {
        /// The rejected field.
        field: &'static str,
        /// Maximum allowed bytes.
        max_bytes: usize,
    },
    /// A field contains a control or bidirectional formatting character.
    #[error("{field} contains a control or bidirectional formatting character; help: use plain single-line text")]
    UnsafeCharacter {
        /// The rejected field.
        field: &'static str,
    },
}

impl StartupFailure {
    /// Create a bounded, single-line title, summary, and recovery instruction.
    ///
    /// No error or source is implicitly converted to display text. The caller
    /// selects the exact copy users may see, including a concrete next step.
    pub fn new(
        title: impl AsRef<str>,
        summary: impl AsRef<str>,
        help: impl AsRef<str>,
    ) -> Result<Self, StartupFailureError> {
        let title = title.as_ref();
        let summary = summary.as_ref();
        let help = help.as_ref();
        check_text(title, "title", MAX_STARTUP_TITLE_BYTES)?;
        check_text(summary, "summary", MAX_STARTUP_SUMMARY_BYTES)?;
        check_text(help, "help", MAX_STARTUP_HELP_BYTES)?;
        Ok(Self {
            title: title.to_owned(),
            summary: summary.to_owned(),
            help: help.to_owned(),
        })
    }

    fn detail(&self) -> String {
        let mut detail = String::with_capacity(self.summary.len() + self.help.len() + 2);
        detail.push_str(&self.summary);
        detail.push_str("\n\n");
        detail.push_str(&self.help);
        detail
    }
}

#[cold]
#[inline(never)]
fn check_text(
    text: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), StartupFailureError> {
    if text.len() > max_bytes {
        return Err(StartupFailureError::TooLong { field, max_bytes });
    }
    if text.trim().is_empty() {
        return Err(StartupFailureError::Empty { field });
    }
    if text.chars().any(|c| {
        c.is_control()
            || matches!(
                c,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    }) {
        return Err(StartupFailureError::UnsafeCharacter { field });
    }
    Ok(())
}

/// A failure to present or acknowledge the native pre-frame alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartupPresentationError {
    /// No native presenter is implemented on this target.
    #[error("startup alert is unsupported on this platform; help: record and report the startup error through your host")]
    Unsupported,
    /// AppKit must be called on the process main thread.
    #[error("startup alert must run on the macOS main thread; help: present before starting background server work")]
    NotMainThread,
    /// No usable GUI display or application activation was available.
    #[error("startup alert has no available GUI session; help: retain the host's durable log and startup error")]
    Unavailable,
    /// The dialog stopped without its sole button being acknowledged.
    #[error("startup alert was dismissed without acknowledgement; help: retain the host's durable log and startup error")]
    NotAcknowledged,
}

/// Show one OS-owned, blocking startup error alert, before constructing a frame.
///
/// Returns `Ok(())` only after the user acknowledges the sole button. This
/// function does not create a WebView, log the root error, swallow it, or stop
/// the host: always preserve and propagate the original failure separately,
/// even when presentation returns an error. On macOS call from the main thread;
/// Windows may present from a host thread; Linux returns
/// [`StartupPresentationError::Unsupported`].
#[must_use = "handle native presentation failure separately from the original startup error"]
pub fn present_startup_failure(failure: &StartupFailure) -> Result<(), StartupPresentationError> {
    #[cfg(target_os = "macos")]
    {
        macos::present(failure)
    }
    #[cfg(windows)]
    {
        windows::present(failure)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = failure;
        Err(StartupPresentationError::Unsupported)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDOK, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };

    use super::{StartupFailure, StartupPresentationError};

    pub(super) fn present(failure: &StartupFailure) -> Result<(), StartupPresentationError> {
        let title = HSTRING::from(failure.title.as_str());
        let detail = HSTRING::from(failure.detail());
        // SAFETY: Both HSTRING buffers live through this synchronous Win32
        // call. The null owner is deliberate: no frame exists on this path.
        let response = unsafe {
            MessageBoxW(
                None,
                &detail,
                &title,
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            )
        };
        if response == IDOK {
            Ok(())
        } else if response.0 == 0 {
            Err(StartupPresentationError::Unavailable)
        } else {
            Err(StartupPresentationError::NotAcknowledged)
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication,
        NSApplicationActivationPolicy, NSScreen,
    };
    use objc2_foundation::NSString;

    use super::{StartupFailure, StartupPresentationError};

    pub(super) fn present(failure: &StartupFailure) -> Result<(), StartupPresentationError> {
        let mtm = MainThreadMarker::new().ok_or(StartupPresentationError::NotMainThread)?;
        if NSScreen::mainScreen(mtm).is_none() {
            return Err(StartupPresentationError::Unavailable);
        }
        let app = NSApplication::sharedApplication(mtm);
        if !app.setActivationPolicy(NSApplicationActivationPolicy::Regular) {
            return Err(StartupPresentationError::Unavailable);
        }
        app.finishLaunching();
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Critical);
        alert.setMessageText(&NSString::from_str(&failure.title));
        let detail = failure.detail();
        alert.setInformativeText(&NSString::from_str(&detail));
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        if alert.runModal() == NSAlertFirstButtonReturn {
            Ok(())
        } else {
            Err(StartupPresentationError::NotAcknowledged)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_bounded_explicit_copy() -> Result<(), StartupFailureError> {
        let failure = StartupFailure::new(
            "Unable to open",
            "The application could not start.",
            "Check the installation and try again.",
        )?;
        assert_eq!(failure.title, "Unable to open");
        assert_eq!(failure.help, "Check the installation and try again.");
        assert_eq!(
            failure.detail(),
            "The application could not start.\n\nCheck the installation and try again."
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_oversize_and_disguised_multiline_text() {
        assert_eq!(
            StartupFailure::new(" ", "Failed", "Try again"),
            Err(StartupFailureError::Empty { field: "title" })
        );
        assert_eq!(
            StartupFailure::new(
                "Failed",
                "x".repeat(MAX_STARTUP_SUMMARY_BYTES + 1),
                "Try again"
            ),
            Err(StartupFailureError::TooLong {
                field: "summary",
                max_bytes: MAX_STARTUP_SUMMARY_BYTES,
            })
        );
        assert_eq!(
            StartupFailure::new("é".repeat(61), "Failed", "Try again"),
            Err(StartupFailureError::TooLong {
                field: "title",
                max_bytes: MAX_STARTUP_TITLE_BYTES,
            })
        );
        assert_eq!(
            StartupFailure::new("Failed", "Failed", "x".repeat(MAX_STARTUP_HELP_BYTES + 1)),
            Err(StartupFailureError::TooLong {
                field: "help",
                max_bytes: MAX_STARTUP_HELP_BYTES,
            })
        );
        for unsafe_text in [
            "Try\nagain",
            "Try\ragain",
            "Try\u{0000}again",
            "Try\u{061c}again",
            "Try\u{200e}again",
            "Try\u{200f}again",
            "Try\u{202e}again",
            "Try\u{2069}again",
        ] {
            assert_eq!(
                StartupFailure::new("Failed", "Startup failed", unsafe_text),
                Err(StartupFailureError::UnsafeCharacter { field: "help" })
            );
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn unsupported_target_reports_error() -> Result<(), StartupFailureError> {
        let failure = StartupFailure::new("Failed", "Startup failed", "Try again")?;
        assert_eq!(
            present_startup_failure(&failure),
            Err(StartupPresentationError::Unsupported)
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_main_thread_reports_error_without_starting_appkit() -> Result<(), StartupFailureError> {
        let failure = StartupFailure::new("Failed", "Startup failed", "Try again")?;
        let result = std::thread::spawn(move || present_startup_failure(&failure)).join();
        assert!(matches!(
            result,
            Ok(Err(StartupPresentationError::NotMainThread))
        ));
        Ok(())
    }
}
