// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native PNG clipboard adapter. All Win32 calls run on a bounded worker,
//! never on WebView2's owning STA; the capture resource remains window-owned.

use std::ffi::c_void;
use std::sync::Arc;

use windows::core::{w, Owned};
use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL, HWND,
};
use windows::Win32::System::{DataExchange, Memory};

use crate::clipboard::{matches_logical_png, valid_png_bytes, ClipboardError, ClipboardState};

pub(crate) fn submit(
    owner: Arc<ClipboardState>,
    id: u64,
    window: usize,
) -> Result<(), ClipboardError> {
    if window == 0 {
        return Err(ClipboardError::Scheduler);
    }
    std::thread::Builder::new()
        .name("webui-clipboard-windows".into())
        .spawn(move || {
            let result = write_registered_png(&owner, id, window);
            // Only the native return releases the Busy reservation, even
            // after a logical deadline or a dropped Rust Future.
            owner.complete(id, result);
        })
        .map(|_| ())
        .map_err(|_| ClipboardError::Scheduler)
}

fn os_error(operation: &'static str) -> ClipboardError {
    // SAFETY: Read immediately after a failing Win32 call on this worker.
    ClipboardError::Os {
        operation,
        code: unsafe { GetLastError().0 },
    }
}

struct OpenClipboard;

impl OpenClipboard {
    fn close(self) -> Result<(), ClipboardError> {
        // A failed close cannot be treated as success even if readback
        // matched. Drop must not close a second time.
        let result =
            unsafe { DataExchange::CloseClipboard() }.map_err(|_| os_error("CloseClipboard"));
        std::mem::forget(self);
        result
    }
}

impl Drop for OpenClipboard {
    fn drop(&mut self) {
        // This path covers all errors before explicit close; the first error
        // remains authoritative, but failures to release the OS clipboard
        // cannot be silently ignored.
        if let Err(error) = unsafe { DataExchange::CloseClipboard() } {
            eprintln!("WebUI: failed to release the native clipboard after error: {error}");
        }
    }
}

/// GlobalUnlock returns zero both when the final lock is removed and when
/// it fails. Distinguish success with GetLastError after clearing the slot.
fn unlock(handle: HGLOBAL) -> Result<(), ClipboardError> {
    // SAFETY: The caller owns one successful GlobalLock for this valid handle.
    unsafe { SetLastError(ERROR_SUCCESS) };
    let unlocked = unsafe { Memory::GlobalUnlock(handle) };
    if unlocked.is_err() && unsafe { GetLastError() } != ERROR_SUCCESS {
        return Err(os_error("GlobalUnlock"));
    }
    Ok(())
}

fn copy_to_global(data: &[u8]) -> Result<Owned<HGLOBAL>, ClipboardError> {
    if !valid_png_bytes(data) {
        return Err(ClipboardError::InvalidData);
    }
    // SAFETY: GMEM_MOVEABLE is required by SetClipboardData. Ownership stays
    // in Owned until that call succeeds; all early errors free this allocation.
    let handle = unsafe { Memory::GlobalAlloc(Memory::GMEM_MOVEABLE, data.len()) }
        .map_err(|_| os_error("GlobalAlloc"))?;
    let owned = unsafe { Owned::new(handle) };
    let ptr = unsafe { Memory::GlobalLock(*owned) };
    if ptr.is_null() {
        return Err(os_error("GlobalLock"));
    }
    // SAFETY: GlobalAlloc reserved at least the checked data.len() bytes and
    // a successful lock pins the allocation until unlock. PNG is <=12 MiB.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u8>(), data.len()) };
    unlock(*owned)?;
    Ok(owned)
}

fn verify_readback(format: u32, png: &[u8]) -> Result<(), ClipboardError> {
    // SAFETY: Clipboard stays open on this worker throughout readback.
    let raw =
        unsafe { DataExchange::GetClipboardData(format) }.map_err(|_| ClipboardError::Readback)?;
    let handle = HGLOBAL(raw.0);
    let allocation = unsafe { Memory::GlobalSize(handle) };
    if allocation < png.len() {
        return Err(ClipboardError::Readback);
    }
    let ptr = unsafe { Memory::GlobalLock(handle) };
    if ptr.is_null() {
        return Err(os_error("GlobalLock(readback)"));
    }
    // SAFETY: GlobalSize proves this live clipboard-owned allocation has at
    // least png.len() bytes; compare only that logical length, not allocator
    // padding or arbitrary unbounded data from another clipboard writer.
    let observed = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), png.len()) };
    let matched = matches_logical_png(png, observed, allocation);
    unlock(handle)?;
    if matched {
        Ok(())
    } else {
        Err(ClipboardError::Readback)
    }
}

fn write_registered_png(
    owner: &ClipboardState,
    id: u64,
    window: usize,
) -> Result<(), ClipboardError> {
    let png = owner.payload(id)?;
    let memory = copy_to_global(&png)?;
    // SAFETY: The registered format is specifically PNG encoded bytes; CF_DIB
    // is not a replacement and could alter encoding/alpha fidelity.
    let format = unsafe { DataExchange::RegisterClipboardFormatW(w!("PNG")) };
    if format == 0 {
        return Err(os_error("RegisterClipboardFormatW"));
    }
    // SAFETY: The live owning HWND is supplied as clipboard owner. This worker
    // never dereferences it and cancellation is rechecked before EmptyClipboard.
    let hwnd = HWND(window as *mut c_void);
    unsafe { DataExchange::OpenClipboard(Some(hwnd)) }.map_err(|_| ClipboardError::Contended)?;
    let opened = OpenClipboard;
    let write = (|| {
        // Capture token and lifetime are rechecked at the final reversible
        // point; an OS write already admitted cannot be rolled back safely.
        owner.begin_write(id)?;
        unsafe { DataExchange::EmptyClipboard() }.map_err(|_| os_error("EmptyClipboard"))?;
        let raw = HANDLE(memory.0);
        // SAFETY: Ownership transfers to Windows only on success. On failure,
        // Owned drops and GlobalFree releases the moveable HGLOBAL.
        unsafe { DataExchange::SetClipboardData(format, Some(raw)) }
            .map_err(|_| ClipboardError::Rejected)?;
        std::mem::forget(memory);
        verify_readback(format, &png)
    })();
    let closed = opened.close();
    match (write, closed) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(primary), Err(close)) => {
            eprintln!("WebUI: native clipboard close also failed: {close}");
            Err(primary)
        }
    }
}
