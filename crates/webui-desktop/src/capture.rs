// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Trusted-host, per-window native web-content capture. No renderer bridge is installed.

use std::future::Future;
use std::pin::Pin;
#[cfg(any(target_os = "macos", windows))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(target_os = "macos", windows))]
use std::sync::{Arc, Condvar, Mutex, Weak};
#[cfg(any(target_os = "macos", windows))]
use std::task::Waker;
use std::task::{Context, Poll};
#[cfg(any(target_os = "macos", windows))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "macos", windows))]
const CAPTURE_DEADLINE: Duration = Duration::from_secs(10);
#[cfg(any(target_os = "macos", windows))]
const MAX_CAPTURE_DEADLINE_THREADS_PER_WINDOW: usize = 8;

/// Hard maximum final raster width, measured in pixels after backing scale.
pub const MAX_WEB_CAPTURE_WIDTH: u32 = 1600;
/// Hard maximum final raster height, measured in pixels after backing scale.
pub const MAX_WEB_CAPTURE_HEIGHT: u32 = 1200;
/// Maximum RGBA raster storage, before PNG encoding.
pub const MAX_WEB_CAPTURE_RASTER_BYTES: usize = 1600 * 1200 * 4;
/// Maximum retained encoded PNG, below the SDK's 16 MiB IPC retained budget.
pub const MAX_WEB_CAPTURE_PNG_BYTES: usize = 12 * 1024 * 1024;
/// One native binary chunk returned to a trusted host for paced IPC delivery.
pub const MAX_WEB_CAPTURE_CHUNK_BYTES: usize = 20 * 1024;

// Pure Windows policy is exercised on macOS too; COM and HWND remain in the
// Windows adapter.
#[cfg(all(feature = "native-capture", any(windows, test)))]
pub(crate) mod windows_png;

/// Host-chosen, validated limits on one visible-content snapshot. macOS
/// downscales the whole viewport; Windows rejects oversized viewports because
/// WebView2 CapturePreview cannot downsample. Neither adapter crops or upscales.
#[derive(Clone, Copy, Debug)]
pub struct CaptureOptions {
    pub(crate) max_width: u32,
    pub(crate) max_height: u32,
    pub(crate) max_png_bytes: usize,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            max_width: MAX_WEB_CAPTURE_WIDTH,
            max_height: MAX_WEB_CAPTURE_HEIGHT,
            max_png_bytes: MAX_WEB_CAPTURE_PNG_BYTES,
        }
    }
}

impl CaptureOptions {
    /// Default full-viewport capture with SDK-owned hard limits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set final pixel bounds, at most 1600×1200; no crop or upscaling.
    /// Windows rejects a viewport above these bounds before native capture.
    ///
    /// # Errors
    ///
    /// Rejects zero or limits exceeding the SDK raster bound.
    pub fn max_dimensions(mut self, width: u32, height: u32) -> Result<Self, CaptureError> {
        if width == 0
            || height == 0
            || width > MAX_WEB_CAPTURE_WIDTH
            || height > MAX_WEB_CAPTURE_HEIGHT
        {
            return Err(CaptureError::InvalidOptions);
        }
        self.max_width = width;
        self.max_height = height;
        Ok(self)
    }

    /// Set the maximum retained PNG bytes (24 bytes to 12 MiB).
    ///
    /// # Errors
    ///
    /// Rejects an unusable or over-budget encoded size.
    pub fn max_png_bytes(mut self, bytes: usize) -> Result<Self, CaptureError> {
        if !(24..=MAX_WEB_CAPTURE_PNG_BYTES).contains(&bytes) {
            return Err(CaptureError::InvalidOptions);
        }
        self.max_png_bytes = bytes;
        Ok(self)
    }
}

/// Capture admission, native callback, or resource-read error.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// The options exceed SDK memory or raster limits.
    #[error("capture bounds must be nonzero and within 1600×1200 pixels and 12 MiB PNG")]
    InvalidOptions,
    /// An earlier native callback is still in flight, including a cancelled one.
    #[error("a native content snapshot is already in flight on this window")]
    Busy,
    /// The native view has no finished and attached main document.
    #[error("visible web content is unavailable before a finished main document")]
    Unavailable,
    /// The window or owning verified host has retired.
    #[error("the capture window or verified host has closed")]
    Closed,
    /// A navigation or dropped request invalidated a pending snapshot.
    #[error("content snapshot was superseded by navigation or cancellation")]
    Cancelled,
    /// The native webview did not acknowledge the snapshot before its deadline.
    #[error("native content snapshot did not complete within ten seconds")]
    Timeout,
    /// The opaque resource was released, retaken, or belongs to another window.
    #[error("captured content is no longer retained by this window")]
    Released,
    /// Offset lies beyond the retained PNG.
    #[error("PNG read offset exceeds the encoded content length")]
    InvalidOffset,
    /// The native webview returned no complete viewport image.
    #[error("native webview did not return a complete visible viewport image")]
    Incomplete,
    /// Raster dimensions, raw bytes, or PNG size exceed checked bounds.
    #[error("native content snapshot exceeds the configured raster or PNG budget")]
    TooLarge,
    /// Native rendering or PNG conversion failed.
    #[error("native content capture failed: {0}")]
    Native(String),
    /// Numeric native error; page-provided diagnostic text is never allocated.
    #[error("native content snapshot failed with error code {0}")]
    NativeCode(isize),
    /// The native adapter cannot schedule this owning-thread request.
    #[error("native content capture scheduler is unavailable")]
    Scheduler,
    /// Earlier capture deadline workers have not finished exiting.
    #[error("native capture deadline capacity is exhausted; retry after pending work settles")]
    Overloaded,
    /// No native capture adapter exists on this platform.
    #[error("visible web-content capture is unsupported on this platform")]
    Unsupported,
}

