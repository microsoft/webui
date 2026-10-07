// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! One OS folder dialog per request on its own COM STA. The message-only
//! cancellation window belongs to that same STA, never WebView2's UI thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use webview2_com::CoTaskMemPWSTR;
use windows::core::{w, HRESULT, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM,
    FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    self, CreateWindowExW, DefWindowProcW, DestroyWindow, PostMessageW, RegisterClassW, WNDCLASSW,
};

use crate::native_services::{
    checked_directory, DirectoryPickerOptions, DirectorySelection, Inner, NativeServiceError,
    PickerPermit, Timer, MAX_NATIVE_DOCUMENT_PATH_BYTES,
};

const CANCEL_MESSAGE: u32 = WindowsAndMessaging::WM_APP + 23;
const CANCEL_HRESULT: HRESULT = HRESULT(0x800704C7_u32.cast_signed());
thread_local! {
    static TARGETS: RefCell<HashMap<usize, Rc<Target>>> = RefCell::new(HashMap::new());
}

struct Target {
    id: u64,
    cookie: usize,
    dialog: IFileOpenDialog,
    showing: Cell<bool>,
    signal: Arc<CancelSignal>,
}

pub(crate) struct CancelSignal {
    id: u64,
    cookie: usize,
    cancelled: AtomicBool,
    relay: AtomicUsize,
}

impl CancelSignal {
    pub(crate) fn new(id: u64, cookie: usize) -> Self {
        Self {
            id,
            cookie,
            cancelled: AtomicBool::new(false),
            relay: AtomicUsize::new(0),
        }
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let hwnd = self.relay.load(Ordering::Acquire);
        if hwnd != 0 {
            let Ok(id) = usize::try_from(self.id) else {
                eprintln!("WebUI: picker request ID is not representable by native wake");
                return;
            };
            // SAFETY: A payload-free wake to the worker's own message-only
            // HWND. The receiving STA verifies id and window generation.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(hwnd as *mut c_void)),
                    CANCEL_MESSAGE,
                    WPARAM(self.cookie),
                    LPARAM(id.cast_signed()),
                )
            };
        }
    }
}

pub(crate) fn cancel_picker(owner: &Inner, expected_id: Option<u64>) {
    let signal = owner
        .picker_signal
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(std::sync::Weak::upgrade));
    if let Some(signal) = signal {
        if expected_id.is_none_or(|id| id == signal.id) {
            signal.cancel();
        }
    }
}

