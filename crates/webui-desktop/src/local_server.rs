// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Direct loopback HTTP content for an opt-in native window.

#[cfg(feature = "application-ipc")]
use std::net::TcpListener;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[cfg(all(
    feature = "application-ipc",
    any(target_os = "macos", target_os = "windows")
))]
use crate::ipc::{IpcBridge, IpcWindow};
#[cfg(feature = "application-ipc")]
use crate::ipc::{IpcHost, IpcOptions, IpcRegistry, IpcWindowOwner};
use crate::{
    DesktopError, DesktopEvent, DesktopShellConfig, EventRegistrationError, EventRegistry,
    EventResponse, EventSubscription, Result, WindowHandle, WindowOptions,
};

#[cfg(feature = "application-ipc")]
mod owned_ipc;
#[cfg(feature = "application-ipc")]
pub use owned_ipc::bind_owned_local_server;
#[cfg(feature = "application-ipc")]
pub(crate) use owned_ipc::OwnedLocalServerIpc;

/// Suggested static URL for hosts that choose to mount the matching embedded
/// local-only browser runtime. This is an asset route, never an IPC endpoint.
#[cfg(all(
    feature = "application-ipc",
    any(target_os = "macos", target_os = "windows")
))]
pub const LOCAL_IPC_RUNTIME_PATH: &str = "/_webui/ipc/local-runtime.js";

/// Borrow the dedicated local IPC browser bundle without copying its bytes.
///
/// Hosts may serve these bytes at [`LOCAL_IPC_RUNTIME_PATH`] with
/// `text/javascript; charset=utf-8`, or bundle
/// `@microsoft/webui-desktop/native` in their own application assets instead.
/// The SDK does not proxy or intercept the host's listener. This file
/// contains no session credentials; application IPC messages stay on the
/// separate private native lane.
#[cfg(all(
    feature = "application-ipc",
    any(target_os = "macos", target_os = "windows")
))]
#[must_use]
pub fn local_ipc_runtime_asset() -> &'static [u8] {
    crate::ipc_assets::LOCAL_BROWSER_RUNTIME
}

#[derive(Clone)]
enum CloseCallback {
    #[cfg(any(target_os = "macos", test))]
    Native(Arc<dyn Fn() + Send + Sync>),
    #[cfg(any(target_os = "windows", target_os = "linux", test))]
    Fallible(Arc<dyn Fn() -> std::result::Result<(), HostCloseError> + Send + Sync>),
}

impl CloseCallback {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            #[cfg(any(target_os = "macos", test))]
            (Self::Native(left), Self::Native(right)) => Arc::ptr_eq(left, right),
            #[cfg(any(target_os = "windows", target_os = "linux", test))]
            (Self::Fallible(left), Self::Fallible(right)) => Arc::ptr_eq(left, right),
            #[cfg(test)]
            _ => false,
        }
    }

    fn dispatch(&self) -> std::result::Result<(), HostCloseError> {
        match self {
            #[cfg(any(target_os = "macos", test))]
            Self::Native(callback) => {
                callback();
                Ok(())
            }
            #[cfg(any(target_os = "windows", target_os = "linux", test))]
            Self::Fallible(callback) => callback(),
        }
    }
}

/// Failure to notify the native window that its verified host retired.
#[derive(Debug, thiserror::Error)]
pub enum HostCloseError {
    /// The host has not yet revoked this lifetime.
    #[error("host lifetime is still active; help: call revoke() before retry_close()")]
    StillActive,
    /// The native window could not be woken; admission remains retired.
    #[error("native close wake failed: {message}; help: keep the listener bound and retry_close() until WindowClosed")]
    WakeFailed {
        /// Native failure reported while scheduling the close.
        message: String,
    },
}

struct HostLifetimeInner {
    active: AtomicBool,
    close: Mutex<Option<CloseCallback>>,
    close_failed: AtomicBool,
}

/// Weak, host-revocable admission signal for one local HTTP window.
///
/// The trusted Rust host must retain the matching [`HostLifetimeOwner`] for
/// the listener/attached connection's lifetime. This is not authentication:
/// construct it only after verifying the existing server or daemon identity.
/// An external daemon can crash and release its port before the host observes
/// that loss; this signal alone cannot prove identity after such a race.
#[derive(Clone)]
pub struct HostLifetime(Weak<HostLifetimeInner>);