/// Opaque metadata for at most one retained native PNG per window. It does
/// not own an extra byte copy, and its identity cannot target another window.
// Other adapters return Unsupported, so their opaque identity is never
// constructed or inspected on those targets.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
#[derive(Debug)]
pub struct CapturedContent {
    window_generation: u64,
    id: u64,
    epoch: u64,
    revision: u64,
    /// Final encoded raster width in physical pixels.
    pub width: u32,
    /// Final encoded raster height in physical pixels.
    pub height: u32,
    /// Number of bytes available through bounded chunk reads.
    pub png_bytes: usize,
}

#[cfg(all(any(target_os = "macos", windows), feature = "native-clipboard"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CaptureToken {
    pub(crate) window_generation: u64,
    pub(crate) id: u64,
    pub(crate) epoch: u64,
    pub(crate) revision: u64,
}

#[cfg(all(any(target_os = "macos", windows), feature = "native-clipboard"))]
impl CapturedContent {
    pub(crate) fn token(&self) -> CaptureToken {
        CaptureToken {
            window_generation: self.window_generation,
            id: self.id,
            epoch: self.epoch,
            revision: self.revision,
        }
    }
}

/// A bounded native PNG read. Generated IPC methods remain an explicit host
/// choice and must credit one chunk at a time within their aggregate budgets.
#[derive(Debug)]
pub struct CapturedContentChunk {
    /// Binary PNG bytes, never more than 20 KiB.
    pub bytes: Vec<u8>,
    /// Offset for the next read.
    pub next_offset: usize,
    /// True when all PNG bytes have been returned.
    pub eof: bool,
}

#[cfg(any(target_os = "macos", windows))]
struct Slot(Mutex<SlotState>);

#[cfg(any(target_os = "macos", windows))]
struct SlotState {
    result: Option<Result<CapturedContent, CaptureError>>,
    waker: Option<Waker>,
}

#[cfg(any(target_os = "macos", windows))]
impl Slot {
    fn complete(&self, result: Result<CapturedContent, CaptureError>) {
        let wake = if let Ok(mut state) = self.0.lock() {
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            state.waker.take()
        } else {
            None
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<CapturedContent, CaptureError>> {
        let Ok(mut state) = self.0.lock() else {
            return Poll::Ready(Err(CaptureError::Scheduler));
        };
        if let Some(result) = state.result.take() {
            return Poll::Ready(result);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(any(target_os = "macos", windows))]
struct Active {
    id: u64,
    epoch: u64,
    revision: u64,
    options: CaptureOptions,
    abandoned: bool,
    timed_out: bool,
    deadline: Instant,
    timer: Arc<DeadlineTimer>,
    slot: Weak<Slot>,
}

#[cfg(any(target_os = "macos", windows))]
struct DeadlineTimer {
    finished: Mutex<bool>,
    changed: Condvar,
}

#[cfg(any(target_os = "macos", windows))]
impl DeadlineTimer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            finished: Mutex::new(false),
            changed: Condvar::new(),
        })
    }

