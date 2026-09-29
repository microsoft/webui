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

/// Maximum UTF-8 URL input, in bytes.
pub const MAX_NATIVE_URL_BYTES: usize = 2048;
/// Maximum local path representation, in bytes (not the document's size).
pub const MAX_NATIVE_DOCUMENT_PATH_BYTES: usize = 4096;
const OPEN_DEADLINE: Duration = Duration::from_secs(10);
// An OS opener can return before its timer thread has observed the shutdown
// signal. Bound those short-lived leftovers even under a rapid host retry loop.
const MAX_DEADLINE_THREADS_PER_WINDOW: usize = 8;

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
}

struct Inner {
    executor: Arc<ApplicationExecutor>,
    lifetime: HostLifetime,
    closed: AtomicBool,
    generation: AtomicU64,
    busy: AtomicBool,
    active_timers: std::sync::atomic::AtomicUsize,
    timer: Mutex<Option<Weak<Timer>>>,
    registration: Mutex<Option<EventSubscription>>,
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
        let inner = Arc::new(Inner {
            executor,
            lifetime,
            closed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            busy: AtomicBool::new(false),
            active_timers: std::sync::atomic::AtomicUsize::new(0),
            timer: Mutex::new(None),
            registration: Mutex::new(None),
        });
        let weak = Arc::downgrade(&inner);
        let subscription = events.subscribe(move |event| {
            if let Some(inner) = weak.upgrade() {
                match event {
                    DesktopEvent::NavigationRequested { .. } => inner.cancel(false),
                    DesktopEvent::WindowClosed { .. } | DesktopEvent::Exiting => inner.cancel(true),
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
    /// queued work; an OS launch already handed off cannot be recalled.
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
        self.0.cancel(true);
        if let Ok(mut registration) = self.0.registration.lock() {
            registration.take();
        }
    }
}

impl Inner {
    fn cancel(&self, close: bool) {
        if close {
            self.closed.store(true, Ordering::Release);
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(timer) = self.timer.lock() {
            if let Some(timer) = timer.as_ref().and_then(Weak::upgrade) {
                timer.wake();
            }
        }
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