/// Owner of the host-provided local-server retirement signal.
///
/// Dropping or revoking this owner permanently retires its frame. For an
/// attached daemon, own only the verified connection's lifetime: dropping
/// this handle never terminates the daemon or acquires its listener. An
/// unexpected external crash may precede observation and revocation.
#[must_use = "retain the owner until the verified server or attached daemon disconnects"]
pub struct HostLifetimeOwner(Arc<HostLifetimeInner>);

/// Native close registration, owned by the active window, not by the host.
pub(crate) struct HostCloseRegistration {
    lifetime: HostLifetime,
    callback: CloseCallback,
}

impl Drop for HostCloseRegistration {
    fn drop(&mut self) {
        if let Some(inner) = self.lifetime.0.upgrade() {
            let mut close = inner
                .close
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if close
                .as_ref()
                .is_some_and(|current| current.same(&self.callback))
            {
                close.take();
            }
        }
    }
}

impl HostLifetime {
    /// Create a one-owner lifetime and a weak frame capability.
    #[must_use = "retain the returned owner until its local-server window closes"]
    pub fn new() -> (HostLifetimeOwner, Self) {
        let inner = Arc::new(HostLifetimeInner {
            active: AtomicBool::new(true),
            close: Mutex::new(None),
            close_failed: AtomicBool::new(false),
        });
        (
            HostLifetimeOwner(Arc::clone(&inner)),
            Self(Arc::downgrade(&inner)),
        )
    }

    /// Whether the verified owner is still present and has not revoked access.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|inner| inner.active.load(Ordering::Acquire))
    }

    pub(crate) fn allows_navigation(&self, origin: &LoopbackOrigin, url: &str) -> bool {
        self.is_active() && origin.allows(url)
    }

    pub(crate) fn require_active(&self) -> Result<()> {
        if self.is_active() {
            Ok(())
        } else {
            Err(retired_host())
        }
    }

    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn register_close(
        &self,
        callback: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<HostCloseRegistration> {
        self.install_close(CloseCallback::Native(callback))
    }

    #[cfg(any(target_os = "windows", target_os = "linux", test))]
    pub(crate) fn register_close_fallible(
        &self,
        callback: Arc<dyn Fn() -> std::result::Result<(), HostCloseError> + Send + Sync>,
    ) -> Result<HostCloseRegistration> {
        self.install_close(CloseCallback::Fallible(callback))
    }

    fn install_close(&self, callback: CloseCallback) -> Result<HostCloseRegistration> {
        let inner = self.0.upgrade().ok_or_else(retired_host)?;
        let mut close = inner
            .close
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !inner.active.load(Ordering::Acquire) {
            return Err(retired_host());
        }
        if close.is_some() {
            return Err(DesktopError::UnsupportedRuntime {
                message: "this host lifetime already owns a running desktop window".to_string(),
                help: "Create one HostLifetime per local-server window".to_string(),
            });
        }
        *close = Some(callback.clone());
        Ok(HostCloseRegistration {
            lifetime: self.clone(),
            callback,
        })
    }
}

impl HostLifetimeOwner {
    /// Atomically retire this host-provided admission signal and schedule close.
    ///
    /// Idempotent and nonblocking with respect to the UI thread. Invoke this
    /// before releasing a listener or discarding a verified daemon connection.
    ///
    /// # Errors
    ///
    /// Returns [`HostCloseError::WakeFailed`] if the native close notification
    /// could not be queued. Admission is still permanently retired; keep the
    /// listener bound and call [`Self::retry_close`] deliberately.
    pub fn revoke(&self) -> std::result::Result<(), HostCloseError> {
        if !self.0.active.swap(false, Ordering::AcqRel) {
            return if self.0.close_failed.load(Ordering::Acquire) {
                self.dispatch_close()
            } else {
                Ok(())
            };
        }
        self.dispatch_close()
    }

    /// Retry one failed native close wake after revocation.
    ///
    /// This does not reactivate the frame. The host must retain its listener
    /// and owner until it observes `WindowClosed`; retries are deliberate,
    /// not a background loop or a shutdown acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns [`HostCloseError`] if the owner is still active or native wake
    /// delivery fails again.
    pub fn retry_close(&self) -> std::result::Result<(), HostCloseError> {
        if self.0.active.load(Ordering::Acquire) {
            return Err(HostCloseError::StillActive);
        }
        self.dispatch_close()
    }