    fn finish(&self) {
        if let Ok(mut finished) = self.finished.lock() {
            *finished = true;
            self.changed.notify_one();
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
struct TimerReservation(Arc<CaptureState>);

#[cfg(any(target_os = "macos", windows))]
impl Drop for TimerReservation {
    fn drop(&mut self) {
        self.0.active_timer_threads.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(any(target_os = "macos", windows))]
struct Retained {
    id: u64,
    epoch: u64,
    revision: u64,
    // Intentionally retain the encoder's exact-sized Vec allocation by
    // moving only its header into Arc; Arc<[u8]>::from(Vec) copies the
    // entire (potentially multi-MiB) PNG at this boundary.
    #[allow(clippy::rc_buffer)]
    png: Arc<Vec<u8>>,
}

#[cfg(any(target_os = "macos", windows))]
struct State {
    epoch: u64,
    revision: u64,
    finished: bool,
    closed: bool,
    next_id: u64,
    active: Option<Active>,
    retained: Option<Retained>,
}

/// A private, bounded resource owner for one exact native window generation.
#[cfg(any(target_os = "macos", windows))]
pub(crate) struct CaptureState {
    window_generation: u64,
    lifetime: crate::HostLifetime,
    active_timer_threads: AtomicUsize,
    state: Mutex<State>,
    #[cfg(target_os = "macos")]
    dispatch: Mutex<Option<Arc<crate::macos::capture::Dispatch>>>,
    #[cfg(windows)]
    dispatch: Mutex<Option<Arc<crate::windows::capture::Dispatch>>>,
}

#[cfg(any(target_os = "macos", windows))]
impl CaptureState {
    pub(crate) fn new(lifetime: crate::HostLifetime, window_generation: u64) -> Arc<Self> {
        Arc::new(Self {
            window_generation,
            lifetime,
            active_timer_threads: AtomicUsize::new(0),
            state: Mutex::new(State {
                epoch: 0,
                revision: 0,
                finished: false,
                closed: false,
                next_id: 0,
                active: None,
                retained: None,
            }),
            dispatch: Mutex::new(None),
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn attach(&self, dispatch: Arc<crate::macos::capture::Dispatch>) {
        if let Ok(mut attached) = self.dispatch.lock() {
            *attached = Some(dispatch);
        }
    }

    #[cfg(windows)]
    pub(crate) fn attach(&self, dispatch: Arc<crate::windows::capture::Dispatch>) {
        if let Ok(mut attached) = self.dispatch.lock() {
            *attached = Some(dispatch);
        }
    }

    #[cfg(all(test, target_os = "macos", feature = "native-capture"))]
    pub(crate) fn test_store_retained(&self, bytes: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.epoch = 1;
            state.finished = true;
            state.retained = Some(Retained {
                id: 1,
                epoch: 1,
                revision: 0,
                png: Arc::new(vec![0; bytes]),
            });
        }
    }

    #[cfg(all(test, target_os = "macos", feature = "native-clipboard"))]
    pub(crate) fn test_store_retained_png(&self, png: &[u8]) {
        self.test_store_retained(png.len());
        if let Ok(mut state) = self.state.lock() {
            if let Some(retained) = state.retained.as_mut() {
                retained.png = Arc::new(png.to_vec());
            }
        }
    }

    #[cfg(all(test, target_os = "macos", feature = "native-capture"))]
    pub(crate) fn test_retained_len(&self) -> usize {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.retained.as_ref().map(|retained| retained.png.len()))
            .unwrap_or(0)
    }

    #[cfg(all(test, windows))]
    pub(crate) fn test_finished(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.finished)
    }

    #[cfg(all(test, target_os = "macos", feature = "native-clipboard"))]
    pub(crate) fn test_content(&self) -> Option<CapturedContent> {
        self.state.lock().ok().and_then(|state| {
            state.retained.as_ref().map(|retained| CapturedContent {
                window_generation: self.window_generation,
                id: retained.id,
                epoch: retained.epoch,
                revision: retained.revision,
                width: 1,
                height: 1,
                png_bytes: retained.png.len(),
            })
        })
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        options: CaptureOptions,
    ) -> Result<CaptureRequest, CaptureError> {
        if !self.lifetime.is_active() {
            self.close();
            return Err(CaptureError::Closed);
        }
        let dispatch = self
            .dispatch
            .lock()
            .map_err(|_| CaptureError::Scheduler)?
            .as_ref()
            .cloned()
            .ok_or(CaptureError::Unavailable)?;
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        let reservation = self.reserve_timer()?;
        let deadline = Instant::now() + CAPTURE_DEADLINE;
        let timer = DeadlineTimer::new();
        let (id, epoch) = {
            let mut state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
            if state.closed {
                return Err(CaptureError::Closed);
            }
            if !state.finished || state.epoch == 0 {
                return Err(CaptureError::Unavailable);
            }
            if state.active.is_some() {
                return Err(CaptureError::Busy);
            }
            let id = state
                .next_id
                .checked_add(1)
                .ok_or(CaptureError::Scheduler)?;
            state.next_id = id;
            state.retained = None;
            state.active = Some(Active {
                id,
                epoch: state.epoch,
                revision: state.revision,
                options,
                abandoned: false,
                timed_out: false,
                deadline,
                timer: Arc::clone(&timer),
                slot: Arc::downgrade(&slot),
            });
            (id, state.epoch)
        };
        if let Err(error) = Self::start_deadline(&timer, id, epoch, deadline, reservation) {
            self.abort_unsubmitted(id);
            return Err(error);
        }
        if let Err(error) = dispatch.submit(id, epoch, options) {
            self.abort_unsubmitted(id);
            return Err(error);
        }
        Ok(CaptureRequest {
            owner: Arc::clone(self),
            slot,
            id,
            epoch,
            deadline,
            delivered: false,
        })
    }

    fn reserve_timer(self: &Arc<Self>) -> Result<TimerReservation, CaptureError> {
        self.active_timer_threads
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_CAPTURE_DEADLINE_THREADS_PER_WINDOW).then_some(count + 1)
            })
            .map_err(|_| CaptureError::Overloaded)?;
        Ok(TimerReservation(Arc::clone(self)))
    }

    fn start_deadline(
        timer: &Arc<DeadlineTimer>,
        id: u64,
        epoch: u64,
        deadline: Instant,
        reservation: TimerReservation,
    ) -> Result<(), CaptureError> {
        let timer = Arc::clone(timer);
        let weak_owner = Arc::downgrade(&reservation.0);
        std::thread::Builder::new()
            .name("webui-capture-deadline".into())
            .spawn(move || {
                let _reservation = reservation;
                let Ok(finished) = timer.finished.lock() else {
                    return;
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                let Ok((finished, _)) =
                    timer
                        .changed
                        .wait_timeout_while(finished, remaining, |finished| !*finished)
                else {
                    return;
                };
                if !*finished {
                    drop(finished);
                    if let Some(owner) = weak_owner.upgrade() {
                        owner.timeout(id, epoch);
                    }
                }
            })
            .map(|_| ())
            .map_err(|_| CaptureError::Scheduler)
    }

    fn abort_unsubmitted(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state.active.as_ref().is_some_and(|active| active.id == id) {
                if let Some(active) = state.active.take() {
                    active.timer.finish();
                }
            }
        }
    }

    fn timeout(&self, id: u64, epoch: u64) {
        let slot = if let Ok(mut state) = self.state.lock() {
            let Some(active) = state
                .active
                .as_mut()
                .filter(|active| active.id == id && active.epoch == epoch)
            else {
                return;
            };
            active.timed_out = true;
            active.slot.upgrade()
        } else {
            None
        };
        #[cfg(windows)]
        self.discard_windows_stream(Some(id));
        if let Some(slot) = slot {
            slot.complete(Err(CaptureError::Timeout));
        }
        // Keep the native reservation until the native API actually calls back.
    }

    pub(crate) fn current(&self, id: u64, epoch: u64) -> bool {
        self.lifetime.is_active()
            && self.state.lock().is_ok_and(|state| {
                !state.closed
                    && state.finished
                    && state.epoch == epoch
                    && state.active.as_ref().is_some_and(|active| {
                        active.id == id
                            && active.revision == state.revision
                            && !active.abandoned
                            && !active.timed_out
                            && Instant::now() < active.deadline
                    })
            })
    }

    pub(crate) fn complete_if_current(
        &self,
        id: u64,
        epoch: u64,
        encode: impl FnOnce() -> Result<(u32, u32, Vec<u8>), CaptureError>,
    ) {
        if self.current(id, epoch) {
            self.complete(id, epoch, encode());
        } else {
            self.complete(id, epoch, Err(CaptureError::Cancelled));
        }
    }

    pub(crate) fn complete(
        &self,
        id: u64,
        epoch: u64,
        result: Result<(u32, u32, Vec<u8>), CaptureError>,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.active.as_ref().is_none_or(|active| active.id != id) {
            return;
        }
        let Some(active) = state.active.take() else {
            return;
        };
        active.timer.finish();
        let result = if state.closed || !self.lifetime.is_active() {
            Err(CaptureError::Closed)
        } else if !state.finished
            || state.epoch != epoch
            || active.epoch != epoch
            || active.revision != state.revision
            || active.abandoned
        {
            Err(CaptureError::Cancelled)
        } else if active.timed_out || Instant::now() >= active.deadline {
            Err(CaptureError::Timeout)
        } else {
            result.and_then(|(width, height, png)| {
                let raster = usize::try_from(width)
                    .ok()
                    .and_then(|width| {
                        usize::try_from(height)
                            .ok()
                            .and_then(|height| width.checked_mul(height))
                    })
                    .and_then(|pixels| pixels.checked_mul(4));
                if width == 0
                    || height == 0
                    || width > active.options.max_width
                    || height > active.options.max_height
                    || raster.is_none_or(|size| size > MAX_WEB_CAPTURE_RASTER_BYTES)
                    || png.len() < 24
                    || png.len() > active.options.max_png_bytes
                    || !png.starts_with(b"\x89PNG\r\n\x1a\n")
                    || png[16..20] != width.to_be_bytes()
                    || png[20..24] != height.to_be_bytes()
                {
                    return Err(CaptureError::TooLarge);
                }
                let content = CapturedContent {
                    window_generation: self.window_generation,
                    id,
                    epoch,
                    revision: active.revision,
                    width,
                    height,
                    png_bytes: png.len(),
                };
                if !active.abandoned && active.slot.upgrade().is_some() {
                    state.retained = Some(Retained {
                        id,
                        epoch,
                        revision: active.revision,
                        png: Arc::new(png),
                    });
                }
                Ok(content)
            })
        };
        drop(state);
        if let Some(slot) = active.slot.upgrade() {
            slot.complete(result);
        }
    }

    pub(crate) fn invalidate(&self, epoch: u64, closed: bool) {
        #[cfg(windows)]
        let mut active_id = None;
        let slot = if let Ok(mut state) = self.state.lock() {
            state.epoch = epoch;
            state.finished = false;
            state.closed |= closed;
            state.retained = None;
            // Reservation remains until the native callback returns. Even
            // dropping/cancelling the Rust future cannot overlap native snapshots.
            if let Some(active) = state.active.as_ref() {
                active.timer.finish();
                #[cfg(windows)]
                {
                    active_id = Some(active.id);
                }
            }
            state
                .active
                .as_ref()
                .and_then(|active| active.slot.upgrade())
        } else {
            None
        };
        #[cfg(windows)]
        if let Some(id) = active_id {
            self.discard_windows_stream(Some(id));
        }
        if let Some(slot) = slot {
            slot.complete(Err(if closed {
                CaptureError::Closed
            } else {
                CaptureError::Cancelled
            }));
        }
    }

    pub(crate) fn close(&self) {
        // HostLifetimeOwner calls this synchronously under its close-callback
        // lock. Only retire bytes/state here: invoking an arbitrary Future
        // waker under that lock could synchronously reenter retry_close.
        let retained = if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.finished = false;
            state.retained.take()
        } else {
            None
        };
        drop(retained);
        #[cfg(windows)]
        self.discard_windows_stream(None);
    }