struct SlotState {
    answer: Option<Result<DirectorySelection, NativeServiceError>>,
    waker: Option<Waker>,
}
pub(crate) struct PickerSlot(Mutex<SlotState>);
impl PickerSlot {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(SlotState {
            answer: None,
            waker: None,
        }))
    }
    pub(crate) fn complete(&self, answer: Result<DirectorySelection, NativeServiceError>) {
        let wake = if let Ok(mut state) = self.0.lock() {
            if state.answer.is_some() {
                return;
            }
            state.answer = Some(answer);
            state.waker.take()
        } else {
            None
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    pub(crate) fn poll(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<DirectorySelection, NativeServiceError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(NativeServiceError::Unavailable));
        };
        if let Some(answer) = state.answer.take() {
            return Poll::Ready(answer);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub(crate) struct PickerJob {
    pub(crate) permit: Arc<PickerPermit>,
    pub(crate) slot: Arc<PickerSlot>,
    pub(crate) signal: Arc<CancelSignal>,
    pub(crate) options: DirectoryPickerOptions,
    pub(crate) identity: (u64, u64),
    pub(crate) target: (usize, usize),
    pub(crate) deadline: Instant,
    pub(crate) timer: Arc<Timer>,
}

pub(crate) fn submit(owner: Arc<Inner>, job: PickerJob) -> Result<(), NativeServiceError> {
    std::thread::Builder::new()
        .name("webui-folder-picker-sta".into())
        .spawn(move || {
            let result = run(&owner, &job.signal, &job.options, job.identity, job.target);
            let result = if !owner.picker_current(job.identity.0, job.identity.1, job.target)
                || job.signal.cancelled.load(Ordering::Acquire)
            {
                Err(NativeServiceError::Cancelled)
            } else if Instant::now() >= job.deadline {
                Err(NativeServiceError::PickerTimeout)
            } else {
                result
            };
            // All COM objects and the relay have left scope before releasing
            // Busy and waking the host. No UI STA or host future is blocked.
            job.timer.finish();
            job.permit.release();
            job.slot.complete(result);
        })
        .map(|_| ())
        .map_err(|_| NativeServiceError::Unavailable)
}

fn native_failure(op: &'static str, error: windows::core::Error) -> NativeServiceError {
    NativeServiceError::PickerOs {
        operation: op,
        code: error.code().0,
    }
}

unsafe extern "system" fn relay_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == CANCEL_MESSAGE {
        let target = TARGETS.with(|targets| targets.borrow().get(&(hwnd.0 as usize)).cloned());
        if let Some(target) = target {
            if wp.0 == target.cookie
                && u64::try_from(lp.0.cast_unsigned()).ok() == Some(target.id)
                && target.showing.get()
                && target.signal.cancelled.load(Ordering::Acquire)
            {
                // SAFETY: This WndProc runs on the exact COM STA which created
                // the dialog. A stale or forged posted wake cannot close a
                // different dialog or reach a WebView2 COM interface.
                if let Err(error) = unsafe { target.dialog.Close(CANCEL_HRESULT) } {
                    // Close can lose a race with dialog teardown or fail
                    // before Show is modal. Never infer completion here:
                    // Busy stays reserved until Show really returns.
                    eprintln!(
                        "WebUI: IFileDialog::Close did not acknowledge cancellation (HRESULT {:#x}); waiting for native return",
                        error.code().0
                    );
                }
            }
        }
        return LRESULT(0);
    }
    // SAFETY: All other native messages belong to the system class handler.
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}

fn make_relay() -> Result<HWND, NativeServiceError> {
    // SAFETY: Module stays loaded during the worker; RegisterClassW returns
    // zero when this process already registered the same named class.
    let instance = unsafe {
        HINSTANCE(
            GetModuleHandleW(None)
                .map_err(|e| native_failure("GetModuleHandleW", e))?
                .0,
        )
    };
    let class = WNDCLASSW {
        lpfnWndProc: Some(relay_proc),
        hInstance: instance,
        lpszClassName: w!("WebUIPickerWorkerSTA"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    // SAFETY: HWND_MESSAGE creates a nonvisual window only on this COM STA.
    unsafe {
        CreateWindowExW(
            Default::default(),
            w!("WebUIPickerWorkerSTA"),
            w!(""),
            WindowsAndMessaging::WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(WindowsAndMessaging::HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|e| native_failure("CreateWindowExW", e))
}

fn selected_directory(dialog: &IFileOpenDialog) -> Result<PathBuf, NativeServiceError> {
    let item = unsafe { dialog.GetResult() }.map_err(|e| native_failure("GetResult", e))?;
    let raw = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
        .map_err(|e| native_failure("GetDisplayName", e))?;
    let _guard = CoTaskMemPWSTR::from(raw);
    if raw.is_null() {
        return Err(NativeServiceError::InvalidSelection);
    }
    let mut len = 0;
    // SAFETY: Shell allocated a NUL-terminated CoTaskMem UTF-16 buffer.
    // The checked bound plus terminator is inspected before any path copy.
    unsafe {
        while len <= MAX_NATIVE_DOCUMENT_PATH_BYTES && *raw.0.add(len) != 0 {
            len += 1;
        }
        if len == 0 || len > MAX_NATIVE_DOCUMENT_PATH_BYTES {
            return Err(NativeServiceError::InvalidSelection);
        }
        let path = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw.0, len)));
        checked_directory(&path, false)
    }
}

fn run(
    owner: &Inner,
    signal: &Arc<CancelSignal>,
    options: &DirectoryPickerOptions,
    (id, generation): (u64, u64),
    target: (usize, usize),
) -> Result<DirectorySelection, NativeServiceError> {
    if !owner.picker_current(id, generation, target) || signal.cancelled.load(Ordering::Acquire) {
        return Err(NativeServiceError::Cancelled);
    }
    // SAFETY: This worker owns a fresh COM apartment and uninitializes it
    // only after all dialog interfaces and HWND resources are released.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|e| native_failure("CoInitializeEx", e))?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: Paired with successful CoInitializeEx on this worker.
            unsafe { CoUninitialize() };
        }
    }
    let _apartment = Apartment;
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| native_failure("CoCreateInstance", e))?;
    let flags = unsafe { dialog.GetOptions() }.map_err(|e| native_failure("GetOptions", e))?;
    unsafe { dialog.SetOptions(flags | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST) }
        .map_err(|e| native_failure("SetOptions", e))?;
    if let Some(title) = options.title.as_deref() {
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        unsafe { dialog.SetTitle(PCWSTR(wide.as_ptr())) }
            .map_err(|e| native_failure("SetTitle", e))?;
    }
    if let Some(initial) = options.initial_directory.as_deref() {
        let path = checked_directory(initial, true)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
            .map_err(|e| native_failure("SHCreateItemFromParsingName", e))?;
        unsafe { dialog.SetFolder(&item) }.map_err(|e| native_failure("SetFolder", e))?;
    }
    let relay = make_relay()?;
    let live_hwnd =
        unsafe { WindowsAndMessaging::IsWindow(Some(HWND(target.0 as *mut c_void))) }.as_bool();
    if !owner.picker_current(id, generation, target)
        || signal.cancelled.load(Ordering::Acquire)
        || !live_hwnd
    {
        // SAFETY: This worker created the relay; no COM dialog is showing.
        let _ = unsafe { DestroyWindow(relay) };
        return Err(if !live_hwnd {
            NativeServiceError::Closed
        } else {
            NativeServiceError::Cancelled
        });
    }
    let picker = Rc::new(Target {
        id,
        cookie: target.1,
        dialog: dialog.clone(),
        showing: Cell::new(true),
        signal: Arc::clone(signal),
    });
    TARGETS.with(|targets| {
        targets
            .borrow_mut()
            .insert(relay.0 as usize, Rc::clone(&picker));
    });
    signal.relay.store(relay.0 as usize, Ordering::Release);
    if !owner.picker_current(id, generation, target) || signal.cancelled.load(Ordering::Acquire) {
        signal.cancel();
    }
    // SAFETY: The exact frame HWND is system-owned and not dereferenced.
    // Show runs on this worker's STA; its nested modal loop dispatches the
    // worker-owned cancellation relay, never a cross-apartment COM call.
    let result = unsafe { dialog.Show(Some(HWND(target.0 as *mut c_void))) };
    picker.showing.set(false);
    signal.relay.store(0, Ordering::Release);
    TARGETS.with(|targets| {
        targets.borrow_mut().remove(&(relay.0 as usize));
    });
    // SAFETY: The message-only HWND is destroyed on its creating STA after
    // Show exits; a late wake has no native COM target to access.
    if let Err(error) = unsafe { DestroyWindow(relay) } {
        return Err(native_failure("DestroyWindow", error));
    }
    match result {
        Ok(())
            if owner.picker_current(id, generation, target)
                && !signal.cancelled.load(Ordering::Acquire) =>
        {
            selected_directory(&dialog).map(DirectorySelection::Selected)
        }
        Ok(()) => Err(NativeServiceError::Cancelled),
        Err(error) if error.code() == CANCEL_HRESULT => Ok(DirectorySelection::Cancelled),
        Err(error) => Err(native_failure("IFileOpenDialog::Show", error)),
    }
}
