// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-only WebView2 capture. The WebView2 interface is touched only by its
//! owning window STA; a bounded COM stream is shared with its completion.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use webview2_com::CapturePreviewCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Controller3,
    COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
use windows::core::{implement, Error, Interface, Ref, Result, HRESULT};
use windows::Win32::Foundation::{
    E_INVALIDARG, E_NOTIMPL, HWND, STG_E_INVALIDFUNCTION, STG_E_MEDIUMFULL,
};
use windows::Win32::System::Com::{
    ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE, STATFLAG, STATSTG, STGC, STGM_WRITE,
    STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::capture::{
    windows_png, CaptureError, CaptureOptions, CaptureState, MAX_WEB_CAPTURE_PNG_BYTES,
};

static NEXT_DISPATCH: AtomicUsize = AtomicUsize::new(1);
pub(super) const CAPTURE_WAKE_MESSAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 5;

pub(crate) struct Dispatch {
    hwnd: usize,
    id: usize,
    closed: AtomicBool,
    pending: Mutex<Option<(u64, u64, CaptureOptions)>>,
    stream: Mutex<Option<(u64, Weak<Mutex<Bytes>>)>>,
}

pub(super) struct Registration {
    dispatch: Arc<Dispatch>,
    owner: Arc<CaptureState>,
}

impl Registration {
    pub(super) fn owner_viewport_changed(&self) {
        self.owner.viewport_changed();
    }
    pub(super) fn close(&self) {
        self.dispatch.closed.store(true, Ordering::Release);
        self.dispatch.discard_stream(None);
        if let Ok(mut pending) = self.dispatch.pending.lock() {
            pending.take();
        }
        self.owner.notify_closed();
    }

    pub(super) fn drain(
        &self,
        cookie: usize,
        webview: &ICoreWebView2,
        controller: &ICoreWebView2Controller,
        (hwnd, content): (HWND, HWND),
    ) {
        if cookie != self.dispatch.id {
            return;
        }
        let pending = self
            .dispatch
            .pending
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some((id, epoch, options)) = pending {
            if self.dispatch.closed.load(Ordering::Acquire) || !self.owner.current(id, epoch) {
                self.owner.complete(id, epoch, Err(CaptureError::Cancelled));
            } else {
                start_capture(
                    &self.owner,
                    &self.dispatch,
                    (webview, controller),
                    (hwnd, content),
                    (id, epoch, options),
                );
            }
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn install(
    owner: &Arc<CaptureState>,
    hwnd: HWND,
) -> std::result::Result<Registration, CaptureError> {
    let id = NEXT_DISPATCH
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| CaptureError::Scheduler)?;
    let dispatch = Arc::new(Dispatch {
        hwnd: hwnd.0 as usize,
        id,
        closed: AtomicBool::new(false),
        pending: Mutex::new(None),
        stream: Mutex::new(None),
    });
    owner.attach(Arc::clone(&dispatch));
    Ok(Registration {
        dispatch,
        owner: Arc::clone(owner),
    })
}

impl Dispatch {
    pub(crate) fn discard_stream(&self, expected_id: Option<u64>) {
        let stream = self.stream.lock().ok().and_then(|slot| {
            slot.as_ref().and_then(|(id, weak)| {
                (expected_id.is_none() || expected_id == Some(*id))
                    .then(|| weak.upgrade())
                    .flatten()
            })
        });
        if let Some(stream) = stream {
            if let Ok(mut bytes) = stream.lock() {
                bytes.aborted = true;
                // Dropping the old allocation releases source bytes promptly
                // even when WebView2 never calls back.
                bytes.data = Vec::new();
            }
        }
    }

    fn track_stream(
        &self,
        id: u64,
        stream: &Arc<Mutex<Bytes>>,
    ) -> std::result::Result<(), CaptureError> {
        let mut slot = self.stream.lock().map_err(|_| CaptureError::Scheduler)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(CaptureError::Closed);
        }
        *slot = Some((id, Arc::downgrade(stream)));
        Ok(())
    }

    pub(crate) fn submit(
        &self,
        id: u64,
        epoch: u64,
        options: CaptureOptions,
    ) -> std::result::Result<(), CaptureError> {
        let mut pending = self.pending.lock().map_err(|_| CaptureError::Scheduler)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(CaptureError::Closed);
        }
        if pending.is_some() {
            return Err(CaptureError::Busy);
        }
        *pending = Some((id, epoch, options));
        // Keep the target lock until PostMessageW has accepted this wake. The
        // window generation is checked again before touching its COM object.
        let hwnd = HWND(self.hwnd as *mut c_void);
        // SAFETY: PostMessageW never waits for the UI thread; window teardown
        // closes this dispatch and an HWND reuse cannot match its cookie.
        if unsafe {
            PostMessageW(
                Some(hwnd),
                CAPTURE_WAKE_MESSAGE,
                windows::Win32::Foundation::WPARAM(self.id),
                windows::Win32::Foundation::LPARAM(0),
            )
        }
        .is_err()
        {
            pending.take();
            return Err(CaptureError::Scheduler);
        }
        Ok(())
    }
}

// A single bounded allocation for the native encoded output. All writes
// (including overwrites after Seek) consume the source-byte budget.
struct Bytes {
    data: Vec<u8>,
    position: usize,
    written: usize,
    limit: usize,
    overflow: bool,
    aborted: bool,
}

impl Bytes {
    fn new(limit: usize) -> Self {
        Self {
            data: Vec::new(),
            position: 0,
            written: 0,
            limit: limit.min(MAX_WEB_CAPTURE_PNG_BYTES),
            overflow: false,
            aborted: false,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> HRESULT {
        if self.aborted {
            return STG_E_INVALIDFUNCTION;
        }
        let Some(end) = self.position.checked_add(bytes.len()) else {
            self.overflow = true;
            return STG_E_MEDIUMFULL;
        };
        if end > self.limit
            || self
                .written
                .checked_add(bytes.len())
                .is_none_or(|n| n > self.limit)
        {
            self.overflow = true;
            return STG_E_MEDIUMFULL;
        }
        if end > self.data.len() {
            if end > self.data.capacity() {
                // Grow only when existing capacity is exhausted, and never
                // request more than the selected source/output byte limit.
                let target = end
                    .max(self.data.capacity().saturating_mul(2))
                    .max(4096)
                    .min(self.limit);
                if self
                    .data
                    .try_reserve_exact(target - self.data.len())
                    .is_err()
                {
                    self.overflow = true;
                    return STG_E_MEDIUMFULL;
                }
            }
            self.data.resize(end, 0);
        }
        self.data[self.position..end].copy_from_slice(bytes);
        self.position = end;
        self.written += bytes.len();
        HRESULT(0)
    }

    fn seek(&mut self, offset: i64, origin: STREAM_SEEK) -> Result<u64> {
        if self.aborted {
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        }
        let base = if origin == STREAM_SEEK_SET {
            0_i64
        } else if origin == STREAM_SEEK_CUR {
            i64::try_from(self.position).map_err(|_| Error::from(E_INVALIDARG))?
        } else if origin == STREAM_SEEK_END {
            i64::try_from(self.data.len()).map_err(|_| Error::from(E_INVALIDARG))?
        } else {
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        };
        let next = base
            .checked_add(offset)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| Error::from(STG_E_INVALIDFUNCTION))?;
        self.position = next;
        u64::try_from(next).map_err(|_| Error::from(E_INVALIDARG))
    }
}

#[implement(IStream)]
struct BoundedStream {
    bytes: Arc<Mutex<Bytes>>,
    owner: Weak<CaptureState>,
    id: u64,
    epoch: u64,
}

impl ISequentialStream_Impl for BoundedStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        if !pcbread.is_null() {
            // SAFETY: COM caller supplies the output pointer for this call.
            unsafe { *pcbread = 0 };
        }
        if cb > 0 && pv.is_null() {
            return E_INVALIDARG;
        }
        let Ok(mut bytes) = self.bytes.lock() else {
            return STG_E_INVALIDFUNCTION;
        };
        if bytes.aborted {
            return STG_E_INVALIDFUNCTION;
        }
        let available = bytes.data.len().saturating_sub(bytes.position);
        let count = available.min(cb as usize);
        if count > 0 {
            // SAFETY: pv points to at least cb writable bytes by IStream's
            // contract; count is <= cb and data bounds were checked above.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.data.as_ptr().add(bytes.position),
                    pv.cast::<u8>(),
                    count,
                );
            }
            bytes.position += count;
        }
        if !pcbread.is_null() {
            // SAFETY: count <= cb <= u32::MAX.
            unsafe { *pcbread = u32::try_from(count).unwrap_or(cb) };
        }
        HRESULT(0)
    }

    fn Write(&self, pv: *const c_void, cb: u32, pcbwritten: *mut u32) -> HRESULT {
        if !pcbwritten.is_null() {
            // SAFETY: COM caller supplies the output pointer for this call.
            unsafe { *pcbwritten = 0 };
        }
        if cb > 0 && pv.is_null() {
            return E_INVALIDARG;
        }
        let Ok(mut bytes) = self.bytes.lock() else {
            return STG_E_INVALIDFUNCTION;
        };
        if self
            .owner
            .upgrade()
            .is_none_or(|owner| !owner.current(self.id, self.epoch))
        {
            bytes.aborted = true;
            bytes.data = Vec::new();
            return STG_E_INVALIDFUNCTION;
        }
        // SAFETY: COM's ISequentialStream contract requires cb readable bytes
        // at pv. Zero-length writes use an empty slice without dereferencing pv.
        let input = if cb == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(pv.cast::<u8>(), cb as usize) }
        };
        let result = bytes.write(input);
        if result.is_ok() && !pcbwritten.is_null() {
            // SAFETY: Valid writable out-pointer supplied by COM.
            unsafe { *pcbwritten = cb };
        }
        result
    }
}