    fn dispatch_close(&self) -> std::result::Result<(), HostCloseError> {
        let close = self
            .0
            .close
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let result = close.as_ref().map_or(Ok(()), CloseCallback::dispatch);
        self.0
            .close_failed
            .store(result.is_err(), Ordering::Release);
        result
    }
}

impl Drop for HostLifetimeOwner {
    fn drop(&mut self) {
        if let Err(error) = self.revoke() {
            eprintln!(
                "WebUI: host lifetime retired but the native close could not be scheduled: {error}"
            );
        }
    }
}

#[cold]
#[inline(never)]
pub(crate) fn retired_host() -> DesktopError {
    DesktopError::UnsupportedRuntime {
        message: "the verified local-server owner has retired".to_string(),
        help: "Keep the authenticated host owner alive until the window closes; create a new frame only after verifying the replacement server".to_string(),
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn should_close_for_cookie(
    lifetime: Option<&HostLifetime>,
    window_cookie: Option<usize>,
    received: usize,
) -> bool {
    window_cookie.is_some_and(|cookie| cookie != 0 && cookie == received)
        && lifetime.is_some_and(|lifetime| !lifetime.is_active())
}

/// Canonical HTTP origin of an already-bound loopback listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopbackOrigin(String);

impl LoopbackOrigin {
    /// Form an exact IP-literal origin from the bound socket address.
    ///
    /// # Errors
    ///
    /// Rejects non-loopback, wildcard, or zero-port addresses. The caller
    /// must verify and retain ownership of the actual listener separately.
    pub fn from_socket_addr(address: SocketAddr) -> Result<Self> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(invalid_local_server(
                "origin must be a bound loopback IP and nonzero port",
            ));
        }
        let host = match address.ip() {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        Ok(Self(format!("http://{host}:{}", address.port())))
    }

    /// The canonical scheme and authority, without a trailing slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn allows(&self, url: &str) -> bool {
        url.strip_prefix(&self.0).is_some_and(|path| {
            (path.is_empty() || path.starts_with('/'))
                && !url.contains('\\')
                && !url.bytes().any(|byte| byte.is_ascii_control())
        })
    }

    #[cfg(all(feature = "application-ipc", target_os = "macos"))]
    pub(crate) fn matches_security_origin(&self, scheme: &str, host: &str, port: isize) -> bool {
        if scheme != "http" || port <= 0 {
            return false;
        }
        let Ok(ip) = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
        else {
            return false;
        };
        let Ok(port) = u16::try_from(port) else {
            return false;
        };
        LoopbackOrigin::from_socket_addr(SocketAddr::new(ip, port))
            .is_ok_and(|origin| origin == *self)
    }
}

/// Startup URL configuration for a server the trusted host already owns.
pub struct LocalServerOptions {
    origin: LoopbackOrigin,
    path: String,
    lifetime: HostLifetime,
}

impl LocalServerOptions {
    /// Start at `/` on the supplied exact HTTP origin.
    #[must_use]
    pub fn new(origin: LoopbackOrigin, lifetime: HostLifetime) -> Self {
        Self {
            origin,
            path: "/".to_string(),
            lifetime,
        }
    }

    /// Choose a path and optional query on that origin.
    ///
    /// # Errors
    ///
    /// Rejects authority-relative paths, fragments, backslashes, control
    /// characters and empty paths.
    pub fn initial_path(mut self, path_and_query: &str) -> Result<Self> {
        if !path_and_query.starts_with('/')
            || path_and_query.starts_with("//")
            || path_and_query.contains(['#', '\\'])
            || path_and_query.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(invalid_local_server("initial path must be a single-rooted path with optional query, without fragment or control characters"));
        }
        self.path = path_and_query.to_string();
        Ok(self)
    }

    pub(crate) fn url(&self) -> String {
        let mut url = String::with_capacity(self.origin.0.len() + self.path.len());
        url.push_str(&self.origin.0);
        url.push_str(&self.path);
        url
    }
}

#[cold]
#[inline(never)]
fn invalid_local_server(message: &str) -> DesktopError {
    DesktopError::UnsupportedRuntime {
        message: message.to_string(),
        help: "Supply the verified address of a bound loopback HTTP listener and a root-relative startup path".to_string(),
    }
}

/// Builder for a native window backed directly by the existing HTTP server.
pub struct LocalServerAppBuilder {
    options: LocalServerOptions,
    window: WindowOptions,
    shell: DesktopShellConfig,
    app_id: Option<String>,
    #[cfg(feature = "application-ipc")]
    ipc: Option<(OwnedLocalServerIpc, IpcRegistry, IpcOptions)>,
}