    #[cfg(windows)]
    fn discard_windows_stream(&self, expected_id: Option<u64>) {
        if let Ok(dispatch) = self.dispatch.lock() {
            if let Some(dispatch) = dispatch.as_ref() {
                dispatch.discard_stream(expected_id);
            }
        }
    }

    pub(crate) fn notify_closed(&self) {
        let epoch = self
            .state
            .lock()
            .map_or(0, |state| state.epoch.saturating_add(1));
        self.invalidate(epoch, true);
    }

    pub(crate) fn finished(&self, epoch: u64) {
        if let Ok(mut state) = self.state.lock() {
            if self.lifetime.is_active() && !state.closed && state.epoch == epoch && epoch != 0 {
                state.finished = true;
            }
        }
    }

    pub(crate) fn viewport_changed(&self) {
        #[cfg(windows)]
        let mut active_id = None;
        let slot = if let Ok(mut state) = self.state.lock() {
            let Some(revision) = state.revision.checked_add(1) else {
                drop(state);
                self.close();
                return;
            };
            state.revision = revision;
            state.retained = None;
            if let Some(active) = state.active.as_ref() {
                active.timer.finish();
                #[cfg(windows)]
                {
                    active_id = Some(active.id);
                }
            }
            state
                .active
                .as_ref()
                .and_then(|active| active.slot.upgrade())
        } else {
            None
        };
        #[cfg(windows)]
        if let Some(id) = active_id {
            self.discard_windows_stream(Some(id));
        }
        if let Some(slot) = slot {
            slot.complete(Err(CaptureError::Cancelled));
        }
    }

