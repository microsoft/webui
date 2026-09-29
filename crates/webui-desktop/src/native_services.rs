// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit Rust-host OS openers for a local-server window. No page bridge is
//! registered here: renderer access requires a separate generated IPC grant.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::execution::{ApplicationExecutor, Completion, WorkError};
use crate::{
    DesktopEvent, EventRegistrationError, EventRegistry, EventResponse, EventSubscription,
    HostLifetime,
};
use crate::{ThemeMode, ThemeRequest, ThemeState};

/// Maximum UTF-8 URL input, in bytes.
pub const MAX_NATIVE_URL_BYTES: usize = 2048;
/// Maximum local path representation, in bytes (not the document's size).
pub const MAX_NATIVE_DOCUMENT_PATH_BYTES: usize = 4096;
const OPEN_DEADLINE: Duration = Duration::from_secs(10);
// An OS opener can return before its timer thread has observed the shutdown
// signal. Bound those short-lived leftovers even under a rapid host retry loop.
const MAX_DEADLINE_THREADS_PER_WINDOW: usize = 8;
#[cfg(any(target_os = "macos", all(windows, feature = "native-capture")))]
static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Global Cocoa screen coordinates in points, with a bottom-left origin.
/// These are native points, **not** CSS pixels or backing pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRectPoints {
    /// Left edge in the global Cocoa screen coordinate system.
    pub x: f64,
    /// Bottom edge in the global Cocoa screen coordinate system.
    pub y: f64,
    /// Width in native screen points.
    pub width: f64,
    /// Height in native screen points.
    pub height: f64,
}

/// Read-only native content geometry sampled from the live macOS WKWebView.
///
/// `page_zoom` and `magnification` are observations, not a proven mapping
/// from CSS `getBoundingClientRect()` or `visualViewport` to screen points.
/// The backing scale is *not* a multiplier for CSS coordinates. Re-measure
/// the DOM anchor after layout or viewport changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentGeometry {
    /// Generation of this native window; changes when a new frame is created.
    pub window_generation: u64,
    /// Native main-document validity epoch; changes on actual provisional
    /// starts, even when the previous document survives a failed load.
    pub document_epoch: u64,
    /// Increases on native geometry notifications, document changes, and
    /// observed zoom changes. A later snapshot may have a different revision.
    pub revision: u64,
    /// WKWebView bounds converted through its window into global screen points.
    pub screen_rect: ScreenRectPoints,
    /// Live NSWindow backing scale (screen pixels per native point).
    pub backing_scale: f64,
    /// Live WKWebView pageZoom property, not an inferred CSS transform.
    pub page_zoom: f64,
    /// Live WKWebView magnification property, not an inferred CSS transform.
    pub magnification: f64,
}

/// Failure to validate, schedule, or complete a native OS open.
#[derive(Debug, thiserror::Error)]
pub enum NativeServiceError {
    /// Only absolute HTTP(S) URLs without embedded credentials are permitted.
    #[error(
        "expected an absolute HTTP(S) URL with a host, no credentials, and at most 2048 bytes"
    )]
    InvalidUrl,
    /// The host must supply an absolute, local, regular file.
    #[error("expected an absolute local regular .txt/.log/.md/.csv/.json/.pdf document path of at most 4096 bytes")]
    InvalidDocument,
    /// One OS open is already pending on this window.
    #[error("an OS open is already in progress for this window")]
    Busy,
    /// The owning frame has retired.
    #[error("the native window or verified host has closed")]
    Closed,
    /// The bounded application worker queue is full.
    #[error("native opener capacity is exhausted; retry after the pending operation completes")]
    Overloaded,
    /// A navigation invalidated this window's pending operation.
    #[error("the native operation was cancelled by navigation or window closure")]
    Cancelled,
    /// The OS opener did not acknowledge the request before the deadline.
    #[error(
        "the OS opener did not respond within 10 seconds; its external launch may still finish"
    )]
    Deadline,
    /// The native opener rejected the URL or file.
    #[error("OS opener failed: {0}")]
    Os(String),
    /// Lifecycle registration or timer setup was unavailable.
    #[error("native service unavailable")]
    Unavailable,
    /// Lifecycle cancellation could not be registered.
    #[error("native lifecycle registration failed: {0}")]
    Registration(#[from] EventRegistrationError),
    /// The selected platform has no native content geometry adapter.
    #[error("native content geometry is currently supported only on macOS")]
    Unsupported,
    /// No verified, finished main document or live native view is available.
    #[error("native content geometry is unavailable until the main document finishes loading")]
    GeometryUnavailable,
    /// Navigation, geometry change, or teardown superseded this snapshot.
    #[error("native content geometry became stale; request a fresh snapshot")]
    Stale,
    /// The selected native backend has no proven per-window theme override.
    #[error("native theme override is not supported on this platform")]
    ThemeUnsupported,
    /// Native appearance cannot be read before the window is attached.
    #[error("native theme is unavailable until the window is attached")]
    ThemeUnavailable,
    /// Only one theme change may be queued per window.
    #[error("a native theme change is already pending for this window")]
    ThemeBusy,
    /// AppKit did not acknowledge an appearance change within ten seconds.
    #[error("native theme change timed out after 10 seconds; read current_theme before retrying")]
    ThemeTimeout,
}

struct Inner {
    executor: Arc<ApplicationExecutor>,
    lifetime: HostLifetime,
    closed: AtomicBool,
    #[cfg(target_os = "macos")]
    window_generation: u64,
    /// OS-opener cancellation generation: navigation *requests* cancel opens
    /// even if the page later prevents that request.
    generation: AtomicU64,
    #[cfg(target_os = "macos")]
    document_epoch: AtomicU64,
    #[cfg(target_os = "macos")]
    committed_epoch: AtomicU64,
    #[cfg(target_os = "macos")]
    geometry_revision: AtomicU64,
    #[cfg(target_os = "macos")]
    committed_url: Mutex<Option<String>>,
    #[cfg(target_os = "macos")]
    last_page_zoom: AtomicU64,
    #[cfg(target_os = "macos")]
    last_magnification: AtomicU64,
    #[cfg(target_os = "macos")]
    geometry_dispatch: Mutex<Option<Arc<platform::Dispatch>>>,
    #[cfg(all(any(target_os = "macos", windows), feature = "native-capture"))]
    capture: Arc<crate::capture::CaptureState>,
    #[cfg(all(windows, feature = "native-capture"))]
    document_epoch: AtomicU64,
    #[cfg(all(windows, feature = "native-capture"))]
    navigation_id: AtomicU64,
    #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
    clipboard: Arc<crate::clipboard::ClipboardState>,
    busy: AtomicBool,
    active_timers: std::sync::atomic::AtomicUsize,
    timer: Mutex<Option<Weak<Timer>>>,
    registration: Mutex<Option<EventSubscription>>,
    #[cfg(target_os = "macos")]
    theme: Arc<crate::native_theme::platform::Controller>,
}