impl LocalServerAppBuilder {
    pub(crate) fn new(options: LocalServerOptions) -> Self {
        Self {
            options,
            window: WindowOptions::default(),
            shell: DesktopShellConfig::default(),
            app_id: None,
            #[cfg(feature = "application-ipc")]
            ipc: None,
        }
    }

    /// Configure native window presentation.
    #[must_use]
    pub fn window(mut self, window: WindowOptions) -> Self {
        self.window = window;
        self
    }

    /// Configure native shell presentation.
    #[must_use]
    pub fn shell(mut self, shell: DesktopShellConfig) -> Self {
        self.shell = shell;
        self
    }

    /// Set a stable identity for geometry and platform browser-profile selection.
    #[must_use]
    pub fn app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Enable generated application IPC only for this exact owned listener.
    ///
    /// The host must own and retain the listener and its lifetime owner until
    /// the window closes. Passing only an address or attached-daemon loss
    /// signal cannot grant IPC. The SDK duplicates the socket so an accidental
    /// early drop cannot permit another process to rebind the IPC origin.
    ///
    /// # Errors
    ///
    /// Rejects a listener on another address or a retired host.
    #[cfg(feature = "application-ipc")]
    pub fn application_ipc(
        mut self,
        listener: &TcpListener,
        registry: IpcRegistry,
        options: IpcOptions,
    ) -> Result<Self> {
        let owned = OwnedLocalServerIpc::from_listener(
            listener,
            &self.options.origin,
            &self.options.lifetime,
        )?;
        self.ipc = Some((owned, registry, options));
        Ok(self)
    }

    /// Build the owning window without contacting or authenticating the server.
    ///
    /// # Errors
    ///
    /// Rejects unsupported window or shell options on the selected platform.
    pub fn build(self) -> Result<LocalServerFrame> {
        self.options.lifetime.require_active()?;
        let capabilities = crate::frame::local_platform_capabilities();
        crate::validate_frame_capabilities(&self.window, &self.shell, capabilities)?;
        if !matches!(self.window.titlebar, crate::TitlebarStyle::Native) {
            return Err(DesktopError::UnsupportedRuntime {
                message: "local-server frames currently require a native titlebar".to_string(),
                help: "Use TitlebarStyle::Native; custom titlebar presentation is not wired to local HTTP documents".to_string(),
            });
        }
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return Err(DesktopError::UnsupportedRuntime {
                message: "local-server native frames are not supported on this target".to_string(),
                help: "Use macOS, Windows or Linux with a native webview runtime".to_string(),
            });
        }
        #[cfg(feature = "application-ipc")]
        let ipc_owner = self
            .ipc
            .map(|(owned, registry, options)| {
                IpcWindowOwner::new(
                    Arc::new(registry),
                    options,
                    IpcHost::LocalOwned(Arc::new(owned)),
                )
            })
            .transpose()?;
        let live_background = std::sync::Arc::default();
        let frame_policy = crate::frame_policy::FramePolicy::new(self.options.lifetime.clone());
        Ok(LocalServerFrame {
            options: self.options,
            window: self.window,
            #[cfg(target_os = "macos")]
            shell: self.shell,
            app_id: self.app_id,
            events: EventRegistry::default(),
            window_handle: WindowHandle::with_background(std::sync::Arc::clone(&live_background)),
            live_background,
            frame_policy,
            #[cfg(feature = "native")]
            executor: std::sync::Arc::default(),
            #[cfg(feature = "native-services")]
            native_services: Mutex::new(None),
            #[cfg(feature = "application-ipc")]
            ipc_owner,
        })
    }
}