    fn validate_delivery(&self, content: &CapturedContent) -> Result<(), CaptureError> {
        let state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
        if state.closed || !self.lifetime.is_active() {
            return Err(CaptureError::Closed);
        }
        if !state.finished
            || state.epoch != content.epoch
            || state.retained.as_ref().is_none_or(|retained| {
                retained.id != content.id
                    || retained.epoch != content.epoch
                    || retained.revision != content.revision
            })
        {
            return Err(CaptureError::Cancelled);
        }
        Ok(())
    }

    pub(crate) fn read(
        &self,
        content: &CapturedContent,
        offset: usize,
    ) -> Result<CapturedContentChunk, CaptureError> {
        if !self.lifetime.is_active() {
            self.close();
            return Err(CaptureError::Closed);
        }
        let state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
        if state.closed {
            return Err(CaptureError::Closed);
        }
        let retained = state
            .retained
            .as_ref()
            .filter(|retained| {
                content.window_generation == self.window_generation
                    && retained.id == content.id
                    && retained.epoch == content.epoch
                    && retained.revision == content.revision
                    && state.epoch == retained.epoch
                    && state.revision == retained.revision
                    && state.finished
            })
            .ok_or(CaptureError::Released)?;
        if offset > retained.png.len() {
            return Err(CaptureError::InvalidOffset);
        }
        let end = offset
            .saturating_add(MAX_WEB_CAPTURE_CHUNK_BYTES)
            .min(retained.png.len());
        Ok(CapturedContentChunk {
            bytes: retained.png[offset..end].to_vec(),
            next_offset: end,
            eof: end == retained.png.len(),
        })
    }

    pub(crate) fn release(&self, content: &CapturedContent) -> Result<(), CaptureError> {
        if !self.lifetime.is_active() {
            self.close();
            return Err(CaptureError::Closed);
        }
        let mut state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
        if state.closed {
            return Err(CaptureError::Closed);
        }
        if content.window_generation == self.window_generation
            && state.retained.as_ref().is_some_and(|retained| {
                retained.id == content.id
                    && retained.epoch == content.epoch
                    && retained.revision == content.revision
            })
        {
            state.retained = None;
            Ok(())
        } else {
            Err(CaptureError::Released)
        }
    }

    #[cfg(all(any(target_os = "macos", windows), feature = "native-clipboard"))]
    #[allow(clippy::rc_buffer)]
    pub(crate) fn lease_png(&self, token: CaptureToken) -> Result<Arc<Vec<u8>>, CaptureError> {
        if !self.lifetime.is_active() {
            self.close();
            return Err(CaptureError::Closed);
        }
        let state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
        if state.closed {
            return Err(CaptureError::Closed);
        }
        state
            .retained
            .as_ref()
            .filter(|retained| {
                token.window_generation == self.window_generation
                    && token.id == retained.id
                    && token.epoch == retained.epoch
                    && token.revision == retained.revision
                    && token.epoch == state.epoch
                    && token.revision == state.revision
                    && state.finished
            })
            .map(|retained| Arc::clone(&retained.png))
            .ok_or(CaptureError::Released)
    }