#[allow(clippy::unnecessary_wraps)]
impl IStream_Impl for BoundedStream_Impl {
    fn Seek(&self, offset: i64, origin: STREAM_SEEK, out: *mut u64) -> Result<()> {
        let position = self
            .bytes
            .lock()
            .map_err(|_| Error::from(STG_E_INVALIDFUNCTION))?
            .seek(offset, origin)?;
        if !out.is_null() {
            // SAFETY: COM supplied writable optional position storage.
            unsafe { *out = position };
        }
        Ok(())
    }
    fn SetSize(&self, size: u64) -> Result<()> {
        let mut bytes = self
            .bytes
            .lock()
            .map_err(|_| Error::from(STG_E_INVALIDFUNCTION))?;
        if self
            .owner
            .upgrade()
            .is_none_or(|owner| !owner.current(self.id, self.epoch))
        {
            bytes.aborted = true;
            bytes.data = Vec::new();
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        }
        if bytes.aborted {
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        }
        let size = usize::try_from(size).map_err(|_| Error::from(STG_E_MEDIUMFULL))?;
        if size > bytes.limit {
            bytes.overflow = true;
            return Err(Error::from(STG_E_MEDIUMFULL));
        }
        if size > bytes.data.len() {
            let additional = size - bytes.data.len();
            bytes
                .data
                .try_reserve_exact(additional)
                .map_err(|_| Error::from(STG_E_MEDIUMFULL))?;
        }
        bytes.data.resize(size, 0);
        bytes.position = bytes.position.min(size);
        Ok(())
    }
    fn CopyTo(&self, _: Ref<'_, IStream>, _: u64, _: *mut u64, _: *mut u64) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn Commit(&self, _: &STGC) -> Result<()> {
        Ok(())
    }
    fn Revert(&self) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn Stat(&self, out: *mut STATSTG, _: &STATFLAG) -> Result<()> {
        if out.is_null() {
            return Err(Error::from(E_INVALIDARG));
        }
        let bytes = self
            .bytes
            .lock()
            .map_err(|_| Error::from(STG_E_INVALIDFUNCTION))?;
        let stat = STATSTG {
            r#type: STGTY_STREAM.0 as u32,
            cbSize: u64::try_from(bytes.data.len()).map_err(|_| Error::from(E_INVALIDARG))?,
            grfMode: STGM_WRITE,
            ..Default::default()
        };
        // SAFETY: COM supplies writable STATSTG; the default name is null so
        // there is no ownership transfer or external allocation.
        unsafe { *out = stat };
        Ok(())
    }
    fn Clone(&self) -> Result<IStream> {
        Err(Error::from(E_NOTIMPL))
    }
}