/// Owner of the local-server window, its event callbacks and native commands.
///
/// The trusted host must retain its verified listener for this frame's entire
/// lifetime. The frame does not authenticate the HTTP server or expose
/// page-originated native window controls. Application IPC remains disabled
/// unless the builder was explicitly given the owned listener and generated
/// grants.
pub struct LocalServerFrame {
    pub(crate) options: LocalServerOptions,
    pub(crate) window: WindowOptions,
    #[cfg(target_os = "macos")]
    pub(crate) shell: DesktopShellConfig,
    pub(crate) app_id: Option<String>,
    pub(crate) events: EventRegistry,
    pub(crate) window_handle: WindowHandle,
    pub(crate) live_background: std::sync::Arc<crate::window::LiveBackground>,
    pub(crate) frame_policy: std::sync::Arc<crate::frame_policy::FramePolicy>,
    #[cfg(feature = "native")]
    pub(crate) executor: std::sync::Arc<crate::execution::ApplicationExecutor>,
    #[cfg(feature = "native-services")]
    pub(crate) native_services: Mutex<Option<crate::native_services::NativeServices>>,
    #[cfg(feature = "application-ipc")]
    pub(crate) ipc_owner: Option<IpcWindowOwner>,
}

impl LocalServerFrame {
    /// Borrow a weak handle for exact unprivileged local iframe origins.
    ///
    /// A grant never makes a subframe a native IPC or window-control principal.
    #[must_use]
    pub fn frame_policy(&self) -> crate::FramePolicyHandle {
        self.frame_policy.handle()
    }

    /// Obtain trusted-host OS openers for this window. No renderer global or
    /// IPC grant is installed; retaining the handle does not keep the window alive.
    ///
    /// # Errors
    ///
    /// Fails if the event registry cannot install lifecycle cancellation.
    #[cfg(feature = "native-services")]
    pub fn native_services(
        &self,
    ) -> std::result::Result<crate::NativeServices, crate::NativeServiceError> {
        let mut cached = self
            .native_services
            .lock()
            .map_err(|_| crate::NativeServiceError::Unavailable)?;
        if let Some(services) = cached.as_ref() {
            return Ok(services.clone());
        }
        let services = crate::NativeServices::new(
            &self.events,
            std::sync::Arc::clone(&self.executor),
            self.lifetime().clone(),
        )?;
        *cached = Some(services.clone());
        Ok(services)
    }

    /// Borrow the weak application IPC handle, if explicitly enabled.
    #[cfg(all(
        feature = "application-ipc",
        any(target_os = "macos", target_os = "windows")
    ))]
    #[must_use]
    pub fn ipc(&self) -> Option<IpcWindow> {
        self.ipc_owner.as_ref().map(IpcWindowOwner::window)
    }

    #[cfg(all(
        feature = "application-ipc",
        any(target_os = "macos", target_os = "windows")
    ))]
    pub(crate) fn ipc_bridge(&self) -> Option<IpcBridge> {
        self.ipc_owner.as_ref().map(IpcWindowOwner::bridge)
    }
    /// Borrow the canonical HTTP origin.
    #[must_use]
    pub fn origin(&self) -> &LoopbackOrigin {
        &self.options.origin
    }

    pub(crate) fn lifetime(&self) -> &HostLifetime {
        &self.options.lifetime
    }

    /// Borrow the native command handle.
    #[must_use]
    pub fn window_handle(&self) -> &WindowHandle {
        &self.window_handle
    }

    /// Register a non-blocking event handler for this window's lifetime.
    ///
    /// # Errors
    ///
    /// Returns an error if registration is closed or full.
    pub fn on_event<F>(&self, handler: F) -> std::result::Result<(), EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.events.on_event(handler)
    }

    /// Register an event handler until the returned subscription is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if registration is closed or full.
    pub fn subscribe<F>(
        &self,
        handler: F,
    ) -> std::result::Result<EventSubscription, EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.events.subscribe(handler)
    }
}

impl Drop for LocalServerFrame {
    fn drop(&mut self) {
        self.frame_policy.close();
        #[cfg(feature = "native-services")]
        {
            if let Ok(mut services) = self.native_services.lock() {
                if let Some(services) = services.take() {
                    services.close();
                }
            }
        }
        #[cfg(feature = "application-ipc")]
        if let Some(owner) = &self.ipc_owner {
            owner.close();
        }
        #[cfg(feature = "native")]
        self.executor.close();
        self.window_handle.close();
        self.events.close();
    }
}