    /// Revalidate a capture token and run only a non-blocking admission
    /// update while the resource lock is held. Never call AppKit from `work`.
    #[cfg(all(any(target_os = "macos", windows), feature = "native-clipboard"))]
    pub(crate) fn with_valid_token<R>(
        &self,
        token: CaptureToken,
        work: impl FnOnce() -> R,
    ) -> Result<R, CaptureError> {
        if !self.lifetime.is_active() {
            return Err(CaptureError::Closed);
        }
        let state = self.state.lock().map_err(|_| CaptureError::Scheduler)?;
        if state.closed {
            return Err(CaptureError::Closed);
        }
        if !state.finished
            || state.epoch != token.epoch
            || state.revision != token.revision
            || token.window_generation != self.window_generation
            || state.retained.as_ref().is_none_or(|retained| {
                retained.id != token.id
                    || retained.epoch != token.epoch
                    || retained.revision != token.revision
            })
        {
            return Err(CaptureError::Released);
        }
        Ok(work())
    }

    fn abandon(&self, id: u64) {
        #[cfg(windows)]
        let mut abandoned = false;
        if let Ok(mut state) = self.state.lock() {
            if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
                active.abandoned = true;
                active.timer.finish();
                #[cfg(windows)]
                {
                    abandoned = true;
                }
            }
            if state
                .retained
                .as_ref()
                .is_some_and(|retained| retained.id == id)
            {
                state.retained = None;
            }
        }
        #[cfg(windows)]
        if abandoned {
            self.discard_windows_stream(Some(id));
        }
    }
}

/// Awaitable completion of one native visible-content snapshot.
#[must_use = "await native completion before reading the returned opaque resource"]
pub struct CaptureRequest {
    #[cfg(any(target_os = "macos", windows))]
    owner: Arc<CaptureState>,
    #[cfg(any(target_os = "macos", windows))]
    slot: Arc<Slot>,
    #[cfg(any(target_os = "macos", windows))]
    id: u64,
    #[cfg(any(target_os = "macos", windows))]
    epoch: u64,
    #[cfg(any(target_os = "macos", windows))]
    deadline: Instant,
    #[cfg(any(target_os = "macos", windows))]
    delivered: bool,
}

impl Future for CaptureRequest {
    type Output = Result<CapturedContent, CaptureError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(any(target_os = "macos", windows))]
        {
            let this = self.get_mut();
            if !this.owner.lifetime.is_active() {
                this.owner.close();
                return Poll::Ready(Err(CaptureError::Closed));
            }
            if Instant::now() >= this.deadline {
                this.owner.timeout(this.id, this.epoch);
            }
            let result = this.slot.poll(cx);
            if result.is_ready() {
                this.delivered = true;
            }
            match result {
                Poll::Ready(Ok(content)) if content.epoch != this.epoch => {
                    Poll::Ready(Err(CaptureError::Cancelled))
                }
                Poll::Ready(Ok(content)) => {
                    Poll::Ready(this.owner.validate_delivery(&content).map(|()| content))
                }
                other => other,
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = (self, cx);
            Poll::Ready(Err(CaptureError::Unsupported))
        }
    }
}

impl Drop for CaptureRequest {
    fn drop(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        if !self.delivered {
            self.owner.abandon(self.id);
        }
    }
}