fn start_capture(
    owner: &Arc<CaptureState>,
    dispatch: &Dispatch,
    (webview, controller): (&ICoreWebView2, &ICoreWebView2Controller),
    (hwnd, content): (HWND, HWND),
    (id, epoch, options): (u64, u64, CaptureOptions),
) {
    let mut rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: Called only on the live window's owning STA with its HWND.
    if unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd) }.as_bool()
        || !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd) }.as_bool()
    {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    }
    // SAFETY: content is the live controller parent, distinct from the frame
    // HWND used to route the wake and inspect window visibility.
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(content, &mut rect) }
        .is_err()
    {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    }
    let (Ok(width), Ok(height)) = (
        u32::try_from(rect.right.saturating_sub(rect.left)),
        u32::try_from(rect.bottom.saturating_sub(rect.top)),
    ) else {
        owner.complete(id, epoch, Err(CaptureError::Incomplete));
        return;
    };
    // The host sets controller.Bounds from this same content HWND. In
    // RAW_PIXELS mode Bounds is the physical extent even at high DPI; changing
    // RasterizationScale does not change it. Do not guess for logical Bounds,
    // an older controller, or a stale resize.
    let viewport = (|| {
        let controller3 = controller
            .cast::<ICoreWebView2Controller3>()
            .map_err(|_| CaptureError::Unavailable)?;
        let mut mode = Default::default();
        let mut bounds = windows::Win32::Foundation::RECT::default();
        // SAFETY: The controller, view and HWND belong to this live STA.
        unsafe {
            controller3
                .BoundsMode(&mut mode)
                .map_err(|_| CaptureError::Unavailable)?;
            controller
                .Bounds(&mut bounds)
                .map_err(|_| CaptureError::Unavailable)?;
        }
        if mode != COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS
            || bounds.left != 0
            || bounds.top != 0
            || bounds.right != rect.right
            || bounds.bottom != rect.bottom
        {
            return Err(CaptureError::Unavailable);
        }
        windows_png::preflight(width, height, options)
    })();
    let viewport = match viewport {
        Ok(viewport) => viewport,
        Err(error) => {
            owner.complete(id, epoch, Err(error));
            return;
        }
    };
    let bytes = Arc::new(Mutex::new(Bytes::new(options.max_png_bytes)));
    if let Err(error) = dispatch.track_stream(id, &bytes) {
        owner.complete(id, epoch, Err(error));
        return;
    }
    if !owner.current(id, epoch) {
        dispatch.discard_stream(Some(id));
        owner.complete(id, epoch, Err(CaptureError::Cancelled));
        return;
    }
    let stream: IStream = BoundedStream {
        bytes: Arc::clone(&bytes),
        owner: Arc::downgrade(owner),
        id,
        epoch,
    }
    .into();
    let completion_owner = Arc::clone(owner);
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        completion_owner.complete_if_current(id, epoch, || {
            let bytes = bytes.lock().map_err(|_| CaptureError::Scheduler)?;
            if bytes.overflow {
                return Err(CaptureError::TooLarge);
            }
            result.map_err(|error| CaptureError::NativeCode(error.code().0 as isize))?;
            let mut bytes = bytes;
            let png = std::mem::take(&mut bytes.data);
            let (width, height) = windows_png::validate_png(&png, viewport, options)?;
            Ok((width, height, png))
        });
        Ok(())
    }));
    // SAFETY: This runs on the exact controller's owning STA. WebView2 holds
    // its own COM references to stream and handler through completion; our
    // callback retains the buffer and capture reservation until invoked.
    if let Err(error) = unsafe {
        webview.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &handler,
        )
    } {
        owner.complete(
            id,
            epoch,
            Err(CaptureError::NativeCode(error.code().0 as isize)),
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn small_png_writes_reuse_capacity_through_retention() {
        // Valid 1x1 RGBA PNG, also used by the private macOS pasteboard test.
        let png = [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 1, 99, 248, 63, 139,
            225, 63, 0, 6, 206, 2, 153, 89, 149, 178, 136, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
            130,
        ];
        assert_eq!(png.len(), 70);
        let mut bytes = Bytes::new(MAX_WEB_CAPTURE_PNG_BYTES);
        assert!(bytes.write(&png[..5]).is_ok());
        let capacity = bytes.data.capacity();
        assert!((4096..=8192).contains(&capacity));
        for chunk in png[5..].chunks(5) {
            assert!(bytes.write(chunk).is_ok());
            assert_eq!(bytes.data.capacity(), capacity);
        }
        assert_eq!(bytes.written, png.len());
        let retained = Arc::new(bytes.data);
        assert_eq!(retained.as_slice(), png);
        assert_eq!(retained.capacity(), capacity);
    }

    #[test]
    fn stream_grows_for_real_capacity_exhaustion_not_rewrites() {
        let limit = 12 * 1024;
        let mut bytes = Bytes::new(limit);
        assert!(bytes.write(&[7; 4096]).is_ok());
        let initial = bytes.data.capacity();
        assert!(bytes
            .seek(i64::try_from(initial).unwrap(), STREAM_SEEK_SET)
            .is_ok());
        assert!(bytes.write(&[8]).is_ok());
        let grown = bytes.data.capacity();
        assert!(grown > initial && grown <= limit);
        assert_eq!(bytes.data[initial], 8);

        assert_eq!(bytes.seek(0, STREAM_SEEK_SET).unwrap(), 0);
        assert!(bytes.write(&[9; 16]).is_ok());
        assert_eq!(bytes.data.capacity(), grown);
        assert_eq!(&bytes.data[..16], &[9; 16]);

        assert!(bytes
            .seek(i64::try_from(grown).unwrap(), STREAM_SEEK_SET)
            .is_ok());
        assert!(bytes.write(&[10]).is_ok());
        assert!(bytes.data.capacity() <= limit);
        assert!(bytes.data[initial + 1..grown].iter().all(|byte| *byte == 0));
        assert_eq!(bytes.data[grown], 10);
        assert!(bytes
            .seek(i64::try_from(limit).unwrap(), STREAM_SEEK_SET)
            .is_ok());
        assert_eq!(bytes.write(&[11]).0, STG_E_MEDIUMFULL.0);
    }

    #[test]
    fn stream_caps_growth_rewrites_and_seek_without_allocating_past_limit() {
        let mut bytes = Bytes::new(32);
        assert!(bytes.write(&[7; 16]).is_ok());
        assert_eq!(bytes.seek(0, STREAM_SEEK_SET).unwrap(), 0);
        assert!(bytes.write(&[8; 16]).is_ok());
        assert_eq!(bytes.data.len(), 16);
        assert_eq!(bytes.write(&[9]).0, STG_E_MEDIUMFULL.0);
        assert!(bytes.overflow);
        assert_eq!(bytes.data.len(), 16);
        assert!(bytes.seek(33, STREAM_SEEK_SET).is_err());
        assert!(bytes.seek(-1, STREAM_SEEK_SET).is_err());
        bytes.aborted = true;
        bytes.data = Vec::new();
        assert_eq!(bytes.write(&[1]).0, STG_E_INVALIDFUNCTION.0);
        assert!(bytes.seek(0, STREAM_SEEK_SET).is_err());
        let mut bytes = Bytes::new(32);
        assert!(bytes.write(&[0; 32]).is_ok());
        assert_eq!(bytes.write(&[0]).0, STG_E_MEDIUMFULL.0);
    }

    #[test]
    fn stale_discard_cannot_cancel_a_newer_stream() {
        let dispatch = Dispatch {
            hwnd: 0,
            id: 1,
            closed: AtomicBool::new(false),
            pending: Mutex::new(None),
            stream: Mutex::new(None),
        };
        let bytes = Arc::new(Mutex::new(Bytes::new(64)));
        dispatch.track_stream(2, &bytes).unwrap();
        assert!(bytes.lock().unwrap().write(&[1; 32]).is_ok());
        dispatch.discard_stream(Some(1));
        assert_eq!(bytes.lock().unwrap().data.len(), 32);
        dispatch.discard_stream(Some(2));
        let mut state = bytes.lock().unwrap();
        assert!(state.aborted);
        assert!(state.data.is_empty());
        assert_eq!(state.write(&[1]).0, STG_E_INVALIDFUNCTION.0);
    }
}
