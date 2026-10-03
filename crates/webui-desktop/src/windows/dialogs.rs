// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! TaskDialogIndirect runs on its own COM STA, not WebView2's frame STA.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON,
    TASKDIALOG_COMMON_BUTTON_FLAGS, TDF_ALLOW_DIALOG_CANCELLATION, TDF_CALLBACK_TIMER,
    TDF_POSITION_RELATIVE_TO_WINDOW, TDM_CLICK_BUTTON, TDN_CREATED, TDN_DESTROYED, TDN_TIMER,
    TD_ERROR_ICON, TD_WARNING_ICON,
};
use windows::Win32::UI::WindowsAndMessaging::{self, IDCANCEL};

use crate::native_dialogs::{
    DialogButton, DialogCopy, DialogError, DialogOutcome, DialogState, Signal,
};

const AFFIRM: i32 = 100;

pub(crate) fn submit(
    owner: Arc<DialogState>,
    (id, epoch): (u64, u64),
    copy: DialogCopy,
    signal: Arc<Signal>,
    window: usize,
) -> Result<(), DialogError> {
    if window == 0 {
        return Err(DialogError::Unavailable);
    }
    std::thread::Builder::new()
        .name("webui-modal-windows-sta".into())
        .spawn(move || {
            let result = show(&owner, (id, epoch), &copy, &signal, window);
            owner.complete(id, result);
        })
        .map(|_| ())
        .map_err(|_| DialogError::Unavailable)
}

struct Callback<'a> {
    owner: &'a DialogState,
    signal: &'a Signal,
    id: u64,
    epoch: u64,
    observed: Cell<Option<HWND>>,
    posted: Cell<bool>,
    cancel_message: u32,
    cancel_parameter: usize,
}

unsafe extern "system" fn dialog_callback(
    hwnd: HWND,
    event: windows::Win32::UI::Controls::TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    context: isize,
) -> HRESULT {
    if context == 0 {
        return HRESULT(0);
    }
    // SAFETY: TaskDialogIndirect calls this on its own STA while the stack
    // Callback in show() is live; the pointer is never stored by WebView2.
    let context = unsafe { &*(context as *const Callback<'_>) };
    if event == TDN_CREATED {
        context.observed.set(Some(hwnd));
    } else if event == TDN_DESTROYED {
        context.observed.set(None);
    } else if event == TDN_TIMER
        && (context.signal.cancelled.load(Ordering::Acquire)
            || !context.owner.current(context.id, context.epoch))
        && context.observed.get() == Some(hwnd)
        && !context.posted.get()
    {
        // SAFETY: Posting from this live dialog's own timer callback
        // avoids sending to a stale/reused HWND from another thread.
        if unsafe {
            WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                context.cancel_message,
                WPARAM(context.cancel_parameter),
                LPARAM(0),
            )
        }
        .is_ok()
        {
            context.posted.set(true);
        }
    }
    HRESULT(0)
}

fn show(
    owner: &DialogState,
    (id, epoch): (u64, u64),
    copy: &DialogCopy,
    signal: &Signal,
    window: usize,
) -> Result<DialogOutcome, DialogError> {
    if !owner.current(id, epoch) {
        return Err(DialogError::Navigated);
    }
    // SAFETY: This new worker exclusively owns its COM apartment. The
    // existing WebView2 STA continues pumping its own frame/window messages.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| DialogError::Os {
            operation: "CoInitializeEx",
            code: error.code().0,
        })?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: Paired with successful CoInitializeEx on this worker.
            unsafe { CoUninitialize() };
        }
    }
    let _apartment = Apartment;
    let hwnd = HWND(window as *mut c_void);
    // SAFETY: Checking the exact owner HWND does not dereference it. Close
    // races fail closed through DialogState's generation/lifetime validation.
    if !unsafe { WindowsAndMessaging::IsWindow(Some(hwnd)) }.as_bool() || !owner.current(id, epoch)
    {
        return Err(DialogError::Closed);
    }
    let title = HSTRING::from(copy.title());
    let message = HSTRING::from(copy.message());
    let affirmative = HSTRING::from(
        copy.label(DialogButton::Affirmative)
            .ok_or(DialogError::Unavailable)?,
    );
    let cancel = copy.label(DialogButton::Cancel).map(HSTRING::from);
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: AFFIRM,
            pszButtonText: PCWSTR(affirmative.as_ptr()),
        },
        TASKDIALOG_BUTTON {
            nButtonID: IDCANCEL.0,
            pszButtonText: cancel
                .as_ref()
                .map_or(PCWSTR::null(), |label| PCWSTR(label.as_ptr())),
        },
    ];
    let callback = Callback {
        owner,
        signal,
        id,
        epoch,
        observed: Cell::new(None),
        posted: Cell::new(false),
        // TDF_ALLOW_DIALOG_CANCELLATION documents WM_CLOSE without a
        // visible Cancel button. Confirmations click the actual Cancel ID.
        cancel_message: if cancel.is_some() {
            TDM_CLICK_BUTTON.0.cast_unsigned()
        } else {
            WindowsAndMessaging::WM_CLOSE
        },
        cancel_parameter: if cancel.is_some() { 2 } else { 0 },
    };
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>())
            .map_err(|_| DialogError::Unavailable)?,
        hwndParent: hwnd,
        dwFlags: TDF_CALLBACK_TIMER
            | TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: TASKDIALOG_COMMON_BUTTON_FLAGS(0),
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(message.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: if matches!(copy, DialogCopy::Error(_)) {
                TD_ERROR_ICON
            } else {
                TD_WARNING_ICON
            },
        },
        cButtons: if cancel.is_some() { 2 } else { 1 },
        pButtons: buttons.as_ptr(),
        nDefaultButton: copy.default_button().native_id(AFFIRM, IDCANCEL.0),
        pfCallback: Some(dialog_callback),
        lpCallbackData: std::ptr::from_ref(&callback) as isize,
        ..Default::default()
    };
    let mut button = 0;
    // SAFETY: Config and all HSTRING/button/callback buffers remain live
    // through the synchronous native modal loop on this worker STA.
    unsafe { TaskDialogIndirect(&config, Some(&mut button), None, None) }.map_err(|error| {
        DialogError::Os {
            operation: "TaskDialogIndirect",
            code: error.code().0,
        }
    })?;
    if button == AFFIRM {
        Ok(copy.outcome(DialogButton::Affirmative))
    } else if button == IDCANCEL.0 || button == 0 {
        Ok(copy.outcome(DialogButton::Cancel))
    } else {
        Err(DialogError::Os {
            operation: "TaskDialogIndirect(response)",
            code: button,
        })
    }
}