#[cfg(all(test, any(target_os = "macos", windows)))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::task::Waker;

    fn new_owner() -> (Arc<CaptureState>, crate::HostLifetimeOwner) {
        let (host, lifetime) = crate::HostLifetime::new();
        let owner = CaptureState::new(lifetime, 77);
        {
            let mut state = owner.state.lock().unwrap();
            state.epoch = 1;
            state.finished = true;
        }
        (owner, host)
    }

    fn pending(owner: &Arc<CaptureState>, id: u64) -> CaptureRequest {
        let slot = Arc::new(Slot(Mutex::new(SlotState {
            result: None,
            waker: None,
        })));
        owner.state.lock().unwrap().active = Some(Active {
            id,
            epoch: 1,
            revision: 0,
            options: CaptureOptions::new(),
            abandoned: false,
            timed_out: false,
            deadline: Instant::now() + CAPTURE_DEADLINE,
            timer: DeadlineTimer::new(),
            slot: Arc::downgrade(&slot),
        });
        CaptureRequest {
            owner: Arc::clone(owner),
            slot,
            id,
            epoch: 1,
            deadline: Instant::now() + CAPTURE_DEADLINE,
            delivered: false,
        }
    }

    fn png(len: usize, width: u32, height: u32) -> Vec<u8> {
        let mut result = vec![0; len];
        result[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        result[16..20].copy_from_slice(&width.to_be_bytes());
        result[20..24].copy_from_slice(&height.to_be_bytes());
        result
    }

    #[test]
    fn checked_options_reject_invalid_dimensions_and_encoded_budget() {
        assert!(CaptureOptions::new().max_dimensions(0, 100).is_err());
        assert!(CaptureOptions::new().max_dimensions(1601, 100).is_err());
        assert!(CaptureOptions::new().max_png_bytes(0).is_err());
        assert!(CaptureOptions::new()
            .max_png_bytes(MAX_WEB_CAPTURE_PNG_BYTES + 1)
            .is_err());
        assert!(CaptureOptions::new().max_dimensions(800, 600).is_ok());
    }

    #[test]
    fn reads_are_credited_and_window_identity_retake_release_are_enforced() {
        let (owner, _host) = new_owner();
        let mut request = pending(&owner, 1);
        owner.complete(1, 1, Ok((320, 200, png(25_000, 320, 200))));
        let Poll::Ready(Ok(content)) =
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("test capture should complete");
        };
        drop(request);
        let first = owner.read(&content, 0).unwrap();
        assert_eq!(first.bytes.len(), MAX_WEB_CAPTURE_CHUNK_BYTES);
        assert!(!first.eof);
        let second = owner.read(&content, first.next_offset).unwrap();
        assert!(second.eof);
        assert_eq!(second.next_offset, content.png_bytes);
        assert!(matches!(
            owner.read(&content, 25_001),
            Err(CaptureError::InvalidOffset)
        ));
        let (other, _other_host) = new_owner();
        assert!(matches!(
            other.read(&content, 0),
            Err(CaptureError::Released)
        ));
        assert!(owner.release(&content).is_ok());
        assert!(matches!(
            owner.read(&content, 0),
            Err(CaptureError::Released)
        ));
    }

    #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
    #[test]
    fn retained_arc_moves_encoded_vec_and_lease_identity_survives_release() {
        let (owner, _host) = new_owner();
        let mut request = pending(&owner, 1);
        let encoded = png(25_000, 320, 200);
        let encoded_allocation = encoded.as_ptr();
        owner.complete(1, 1, Ok((320, 200, encoded)));
        let Poll::Ready(Ok(content)) =
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("test capture should complete");
        };
        drop(request);
        let first_lease = owner.lease_png(content.token()).unwrap();
        let second_lease = owner.lease_png(content.token()).unwrap();
        assert!(Arc::ptr_eq(&first_lease, &second_lease));
        assert_eq!(first_lease.as_ptr(), encoded_allocation);
        owner.release(&content).unwrap();
        assert!(matches!(
            owner.lease_png(content.token()),
            Err(CaptureError::Released)
        ));
        assert!(Arc::ptr_eq(&first_lease, &second_lease));
        assert_eq!(first_lease.len(), content.png_bytes);
    }

    #[test]
    fn oversize_drop_navigation_late_reply_and_next_call_after_revoke() {
        let (owner, host) = new_owner();
        drop(pending(&owner, 1));
        owner.complete(1, 1, Ok((320, 200, png(25_000, 320, 200))));
        assert!(owner.state.lock().unwrap().retained.is_none());
        let mut request = pending(&owner, 2);
        owner.complete(2, 1, Ok((1601, 200, png(25_000, 1601, 200))));
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(CaptureError::TooLarge))
        ));
        let mut pending_nav = pending(&owner, 3);
        owner.complete(3, 1, Ok((320, 200, png(25_000, 320, 200))));
        owner.invalidate(2, false);
        owner.finished(2);
        assert!(matches!(
            Pin::new(&mut pending_nav).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(CaptureError::Cancelled))
        ));
        owner.complete(3, 1, Ok((320, 200, png(25_000, 320, 200))));
        assert!(owner.state.lock().unwrap().retained.is_none());
        let _ = host.revoke();
        assert!(matches!(
            owner.begin(CaptureOptions::new()),
            Err(CaptureError::Closed)
        ));
        assert!(owner.state.lock().unwrap().retained.is_none());
    }

    #[test]
    fn viewport_change_cancels_inflight_and_releases_previous_resource() {
        let (owner, _host) = new_owner();
        let mut request = pending(&owner, 1);
        owner.complete(1, 1, Ok((320, 200, png(25_000, 320, 200))));
        let Poll::Ready(Ok(content)) =
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("expected a retained screenshot before resize");
        };
        owner.viewport_changed();
        assert!(matches!(
            owner.read(&content, 0),
            Err(CaptureError::Released)
        ));
        let mut racing = pending(&owner, 2);
        // pending() is a private test builder; associate it with the current
        // revision just as normal admission does.
        owner
            .state
            .lock()
            .unwrap()
            .active
            .as_mut()
            .unwrap()
            .revision = 1;
        owner.viewport_changed();
        assert!(matches!(
            Pin::new(&mut racing).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(CaptureError::Cancelled))
        ));
        owner.complete(2, 1, Ok((320, 200, png(25_000, 320, 200))));
        assert!(owner.state.lock().unwrap().retained.is_none());
    }

    #[test]
    fn deadline_wakes_request_but_preserves_native_reservation_until_late_callback() {
        let (owner, _host) = new_owner();
        let mut request = pending(&owner, 1);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut request).poll(&mut cx),
            Poll::Pending
        ));
        owner.timeout(1, 1);
        assert!(matches!(
            Pin::new(&mut request).poll(&mut cx),
            Poll::Ready(Err(CaptureError::Timeout))
        ));
        assert!(owner.state.lock().unwrap().active.is_some());
        assert!(!owner.current(1, 1));
        let encoded = std::sync::atomic::AtomicUsize::new(0);
        owner.complete_if_current(1, 1, || {
            encoded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok((320, 200, png(25_000, 320, 200)))
        });
        assert_eq!(encoded.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(owner.state.lock().unwrap().active.is_none());
        assert!(owner.state.lock().unwrap().retained.is_none());
        let mut fresh = pending(&owner, 2);
        owner.complete_if_current(2, 1, || Ok((320, 200, png(25_000, 320, 200))));
        assert!(matches!(
            Pin::new(&mut fresh).poll(&mut cx),
            Poll::Ready(Ok(_))
        ));
    }

    #[test]
    fn dropped_future_skips_bitmap_encoder_when_webkit_calls_back_late() {
        let (owner, _host) = new_owner();
        drop(pending(&owner, 1));
        assert!(owner.state.lock().unwrap().active.is_some());
        assert!(!owner.current(1, 1));
        let encoded = std::sync::atomic::AtomicUsize::new(0);
        owner.complete_if_current(1, 1, || {
            encoded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok((320, 200, png(25_000, 320, 200)))
        });
        assert_eq!(encoded.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(owner.state.lock().unwrap().active.is_none());
        assert!(owner.state.lock().unwrap().retained.is_none());
    }

    #[test]
    fn real_deadline_thread_finishes_never_callback_and_does_not_cancel_fast_success() {
        let (owner, _host) = new_owner();
        let mut hung = pending(&owner, 1);
        let deadline = Instant::now() + Duration::from_millis(25);
        let timer = {
            let mut state = owner.state.lock().unwrap();
            let active = state.active.as_mut().unwrap();
            active.deadline = deadline;
            Arc::clone(&active.timer)
        };
        hung.deadline = deadline;
        let reservation = owner.reserve_timer().unwrap();
        CaptureState::start_deadline(&timer, 1, 1, deadline, reservation).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while !owner
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .is_some_and(|active| active.timed_out)
        {
            assert!(Instant::now() < until, "deadline worker did not fire");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            Pin::new(&mut hung).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(CaptureError::Timeout))
        ));
        assert!(owner.state.lock().unwrap().active.is_some());
        owner.complete_if_current(1, 1, || Ok((320, 200, png(25_000, 320, 200))));
        assert!(owner.state.lock().unwrap().active.is_none());

        let mut fast = pending(&owner, 2);
        let timer = owner
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .timer
            .clone();
        CaptureState::start_deadline(
            &timer,
            2,
            1,
            Instant::now() + Duration::from_millis(50),
            owner.reserve_timer().unwrap(),
        )
        .unwrap();
        owner.complete_if_current(2, 1, || Ok((320, 200, png(25_000, 320, 200))));
        std::thread::sleep(Duration::from_millis(75));
        assert!(matches!(
            Pin::new(&mut fast).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(_))
        ));
    }

    #[test]
    fn deadline_worker_capacity_recovers_after_completion() {
        let (owner, _host) = new_owner();
        let reservations = (0..MAX_CAPTURE_DEADLINE_THREADS_PER_WINDOW)
            .map(|_| owner.reserve_timer().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            owner.reserve_timer(),
            Err(CaptureError::Overloaded)
        ));
        assert_eq!(
            owner.active_timer_threads.load(Ordering::Acquire),
            MAX_CAPTURE_DEADLINE_THREADS_PER_WINDOW
        );
        drop(reservations);
        assert_eq!(owner.active_timer_threads.load(Ordering::Acquire), 0);

        let timer = DeadlineTimer::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        let reservation = owner.reserve_timer().unwrap();
        CaptureState::start_deadline(&timer, 99, 1, deadline, reservation).unwrap();
        timer.finish();
        let until = Instant::now() + Duration::from_secs(1);
        while owner.active_timer_threads.load(Ordering::Acquire) != 0 {
            assert!(
                Instant::now() < until,
                "deadline reservation was not released"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn owner_revoke_discards_png_before_close_wake_without_draining_ui() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (host, lifetime) = crate::HostLifetime::new();
        let capture = CaptureState::new(lifetime.clone(), 77);
        {
            let mut state = capture.state.lock().unwrap();
            state.epoch = 1;
            state.finished = true;
        }
        let mut request = pending(&capture, 1);
        capture.complete(1, 1, Ok((320, 200, png(25_000, 320, 200))));
        assert!(matches!(
            Pin::new(&mut request).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(
            capture
                .state
                .lock()
                .unwrap()
                .retained
                .as_ref()
                .unwrap()
                .png
                .len(),
            25_000
        );
        let queued_close_wakes = Arc::new(AtomicUsize::new(0));
        let wake_counter = Arc::clone(&queued_close_wakes);
        let synchronous_capture = Arc::clone(&capture);
        let _registration = lifetime
            .register_close(Arc::new(move || {
                synchronous_capture.close();
                assert!(synchronous_capture.state.lock().unwrap().retained.is_none());
                // Stand-in for schedule_drain(id): do NOT deliver a UI wake.
                wake_counter.fetch_add(1, Ordering::AcqRel);
            }))
            .unwrap();
        host.revoke().unwrap();
        assert!(capture.state.lock().unwrap().retained.is_none());
        assert_eq!(queued_close_wakes.load(Ordering::Acquire), 1);
        host.revoke().unwrap();
        assert_eq!(queued_close_wakes.load(Ordering::Acquire), 1);
        host.retry_close().unwrap();
        assert!(capture.state.lock().unwrap().retained.is_none());
        assert_eq!(queued_close_wakes.load(Ordering::Acquire), 2);
    }
}