struct Busy(Arc<Inner>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

struct TimerReservation(Arc<Inner>);
impl Drop for TimerReservation {
    fn drop(&mut self) {
        self.0.active_timers.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Timer {
    state: Mutex<TimerState>,
    changed: Condvar,
}

#[derive(Default)]
struct TimerState {
    finished: bool,
    waker: Option<Waker>,
}

impl Timer {
    fn wake(&self) {
        let waker = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.waker.take());
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn finish(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.finished = true;
            self.changed.notify_one();
        }
    }
}

/// Cloneable trusted-Rust capability tied to one local-server window.
///
/// This capability never creates a JavaScript global or grants page IPC.
/// Hosts may explicitly expose a generated registry method with their own
/// authorization policy; never forward arbitrary renderer paths to it.
#[derive(Clone)]
pub struct NativeServices(Arc<Inner>);

impl NativeServices {
    pub(crate) fn new(
        events: &EventRegistry,
        executor: Arc<ApplicationExecutor>,
        lifetime: HostLifetime,
    ) -> Result<Self, NativeServiceError> {
        if !lifetime.is_active() {
            return Err(NativeServiceError::Closed);
        }
        #[cfg(target_os = "macos")]
        let theme =
            crate::native_theme::platform::Controller::new(events.clone(), lifetime.clone());
        #[cfg(target_os = "macos")]
        let window_generation = NEXT_WINDOW_GENERATION.fetch_add(1, Ordering::Relaxed);
        #[cfg(all(windows, feature = "native-capture"))]
        let window_generation = NEXT_WINDOW_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| NativeServiceError::Unavailable)?;
        #[cfg(all(any(target_os = "macos", windows), feature = "native-capture"))]
        let capture = crate::capture::CaptureState::new(lifetime.clone(), window_generation);

        #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
        let clipboard =
            crate::clipboard::ClipboardState::new(lifetime.clone(), Arc::clone(&capture));
        let inner = Arc::new(Inner {
            executor,
            lifetime,
            closed: AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            window_generation,
            generation: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            document_epoch: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            committed_epoch: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            geometry_revision: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            committed_url: Mutex::new(None),
            #[cfg(target_os = "macos")]
            last_page_zoom: AtomicU64::new(1_f64.to_bits()),
            #[cfg(target_os = "macos")]
            last_magnification: AtomicU64::new(1_f64.to_bits()),
            #[cfg(target_os = "macos")]
            geometry_dispatch: Mutex::new(None),
            #[cfg(all(any(target_os = "macos", windows), feature = "native-capture"))]
            capture,
            #[cfg(all(windows, feature = "native-capture"))]
            document_epoch: AtomicU64::new(0),
            #[cfg(all(windows, feature = "native-capture"))]
            navigation_id: AtomicU64::new(0),
            #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
            clipboard,
            busy: AtomicBool::new(false),
            active_timers: std::sync::atomic::AtomicUsize::new(0),
            timer: Mutex::new(None),
            registration: Mutex::new(None),
            #[cfg(target_os = "macos")]
            theme,
        });
        let weak = Arc::downgrade(&inner);
        let subscription = events.subscribe(move |event| {
            if let Some(inner) = weak.upgrade() {
                match event {
                    DesktopEvent::NavigationRequested { .. } => inner.cancel_open(false),
                    DesktopEvent::WindowClosed { .. } | DesktopEvent::Exiting => {
                        inner.cancel_open(true);
                    }
                    DesktopEvent::WindowMoved { .. }
                    | DesktopEvent::WindowResized { .. }
                    | DesktopEvent::WindowMaximized { .. }
                    | DesktopEvent::WindowUnmaximized { .. }
                    | DesktopEvent::WindowEnteredFullscreen { .. }
                    | DesktopEvent::WindowLeftFullscreen { .. }
                    | DesktopEvent::ScaleFactorChanged { .. } => {
                        #[cfg(all(windows, feature = "native-capture"))]
                        inner.capture.viewport_changed();
                        #[cfg(target_os = "macos")]
                        {
                            inner.geometry_revision.fetch_add(1, Ordering::AcqRel);
                            #[cfg(feature = "native-capture")]
                            inner.capture.viewport_changed();
                            #[cfg(feature = "native-clipboard")]
                            inner.clipboard.navigation_changed();
                        }
                    }
                    _ => {}
                }
            }
            EventResponse::Continue
        })?;
        *inner
            .registration
            .lock()
            .map_err(|_| NativeServiceError::Unavailable)? = Some(subscription);
        Ok(Self(inner))
    }

    /// Open a parsed, bounded HTTP(S) URL in the user's system browser.
    ///
    /// The returned future resolves after the OS opener responds, **not** on
    /// queue admission or after the browser has loaded the page. Do not block
    /// a native UI callback waiting for it. Navigation/window close cancels
    /// queued work (including a subsequently prevented navigation request);
    /// an OS launch already handed off cannot be recalled.
    ///
    /// # Errors
    ///
    /// Rejects invalid URLs, retired windows, overload, and scheduling errors.
    /// Await the returned operation for OS failure, cancellation, or deadline.
    pub fn open_url(&self, input: &str) -> Result<NativeOpen, NativeServiceError> {
        let url = validate_url(input)?;
        self.start(move || platform::open_url(&url))
    }

    /// Open a trusted host-provided absolute local regular file with the OS
    /// document opener. Only `.txt`, `.log`, `.md`, `.csv`, `.json`, and `.pdf`
    /// documents are accepted. File type and canonical path are checked on a
    /// worker, so filesystem access never blocks the native UI thread.
    ///
    /// This is not a renderer-selected path API. Do not accept paths from
    /// untrusted pages; the host owns selection and filesystem authorization.
    /// A symlink/mount replaced after validation remains an OS-level race.
    ///
    /// # Errors
    ///
    /// Rejects invalid paths, retired windows, overload, and scheduling
    /// errors. Await the operation for file or OS failure and deadline.
    pub fn open_document(&self, path: impl AsRef<Path>) -> Result<NativeOpen, NativeServiceError> {
        let path = path.as_ref();
        if !path.is_absolute()
            || path.as_os_str().len() > MAX_NATIVE_DOCUMENT_PATH_BYTES
            || path.as_os_str().as_encoded_bytes().contains(&0)
        {
            return Err(NativeServiceError::InvalidDocument);
        }

        #[cfg(windows)]
        if path.to_string_lossy().starts_with(r"\\") || path.to_string_lossy().starts_with("//") {
            return Err(NativeServiceError::InvalidDocument);
        }
        let path = path.to_path_buf();
        self.start(move || {
            let path = validate_document(&path)?;
            platform::open_document(&path)
        })
    }

    /// Schedule one read-only snapshot on the native UI thread. Await the
    /// result from a non-UI thread; never synchronously wait inside a native
    /// callback. No document content, DOM coordinates, or secrets are read.
    ///
    /// # Errors
    ///
    /// Returns `Unsupported` on Windows/Linux, `GeometryUnavailable` before
    /// a finished main document, and bounded-capacity/closed errors when the
    /// native window cannot accept a read. The future may resolve `Stale`
    /// if navigation or teardown races its completion.
    pub fn content_geometry(&self) -> Result<GeometryRequest, NativeServiceError> {
        #[cfg(target_os = "macos")]
        {
            platform::request(&self.0)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(NativeServiceError::Unsupported)
        }
    }

    /// Capture only visible native webview content in a bounded, opaque PNG
    /// resource owned by this window and its currently finished main document.
    /// The host must establish iframe readiness separately; this API does not
    /// guarantee that embedded frames have painted.
    /// This installs no renderer global or grant and needs no screen permission.
    ///
    /// # Errors
    ///
    /// Returns `Unsupported` on Linux; on macOS/Windows rejects unavailable,
    /// busy, closed and stale windows and an incomplete or oversized native result.
    /// Its future times out after ten seconds; a hung native callback retains
    /// its Busy reservation rather than permitting overlapping snapshots.
    #[cfg(feature = "native-capture")]
    pub fn capture_web_content(
        &self,
        options: crate::CaptureOptions,
    ) -> Result<crate::CaptureRequest, crate::CaptureError> {
        #[cfg(any(target_os = "macos", windows))]
        {
            if self.0.closed.load(Ordering::Acquire) || !self.0.lifetime.is_active() {
                self.0.capture.close();
                return Err(crate::CaptureError::Closed);
            }
            let request = self.0.capture.begin(options)?;
            #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
            self.0.clipboard.cancel_invalidated();
            Ok(request)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = options;
            Err(crate::CaptureError::Unsupported)
        }
    }

    /// Copy at most 20 KiB from the retained PNG. Generated IPC consumers
    /// must separately authorize and credit each chunk against their own
    /// aggregate transport budget; no full data URL is provided.
    ///
    /// # Errors
    ///
    /// Rejects stale or cross-window handles and offsets beyond the PNG.
    #[cfg(feature = "native-capture")]
    pub fn read_captured_content(
        &self,
        content: &crate::CapturedContent,
        offset: usize,
    ) -> Result<crate::CapturedContentChunk, crate::CaptureError> {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.0.capture.read(content, offset)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = (content, offset);
            Err(crate::CaptureError::Unsupported)
        }
    }

    /// Explicitly release the native PNG bytes. A retake, real navigation or
    /// window retirement also releases them automatically.
    ///
    /// # Errors
    ///
    /// Returns `Released` for stale or foreign resource handles.
    #[cfg(feature = "native-capture")]
    pub fn release_captured_content(
        &self,
        content: &crate::CapturedContent,
    ) -> Result<(), crate::CaptureError> {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.0.capture.release(content)?;
            #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
            self.0.clipboard.cancel_token(content.token());
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = content;
            Err(crate::CaptureError::Unsupported)
        }
    }

    /// Write one currently retained PNG to the macOS general clipboard.
    /// The returned future resolves only after AppKit acknowledges the
    /// `public.png` write and immediate byte-for-byte readback. The trusted
    /// host must await success before any separately validated URL opener;
    /// no issue navigation or renderer method is installed by this API.
    ///
    /// # Errors
    ///
    /// Rejects released/foreign captures, retired documents, concurrent
    /// writes and unsupported platforms. OS refusal, contention, readback
    /// mismatch and a finite deadline are reported by the returned future.
    #[cfg(feature = "native-clipboard")]
    pub fn write_capture_to_clipboard(
        &self,
        content: &crate::CapturedContent,
    ) -> Result<crate::ClipboardRequest, crate::ClipboardError> {
        #[cfg(target_os = "macos")]
        {
            self.0.clipboard.begin(content)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = content;
            Err(crate::ClipboardError::Unsupported)
        }
    }

    /// Schedule an opt-in appearance override on this window's native UI
    /// thread. The resulting state reflects the window's effective appearance.
    /// This does not save the host preference or grant renderer IPC.
    ///
    /// # Errors
    ///
    /// Returns `ThemeUnsupported` on Windows/Linux, or a typed closed, busy,
    /// unavailable, or scheduling error. Await for acknowledgement or timeout.
    pub fn set_theme(&self, mode: ThemeMode) -> Result<ThemeRequest, NativeServiceError> {
        #[cfg(target_os = "macos")]
        {
            if self.0.closed.load(Ordering::Acquire) || !self.0.lifetime.is_active() {
                return Err(NativeServiceError::Closed);
            }
            self.0.theme.set(mode)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = mode;
            Err(NativeServiceError::ThemeUnsupported)
        }
    }

    /// Return the latest effective native appearance, including for a newly
    /// admitted document that missed an earlier `ThemeChanged` event.
    /// Host-rendered CSS remains the application's responsibility.
    ///
    /// # Errors
    ///
    /// Returns `ThemeUnsupported` on Windows/Linux and `ThemeUnavailable`
    /// until the macOS native window has attached.
    pub fn current_theme(&self) -> Result<ThemeState, NativeServiceError> {
        #[cfg(target_os = "macos")]
        {
            if self.0.closed.load(Ordering::Acquire) || !self.0.lifetime.is_active() {
                return Err(NativeServiceError::Closed);
            }
            self.0.theme.snapshot()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(NativeServiceError::ThemeUnsupported)
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn attach_theme(
        &self,
        window: &objc2_app_kit::NSWindow,
        view: &objc2_web_kit::WKWebView,
    ) -> crate::native_theme::platform::Registration {
        self.0.theme.attach(window, view)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn theme_controller(&self) -> Arc<crate::native_theme::platform::Controller> {
        Arc::clone(&self.0.theme)
    }

    #[cfg(all(target_os = "macos", feature = "native-capture"))]
    pub(crate) fn capture_for_revoke(&self) -> Arc<crate::capture::CaptureState> {
        Arc::clone(&self.0.capture)
    }

    #[cfg(all(windows, feature = "native-capture"))]
    pub(crate) fn capture_for_revoke(&self) -> Arc<crate::capture::CaptureState> {
        Arc::clone(&self.0.capture)
    }

    #[cfg(all(windows, feature = "native-capture"))]
    pub(crate) fn capture_navigation_started(&self, navigation_id: u64) {
        let Ok(epoch) =
            self.0
                .document_epoch
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
        else {
            self.0.capture.close();
            return;
        };
        self.0.navigation_id.store(navigation_id, Ordering::Release);
        self.0.capture.invalidate(epoch + 1, false);
    }

    #[cfg(all(windows, feature = "native-capture"))]
    pub(crate) fn capture_navigation_finished(&self, navigation_id: u64) {
        let epoch = self.0.document_epoch.load(Ordering::Acquire);
        if epoch != 0
            && navigation_id != 0
            && self.0.navigation_id.load(Ordering::Acquire) == navigation_id
        {
            self.0.capture.finished(epoch);
        }
    }

    #[cfg(all(target_os = "macos", feature = "native-clipboard"))]
    pub(crate) fn clipboard_for_revoke(&self) -> Arc<crate::clipboard::ClipboardState> {
        Arc::clone(&self.0.clipboard)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn attach_geometry(
        &self,
        window: &objc2_app_kit::NSWindow,
        view: &objc2_web_kit::WKWebView,
    ) -> GeometryRegistration {
        GeometryRegistration {
            geometry: platform::install(&self.0, window, view),
            #[cfg(feature = "native-capture")]
            capture: crate::macos::capture::install(&self.0.capture, window, view),
            #[cfg(feature = "native-clipboard")]
            clipboard: crate::macos::clipboard::install(&self.0.clipboard),
        }
    }

    fn start(
        &self,
        work: impl FnOnce() -> Result<(), NativeServiceError> + Send + 'static,
    ) -> Result<NativeOpen, NativeServiceError> {
        let inner = &self.0;
        if inner.closed.load(Ordering::Acquire) || !inner.lifetime.is_active() {
            return Err(NativeServiceError::Closed);
        }
        if inner.busy.swap(true, Ordering::AcqRel) {
            return Err(NativeServiceError::Busy);
        }
        let permit = Arc::new(Busy(Arc::clone(inner)));
        let generation = inner.generation.load(Ordering::Acquire);
        inner
            .active_timers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_DEADLINE_THREADS_PER_WINDOW).then_some(active + 1)
            })
            .map_err(|_| NativeServiceError::Overloaded)?;
        let timer_reservation = TimerReservation(Arc::clone(inner));
        // Establish the deadline before admitting a worker. If timer startup
        // fails, returning Unavailable must never leave an OS open in flight.
        let timer = Arc::new(Timer {
            state: Mutex::new(TimerState::default()),
            changed: Condvar::new(),
        });
        let deadline = Instant::now() + OPEN_DEADLINE;
        let timeout = Arc::clone(&timer);
        std::thread::Builder::new()
            .name("webui-os-open-deadline".into())
            .spawn(move || {
                let _reservation = timer_reservation;
                let Ok(state) = timeout.state.lock() else {
                    return;
                };
                let Ok((state, _)) =
                    timeout
                        .changed
                        .wait_timeout_while(state, OPEN_DEADLINE, |state| !state.finished)
                else {
                    return;
                };
                if !state.finished {
                    drop(state);
                    timeout.wake();
                }
            })
            .map_err(|_| NativeServiceError::Unavailable)?;
        if let Ok(mut active) = inner.timer.lock() {
            *active = Some(Arc::downgrade(&timer));
        }
        let worker_permit = Arc::clone(&permit);
        let worker_inner = Arc::clone(inner);
        let completion = match inner.executor.submit(move || {
            let _permit = worker_permit;
            if worker_inner.closed.load(Ordering::Acquire)
                || !worker_inner.lifetime.is_active()
                || worker_inner.generation.load(Ordering::Acquire) != generation
            {
                return Err(NativeServiceError::Cancelled);
            }
            work()
        }) {
            Ok(completion) => completion,
            Err(error) => {
                // No worker was admitted; release the timer immediately rather
                // than leaving a sleeper behind for the full deadline.
                timer.finish();
                return Err(work_error(error));
            }
        };
        Ok(NativeOpen {
            inner: Arc::clone(inner),
            completion,
            generation,
            deadline,
            timer,
            _permit: permit,
        })
    }

    pub(crate) fn close(&self) {
        self.0.cancel_open(true);
        if let Ok(mut registration) = self.0.registration.lock() {
            registration.take();
        }
    }

    /// Only WebKit's *actual* main-frame provisional start retires geometry.
    /// Policy requests rejected by Rust handlers leave the old page usable.
    #[cfg(target_os = "macos")]
    pub(crate) fn provisional_started(&self) {
        let inner = &self.0;
        if inner.closed.load(Ordering::Acquire) || !inner.lifetime.is_active() {
            return;
        }
        inner.document_epoch.fetch_add(1, Ordering::AcqRel);
        #[cfg(feature = "native-capture")]
        inner
            .capture
            .invalidate(inner.document_epoch.load(Ordering::Acquire), false);
        #[cfg(feature = "native-clipboard")]
        inner.clipboard.navigation_changed();
        inner.committed_epoch.store(0, Ordering::Release);
        inner.geometry_revision.fetch_add(1, Ordering::AcqRel);
        platform::cancel_pending(inner, false);
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn navigation_committed(&self) {
        // After a real document commit, the previous page is no longer a
        // restoration candidate even if the new document later fails.
        if let Ok(mut url) = self.0.committed_url.lock() {
            url.take();
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn navigation_finished(&self, url: &str) {
        let inner = &self.0;
        if url.is_empty()
            || url.len() > MAX_NATIVE_URL_BYTES
            || inner.closed.load(Ordering::Acquire)
            || !inner.lifetime.is_active()
        {
            return;
        }
        let epoch = inner.document_epoch.load(Ordering::Acquire);
        if epoch == 0 {
            return;
        }
        let Ok(mut committed_url) = inner.committed_url.lock() else {
            return;
        };
        *committed_url = Some(url.to_owned());
        inner.committed_epoch.store(epoch, Ordering::Release);
        #[cfg(feature = "native-capture")]
        inner.capture.finished(epoch);
        inner.geometry_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Restore availability only after the matching provisional navigation
    /// failed and WebKit proves the *same* old document URL is still live.
    /// A new monotonic epoch prevents old queued geometry from being replayed.
    #[cfg(target_os = "macos")]
    pub(crate) fn provisional_failed(&self, observed_url: Option<&str>, loading: bool) -> bool {
        let inner = &self.0;
        if loading || inner.closed.load(Ordering::Acquire) || !inner.lifetime.is_active() {
            return false;
        }
        let Some(observed_url) = observed_url else {
            return false;
        };
        let Ok(committed_url) = inner.committed_url.lock() else {
            return false;
        };
        if committed_url.as_deref() != Some(observed_url) {
            return false;
        }
        let epoch = inner.document_epoch.load(Ordering::Acquire);
        if epoch == 0 {
            return false;
        }
        inner.committed_epoch.store(epoch, Ordering::Release);
        #[cfg(feature = "native-capture")]
        inner.capture.finished(epoch);
        inner.geometry_revision.fetch_add(1, Ordering::AcqRel);
        true
    }
}

impl Inner {
    fn cancel_open(&self, close: bool) {
        #[cfg(target_os = "macos")]
        self.theme.cancel(
            if close {
                NativeServiceError::Closed
            } else {
                NativeServiceError::Cancelled
            },
            close,
        );
        if close {
            self.closed.store(true, Ordering::Release);
            #[cfg(all(windows, feature = "native-capture"))]
            self.capture.close();
            #[cfg(target_os = "macos")]
            {
                self.committed_epoch.store(0, Ordering::Release);
                self.document_epoch.fetch_add(1, Ordering::AcqRel);
                #[cfg(feature = "native-capture")]
                self.capture.close();
                #[cfg(feature = "native-clipboard")]
                self.clipboard.close_silent();
                self.geometry_revision.fetch_add(1, Ordering::AcqRel);
                platform::cancel_pending(self, true);
            }
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(timer) = self.timer.lock() {
            if let Some(timer) = timer.as_ref().and_then(Weak::upgrade) {
                timer.wake();
            }
        }
    }
}

/// Awaitable native geometry read. The result is checked again at delivery:
/// a navigation, window close, or native geometry event makes it stale.
#[must_use = "await the UI-thread read; scheduling is not a geometry snapshot"]
pub struct GeometryRequest {
    #[cfg(target_os = "macos")]
    inner: Arc<Inner>,
    #[cfg(target_os = "macos")]
    epoch: u64,
    #[cfg(target_os = "macos")]
    slot: Arc<platform::GeometrySlot>,
}

impl Future for GeometryRequest {
    type Output = Result<ContentGeometry, NativeServiceError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(target_os = "macos")]
        {
            let this = self.get_mut();
            if this.inner.closed.load(Ordering::Acquire) || !this.inner.lifetime.is_active() {
                return Poll::Ready(Err(NativeServiceError::Closed));
            }
            if this.inner.document_epoch.load(Ordering::Acquire) != this.epoch {
                return Poll::Ready(Err(NativeServiceError::Stale));
            }
            this.slot.poll(cx, &this.inner)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (self, cx);
            Poll::Ready(Err(NativeServiceError::Unsupported))
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) struct GeometryRegistration {
    geometry: platform::Registration,
    #[cfg(feature = "native-capture")]
    capture: crate::macos::capture::Registration,
    #[cfg(feature = "native-clipboard")]
    clipboard: crate::macos::clipboard::Registration,
}

#[cfg(target_os = "macos")]
impl GeometryRegistration {
    pub(crate) fn close(&self) {
        #[cfg(feature = "native-clipboard")]
        self.clipboard.close();
        #[cfg(feature = "native-capture")]
        self.capture.close();
        self.geometry.close();
    }
}

/// Awaitable completion of an OS opener call (not browser load completion).
/// Dropping it cancels queued work but cannot retract an already-issued OS call.
#[must_use = "await the OS opener response; admission alone is not completion"]
pub struct NativeOpen {
    inner: Arc<Inner>,
    completion: Completion<Result<(), NativeServiceError>>,
    generation: u64,
    deadline: Instant,
    timer: Arc<Timer>,
    _permit: Arc<Busy>,
}

impl Future for NativeOpen {
    type Output = Result<(), NativeServiceError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.inner.closed.load(Ordering::Acquire)
            || !this.inner.lifetime.is_active()
            || this.inner.generation.load(Ordering::Acquire) != this.generation
        {
            return Poll::Ready(Err(NativeServiceError::Cancelled));
        }
        if Instant::now() >= this.deadline {
            return Poll::Ready(Err(NativeServiceError::Deadline));
        }
        match Pin::new(&mut this.completion).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(error)) => Poll::Ready(Err(work_error(error))),
            Poll::Pending => {
                if let Ok(mut state) = this.timer.state.lock() {
                    state.waker = Some(cx.waker().clone());
                }
                // Registration may race a lifecycle event or timeout wake.
                if this.inner.closed.load(Ordering::Acquire)
                    || !this.inner.lifetime.is_active()
                    || this.inner.generation.load(Ordering::Acquire) != this.generation
                    || Instant::now() >= this.deadline
                {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }
        }
    }
}

impl Drop for NativeOpen {
    fn drop(&mut self) {
        self.timer.finish();
    }
}

fn work_error(error: WorkError) -> NativeServiceError {
    match error {
        WorkError::Closed => NativeServiceError::Closed,
        WorkError::Overloaded => NativeServiceError::Overloaded,
    }
}

fn validate_url(input: &str) -> Result<String, NativeServiceError> {
    if input.len() > MAX_NATIVE_URL_BYTES
        || input
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\')
    {
        return Err(NativeServiceError::InvalidUrl);
    }
    let url = url::Url::parse(input).map_err(|_| NativeServiceError::InvalidUrl)?;
    let explicit_http_scheme = input.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")
    });
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || !explicit_http_scheme
    {
        return Err(NativeServiceError::InvalidUrl);
    }
    if url.as_str().len() > MAX_NATIVE_URL_BYTES {
        return Err(NativeServiceError::InvalidUrl);
    }
    Ok(url.into())
}

fn validate_document(path: &Path) -> Result<PathBuf, NativeServiceError> {
    let is_document = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            ["txt", "log", "md", "csv", "json", "pdf"]
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        });
    if !is_document {
        return Err(NativeServiceError::InvalidDocument);
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| NativeServiceError::InvalidDocument)?;
    if !metadata.file_type().is_file() {
        return Err(NativeServiceError::InvalidDocument);
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| NativeServiceError::InvalidDocument)?;
    if !canonical.is_absolute()
        || canonical.as_os_str().len() > MAX_NATIVE_DOCUMENT_PATH_BYTES
        || !canonical
            .metadata()
            .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(NativeServiceError::InvalidDocument);
    }
    Ok(canonical)
}