/// Run the frame on the OS UI thread until its window closes.
///
/// Returning (including on startup error) means the SDK's duplicate listener
/// pin has been released after terminal IPC retirement. The caller may then
/// wait for HTTP listener quiescence before releasing its original listener;
/// never wait for quiescence before the native window has closed.
///
/// # Errors
///
/// Returns a typed error for unsupported platforms or native launch failure.
pub fn run_local_server_frame(frame: LocalServerFrame) -> Result<()> {
    frame.lifetime().require_active()?;
    crate::frame::platform_run_local_server_frame(frame)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[cfg(all(
        feature = "application-ipc",
        any(target_os = "macos", target_os = "windows")
    ))]
    #[test]
    fn local_runtime_is_explicit_and_never_substituted_for_the_default_bundle() {
        assert_eq!(LOCAL_IPC_RUNTIME_PATH, "/_webui/ipc/local-runtime.js");
        let local = std::str::from_utf8(local_ipc_runtime_asset()).unwrap();
        assert!(local.contains("createNativeDesktopTransport"));
        assert!(local.contains("nativeCarrierVersion"));
        let bundled = std::str::from_utf8(crate::ipc_assets::BROWSER_RUNTIME).unwrap();
        assert!(!bundled.contains("createNativeDesktopTransport"));
    }

    #[cfg(all(feature = "application-ipc", target_os = "macos"))]
    #[test]
    fn wk_security_origin_must_match_the_bound_ip_and_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = LoopbackOrigin::from_socket_addr(address).unwrap();
        let port = isize::try_from(u32::from(address.port())).unwrap();
        assert!(origin.matches_security_origin("http", "127.0.0.1", port));
        assert!(!origin.matches_security_origin("http", "127.0.0.2", port));
        assert!(!origin.matches_security_origin("https", "127.0.0.1", port));
        assert!(!origin.matches_security_origin("http", "127.0.0.1", port + 1));
    }

    #[test]
    fn origin_requires_bound_loopback_literal() {
        assert!(LoopbackOrigin::from_socket_addr("0.0.0.0:80".parse().unwrap()).is_err());
        assert!(LoopbackOrigin::from_socket_addr("127.0.0.1:0".parse().unwrap()).is_err());
        assert!(LoopbackOrigin::from_socket_addr("192.0.2.1:80".parse().unwrap()).is_err());
        assert_eq!(
            LoopbackOrigin::from_socket_addr("[::1]:443".parse().unwrap())
                .unwrap()
                .as_str(),
            "http://[::1]:443"
        );
    }

    #[test]
    fn startup_path_cannot_escape_origin() {
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        for path in ["//evil/", "http://evil/", "/x#hash", "/\\evil", "/\n", ""] {
            assert!(LocalServerOptions::new(origin.clone(), lifetime.clone())
                .initial_path(path)
                .is_err());
        }
        let options = LocalServerOptions::new(origin.clone(), lifetime)
            .initial_path("/app?q=a")
            .unwrap();
        assert_eq!(options.url(), "http://127.0.0.1:3456/app?q=a");
        for url in [
            "http://127.0.0.1:3456.evil/",
            "http://127.0.0.1:34560/",
            "http://user@127.0.0.1:3456/",
            "https://127.0.0.1:3456/",
            "http://127.0.0.1:3456\\@evil/",
        ] {
            assert!(!origin.allows(url), "{url}");
        }
        assert!(origin.allows("http://127.0.0.1:3456/deep?q=a"));
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn builder_owns_window_shell_events_and_commands_without_a_protocol() {
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        let frame = crate::DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
            .app_id("com.example.local-smoke")
            .window(WindowOptions {
                title: "HTTP".to_string(),
                ..WindowOptions::default()
            })
            .shell(DesktopShellConfig::default())
            .build()
            .unwrap();
        assert_eq!(frame.app_id.as_deref(), Some("com.example.local-smoke"));
        assert_eq!(frame.window.title, "HTTP");
        assert_eq!(frame.origin().as_str(), "http://127.0.0.1:3456");
        frame.on_event(|_| EventResponse::Continue).unwrap();
        let subscription = frame.subscribe(|_| EventResponse::Continue).unwrap();
        assert!(frame.window_handle().set_title("HTTP ready").is_ok());
        drop(subscription);
        drop(frame);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn local_server_rejects_unwired_custom_titlebar() {
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        let result =
            crate::DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime))
                .window(WindowOptions {
                    titlebar: crate::TitlebarStyle::Overlay { height: 48 },
                    ..WindowOptions::default()
                })
                .build();
        assert!(matches!(
            result,
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
    }

    #[test]
    fn revocation_between_navigation_start_and_commit_blocks_the_same_origin() {
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let (owner, lifetime) = HostLifetime::new();
        let url = "http://127.0.0.1:3456/deep";
        assert!(lifetime.allows_navigation(&origin, url));
        owner.revoke().unwrap();
        assert!(!lifetime.allows_navigation(&origin, url));
        assert!(matches!(
            crate::DesktopApp::from_local_server(LocalServerOptions::new(origin, lifetime)).build(),
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
    }

    #[test]
    fn owner_drop_retires_weak_frame_even_if_frame_is_retained() {
        let origin = LoopbackOrigin::from_socket_addr("127.0.0.1:3456".parse().unwrap()).unwrap();
        let (owner, lifetime) = HostLifetime::new();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let frame = crate::DesktopApp::from_local_server(LocalServerOptions::new(
            origin.clone(),
            lifetime.clone(),
        ))
        .build()
        .unwrap();
        drop(owner);
        assert!(!lifetime.is_active());
        assert!(!lifetime.allows_navigation(&origin, "http://127.0.0.1:3456/"));
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(run_local_server_frame(frame).is_err());
    }

    #[test]
    fn revoke_precedes_single_close_wake_even_when_command_queue_is_full() {
        use std::sync::atomic::AtomicUsize;
        let (owner, lifetime) = HostLifetime::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let state = lifetime.clone();
        let _registration = lifetime
            .register_close(Arc::new(move || {
                assert!(!state.is_active());
                observed.fetch_add(1, Ordering::Relaxed);
            }))
            .unwrap();
        let handle = WindowHandle::default();
        for _ in 0..crate::MAX_QUEUED_WINDOW_COMMANDS {
            handle.set_title("queue full").unwrap();
        }
        assert!(handle.set_title("overflow").is_err());
        owner.revoke().unwrap();
        owner.revoke().unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!lifetime.is_active());
    }

    #[test]
    fn registration_drop_releases_callback_without_extending_owner() {
        let (owner, lifetime) = HostLifetime::new();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = Arc::clone(&calls);
        let registration = lifetime
            .register_close(Arc::new(move || {
                captured.fetch_add(1, Ordering::Relaxed);
            }))
            .unwrap();
        drop(registration);
        drop(owner);
        assert!(!lifetime.is_active());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn retired_and_already_running_lifetimes_reject_close_registration() {
        let (owner, lifetime) = HostLifetime::new();
        let first = lifetime.register_close(Arc::new(|| {})).unwrap();
        assert!(matches!(
            lifetime.register_close(Arc::new(|| {})),
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
        drop(first);
        owner.revoke().unwrap();
        assert!(matches!(
            lifetime.register_close(Arc::new(|| {})),
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
    }

    #[test]
    fn attached_owner_revocation_does_not_terminate_its_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (owner, lifetime) = HostLifetime::new();
        owner.revoke().unwrap();
        assert!(!lifetime.is_active());
        assert!(std::net::TcpStream::connect_timeout(
            &address,
            std::time::Duration::from_millis(250)
        )
        .is_ok());
    }

    #[test]
    fn failed_native_wake_preserves_revocation_and_allows_one_explicit_retry() {
        use std::sync::atomic::AtomicUsize;
        let (owner, lifetime) = HostLifetime::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let received = Arc::clone(&attempts);
        let registration = lifetime
            .register_close_fallible(Arc::new(move || {
                if received.fetch_add(1, Ordering::Relaxed) == 0 {
                    Err(HostCloseError::WakeFailed {
                        message: "injected full OS message queue".to_string(),
                    })
                } else {
                    Ok(())
                }
            }))
            .unwrap();
        assert!(matches!(
            owner.retry_close(),
            Err(HostCloseError::StillActive)
        ));
        assert!(matches!(
            owner.revoke(),
            Err(HostCloseError::WakeFailed { .. })
        ));
        assert!(!lifetime.is_active());
        assert!(owner.retry_close().is_ok());
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        drop(registration);
        assert!(owner.retry_close().is_ok());
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn stale_window_generation_cannot_close_a_reused_hwnd() {
        let (owner, lifetime) = HostLifetime::new();
        assert!(!should_close_for_cookie(Some(&lifetime), Some(7), 7));
        owner.revoke().unwrap();
        assert!(should_close_for_cookie(Some(&lifetime), Some(7), 7));
        assert!(!should_close_for_cookie(Some(&lifetime), Some(8), 7));
        assert!(!should_close_for_cookie(Some(&lifetime), None, 7));
        assert!(!should_close_for_cookie(None, Some(7), 7));
        assert!(!should_close_for_cookie(Some(&lifetime), Some(0), 0));
    }
}