#[cfg(target_os = "macos")]
#[path = "macos/services.rs"]
#[allow(unsafe_code)]
mod platform;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::task::Waker;

    fn services() -> (NativeServices, EventRegistry, crate::HostLifetimeOwner) {
        let events = EventRegistry::default();
        let (owner, lifetime) = HostLifetime::new();
        let services = NativeServices::new(&events, Arc::default(), lifetime).unwrap();
        (services, events, owner)
    }

    #[cfg(all(windows, feature = "native-capture"))]
    #[test]
    fn windows_capture_needs_matching_successful_navigation_and_retires_on_revoke() {
        let (services, _events, host) = services();
        let capture = services.capture_for_revoke();
        assert!(!capture.test_finished());
        services.capture_navigation_started(11);
        services.capture_navigation_finished(10);
        assert!(!capture.test_finished());
        services.capture_navigation_finished(11);
        assert!(capture.test_finished());
        services.capture_navigation_started(12);
        services.capture_navigation_finished(11);
        assert!(!capture.test_finished());
        services.capture_navigation_finished(12);
        assert!(capture.test_finished());
        host.revoke().unwrap();
        assert!(matches!(
            services.capture_web_content(crate::CaptureOptions::new()),
            Err(crate::CaptureError::Closed)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[allow(unsafe_code)]
    #[cfg(feature = "native-clipboard")]
    fn public_clipboard_request_completes_through_private_dispatch_and_readback() {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};

        struct PrivateBoard(objc2::rc::Retained<NSPasteboard>);
        impl Drop for PrivateBoard {
            fn drop(&mut self) {
                // SAFETY: This releases this test's unique pasteboard name,
                // never the user's general pasteboard.
                unsafe {
                    let _: () = objc2::msg_send![&*self.0, releaseGlobally];
                }
            }
        }

        let (services, _events, _host) = services();
        let png = &crate::macos::clipboard::TEST_PNG;
        services.0.capture.test_store_retained_png(png);
        let content = services.0.capture.test_content().unwrap();
        let board = PrivateBoard(NSPasteboard::pasteboardWithUniqueName());
        let registration =
            crate::macos::clipboard::install_private(&services.0.clipboard, board.0.clone());
        let mut pending = services.write_capture_to_clipboard(&content).unwrap();
        assert!(matches!(
            services.write_capture_to_clipboard(&content),
            Err(crate::ClipboardError::Busy)
        ));
        // Test mode drains the SAME admitted Dispatch and AppKit helper on
        // this thread instead of running a foreground NSApplication loop.
        registration.test_drain();
        assert!(matches!(
            Pin::new(&mut pending).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
        // SAFETY: Public AppKit PNG pasteboard type is an immutable symbol.
        let data = board.0.dataForType(unsafe { NSPasteboardTypePNG }).unwrap();
        // SAFETY: NSData is immutable and retained throughout comparison.
        assert_eq!(unsafe { data.as_bytes_unchecked() }, png);
        drop(registration);
    }

    fn wait(mut operation: NativeOpen) -> Result<(), NativeServiceError> {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Poll::Ready(result) = Pin::new(&mut operation).poll(&mut context) {
                return result;
            }
            assert!(Instant::now() < until, "native opener did not complete");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn url_is_parsed_and_rejects_schemes_credentials_controls_and_long_inputs() {
        for url in [
            "javascript:alert(1)",
            "file:///tmp/test",
            "https://user:password@example.test/",
            "https://",
            "https://example.test/\n",
            "https://example.test\\@evil.test/",
            "https://example.test/ ",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url(&format!("https://example.test/{}", "a".repeat(2048))).is_err());
        assert_eq!(
            validate_url("https://example.test/path").unwrap(),
            "https://example.test/path"
        );
        let (services, _events, _owner) = services();
        assert!(matches!(
            services.open_url("custom://app"),
            Err(NativeServiceError::InvalidUrl)
        ));
    }

    #[test]
    fn document_is_checked_on_worker_and_never_accepts_missing_or_symlink_files() {
        let (services, _events, _owner) = services();
        assert!(matches!(
            services.open_document("relative.txt"),
            Err(NativeServiceError::InvalidDocument)
        ));
        let path = std::env::temp_dir().join("webui-native-service-file-does-not-exist");
        assert!(matches!(
            wait(services.open_document(&path).unwrap()),
            Err(NativeServiceError::InvalidDocument)
        ));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("log.txt");
        std::fs::write(&file, b"controlled fixture").unwrap();
        assert_eq!(
            validate_document(&file).unwrap(),
            file.canonicalize().unwrap()
        );
        let executable = dir.path().join("unsafe.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        assert!(matches!(
            validate_document(&executable),
            Err(NativeServiceError::InvalidDocument)
        ));
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(matches!(
                validate_document(&link),
                Err(NativeServiceError::InvalidDocument)
            ));
        }
    }

    #[test]
    fn theme_requires_attached_supported_window_and_host_lifetime() {
        let (services, _events, owner) = services();
        #[cfg(target_os = "macos")]
        {
            assert!(matches!(
                services.current_theme(),
                Err(NativeServiceError::ThemeUnavailable)
            ));
            assert!(matches!(
                services.set_theme(ThemeMode::Dark),
                Err(NativeServiceError::ThemeUnavailable)
            ));
            assert_eq!(
                ThemeState {
                    mode: ThemeMode::System,
                    dark: false
                }
                .mode,
                ThemeMode::System
            );
            assert_ne!(ThemeMode::Light, ThemeMode::Dark);
            drop(owner);
            assert!(matches!(
                services.current_theme(),
                Err(NativeServiceError::Closed)
            ));
            assert!(matches!(
                services.set_theme(ThemeMode::Light),
                Err(NativeServiceError::Closed)
            ));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = owner;
            assert!(matches!(
                services.current_theme(),
                Err(NativeServiceError::ThemeUnsupported)
            ));
            assert!(matches!(
                services.set_theme(ThemeMode::System),
                Err(NativeServiceError::ThemeUnsupported)
            ));
        }
    }

    #[test]
    fn queue_completion_and_navigation_cancellation_are_distinct() {
        let (services, events, _owner) = services();
        let (release, receiver) = std::sync::mpsc::channel();
        let pending = services
            .start(move || {
                let _ = receiver.recv_timeout(Duration::from_secs(2));
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            services.start(|| Ok(())),
            Err(NativeServiceError::Busy)
        ));
        let _ = events.dispatch(&DesktopEvent::NavigationRequested {
            window_id: crate::WindowId::PRIMARY,
            url: "http://127.0.0.1/next".into(),
        });
        assert!(matches!(wait(pending), Err(NativeServiceError::Cancelled)));
        // Queued work may have been cancelled before it acquired a worker.
        let _ = release.send(());
        let until = Instant::now() + Duration::from_secs(2);
        let completed = loop {
            match services.start(|| Ok(())) {
                Ok(completed) => break completed,
                Err(NativeServiceError::Busy) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("unexpected scheduling error: {error}"),
            }
        };
        assert!(wait(completed).is_ok());
        let _ = events.dispatch(&DesktopEvent::WindowClosed {
            window_id: crate::WindowId::PRIMARY,
        });
        assert!(matches!(
            services.start(|| Ok(())),
            Err(NativeServiceError::Closed)
        ));
    }

    #[test]
    fn os_handoff_has_a_deadline_even_after_worker_admission() {
        let (services, _events, _owner) = services();
        let (release, receiver) = std::sync::mpsc::channel();
        let mut pending = services
            .start(move || {
                let _ = receiver.recv_timeout(Duration::from_secs(2));
                Ok(())
            })
            .unwrap();
        pending.deadline = Instant::now();
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut pending).poll(&mut context),
            Poll::Ready(Err(NativeServiceError::Deadline))
        ));
        drop(pending);
        let _ = release.send(());
    }

    #[test]
    fn failed_worker_admission_never_runs_os_work_or_retains_busy_permit() {
        let (services, _events, _owner) = services();
        services.0.executor.close();
        let called = Arc::new(AtomicBool::new(false));
        let worker_called = Arc::clone(&called);
        assert!(matches!(
            services.start(move || {
                worker_called.store(true, Ordering::Release);
                Ok(())
            }),
            Err(NativeServiceError::Closed)
        ));
        assert!(!called.load(Ordering::Acquire));
        assert!(!services.0.busy.load(Ordering::Acquire));
    }

    #[test]
    fn deadline_thread_backpressure_precedes_worker_admission() {
        let (services, _events, _owner) = services();
        services
            .0
            .active_timers
            .store(MAX_DEADLINE_THREADS_PER_WINDOW, Ordering::Release);
        let called = Arc::new(AtomicBool::new(false));
        let worker_called = Arc::clone(&called);
        assert!(matches!(
            services.start(move || {
                worker_called.store(true, Ordering::Release);
                Ok(())
            }),
            Err(NativeServiceError::Overloaded)
        ));
        assert!(!called.load(Ordering::Acquire));
        assert!(!services.0.busy.load(Ordering::Acquire));
        services.0.active_timers.store(0, Ordering::Release);
    }

    #[test]
    fn frame_service_handle_is_cached_and_retired_on_drop() {
        let (owner, lifetime) = HostLifetime::new();
        let options = crate::LocalServerOptions::new(
            crate::LoopbackOrigin::from_socket_addr("127.0.0.1:23456".parse().unwrap()).unwrap(),
            lifetime,
        );
        let frame = crate::DesktopApp::from_local_server(options)
            .build()
            .unwrap();
        let services = frame.native_services().unwrap();
        assert!(Arc::ptr_eq(
            &services.0,
            &frame.native_services().unwrap().0
        ));
        drop(frame);
        assert!(matches!(
            services.open_url("https://example.test/"),
            Err(NativeServiceError::Closed)
        ));
        drop(owner);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn geometry_requires_committed_document_and_tracks_native_revision() {
        let (services, events, _owner) = services();
        assert!(matches!(
            services.content_geometry(),
            Err(NativeServiceError::GeometryUnavailable)
        ));
        services.provisional_started();
        let epoch = services.0.document_epoch.load(Ordering::Acquire);
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), 0);
        let revision = services.0.geometry_revision.load(Ordering::Acquire);
        services.navigation_finished("http://127.0.0.1/document");
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), epoch);
        let _ = events.dispatch(&DesktopEvent::WindowMoved {
            window_id: crate::WindowId::PRIMARY,
            x: 100,
            y: 200,
        });
        assert!(services.0.geometry_revision.load(Ordering::Acquire) > revision);
        let _ = events.dispatch(&DesktopEvent::NavigationRequested {
            window_id: crate::WindowId::PRIMARY,
            url: "http://127.0.0.1/next".into(),
        });
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), epoch);
        services.provisional_started();
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), 0);
        services.close();
        assert!(services.0.closed.load(Ordering::Acquire));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn geometry_is_explicitly_unsupported_without_a_mac_window() {
        let (services, _events, _owner) = services();
        assert!(matches!(
            services.content_geometry(),
            Err(NativeServiceError::Unsupported)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn prevented_request_does_not_retire_the_finished_geometry_document() {
        let (services, events, _owner) = services();
        services.provisional_started();
        services.navigation_finished("http://127.0.0.1/current");
        let committed = services.0.committed_epoch.load(Ordering::Acquire);
        assert_ne!(committed, 0);
        let (release, receiver) = std::sync::mpsc::channel();
        let pending_open = services
            .start(move || {
                let _ = receiver.recv_timeout(Duration::from_secs(2));
                Ok(())
            })
            .unwrap();
        events
            .on_event(|event| {
                if matches!(event, DesktopEvent::NavigationRequested { .. }) {
                    EventResponse::PreventDefault
                } else {
                    EventResponse::Continue
                }
            })
            .unwrap();
        let response = events.dispatch(&DesktopEvent::NavigationRequested {
            window_id: crate::WindowId::PRIMARY,
            url: "http://127.0.0.1/prevented".into(),
        });
        assert_eq!(response, EventResponse::PreventDefault);
        assert_eq!(
            services.0.committed_epoch.load(Ordering::Acquire),
            committed,
            "a denied request never started a native navigation"
        );
        assert_eq!(services.0.document_epoch.load(Ordering::Acquire), committed);
        assert_ne!(
            services.0.generation.load(Ordering::Acquire),
            0,
            "OS openers still cancel on navigation requests"
        );
        assert!(matches!(
            wait(pending_open),
            Err(NativeServiceError::Cancelled)
        ));
        let _ = release.send(());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn failed_provisional_restores_only_the_verified_old_document() {
        let (services, _events, _owner) = services();
        services.provisional_started();
        services.navigation_finished("http://127.0.0.1/current");
        let old_epoch = services.0.document_epoch.load(Ordering::Acquire);
        services.provisional_started();
        let next = services.0.document_epoch.load(Ordering::Acquire);
        assert!(next > old_epoch);
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), 0);
        assert!(!services.provisional_failed(Some("http://127.0.0.1/other"), false));
        assert!(!services.provisional_failed(Some("http://127.0.0.1/current"), true));
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), 0);
        assert!(services.provisional_failed(Some("http://127.0.0.1/current"), false));
        assert_eq!(services.0.committed_epoch.load(Ordering::Acquire), next);
        services.provisional_started();
        services.navigation_committed();
        assert!(!services.provisional_failed(Some("http://127.0.0.1/current"), false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn geometry_result_rejects_later_move_and_navigation() {
        let (services, events, _owner) = services();
        services.provisional_started();
        services.navigation_finished("http://127.0.0.1/document");
        let epoch = services.0.document_epoch.load(Ordering::Acquire);
        let revision = services.0.geometry_revision.load(Ordering::Acquire);
        let slot = Arc::new(platform::GeometrySlot::test_slot());
        slot.complete(Ok(ContentGeometry {
            window_generation: services.0.window_generation,
            document_epoch: epoch,
            revision,
            screen_rect: ScreenRectPoints {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
            },
            backing_scale: 2.0,
            page_zoom: 1.0,
            magnification: 1.0,
        }));
        let mut request = GeometryRequest {
            inner: Arc::clone(&services.0),
            epoch,
            slot,
        };
        let _ = events.dispatch(&DesktopEvent::WindowResized {
            window_id: crate::WindowId::PRIMARY,
            width: 300,
            height: 200,
        });
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut request).poll(&mut context),
            Poll::Ready(Err(NativeServiceError::Stale))
        ));
        let _ = events.dispatch(&DesktopEvent::NavigationRequested {
            window_id: crate::WindowId::PRIMARY,
            url: "http://127.0.0.1/next".into(),
        });
        services.provisional_started();
        assert!(matches!(
            Pin::new(&mut request).poll(&mut context),
            Poll::Ready(Err(NativeServiceError::Stale))
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn opt_in_system_opener_fixture_on_macos_arm64() {
        // Explicit opt-in: opening a browser/document must not surprise ordinary
        // `cargo test` users or disclose any data from a personal file.
        if std::env::var_os("WEBUI_NATIVE_SERVICES_FIXTURE").is_none() {
            return;
        }
        let (services, _events, _owner) = services();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("webui-controlled-fixture.txt");
        std::fs::write(&file, b"WebUI native opener test, no personal content.").unwrap();
        assert!(wait(services.open_document(&file).unwrap()).is_ok());
        // Local-only destination, no authentication or request body.
        assert!(wait(
            services
                .open_url("http://127.0.0.1:9/webui-native-fixture")
                .unwrap()
        )
        .is_ok());
    }
}
#[cfg(target_os = "windows")]
#[path = "windows/services.rs"]
#[allow(unsafe_code)]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux/services.rs"]
mod platform;
