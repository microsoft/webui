// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "application-ipc")]
use crate::ipc::{IpcBridge, IpcHost, IpcOptions, IpcWindow, IpcWindowOwner};
use crate::{
    DesktopError, DesktopEvent, DesktopRuntime, DesktopShellConfig, EventRegistrationError,
    EventRegistry, EventResponse, EventSubscription, Result, TitlebarStyle, WindowEffect,
    WindowHandle, WindowOptions,
};

/// Cross-platform desktop frame owned by a native shell backend.
///
/// A frame owns its event registrations and command channel until the backend
/// returns. Use the optional native runner or a custom [`DesktopFrameBackend`];
/// clone individual handles, not the owning frame.
pub struct DesktopFrame {
    #[cfg(feature = "native")]
    pub(crate) executor: Arc<crate::execution::ApplicationExecutor>,
    /// Stable application identity from the bundle or Rust host.
    pub(crate) app_id: Option<String>,
    /// Runtime-neutral WebUI request dispatcher.
    pub(crate) runtime: Arc<DesktopRuntime>,
    /// Cross-platform window options.
    pub(crate) window: WindowOptions,
    /// Cross-platform native shell options from the desktop manifest.
    pub(crate) shell: DesktopShellConfig,
    /// UI-thread lifecycle event registry.
    pub(crate) events: EventRegistry,
    /// Sendable command queue for native window control.
    pub(crate) window_handle: WindowHandle,
    #[cfg(feature = "application-ipc")]
    ipc_owner: IpcWindowOwner,
}

impl DesktopFrame {
    /// Borrow the immutable application identity.
    #[must_use]
    pub fn app_id(&self) -> Option<&str> {
        self.app_id.as_deref()
    }

    /// Borrow the request dispatcher owned by this frame.
    #[must_use]
    pub fn runtime(&self) -> &Arc<DesktopRuntime> {
        &self.runtime
    }

    /// Borrow the immutable window configuration.
    #[must_use]
    pub fn window(&self) -> &WindowOptions {
        &self.window
    }

    /// Borrow the immutable native shell configuration.
    #[must_use]
    pub fn shell(&self) -> &DesktopShellConfig {
        &self.shell
    }

    /// Borrow the frame's event registry for a custom backend.
    #[must_use]
    pub fn events(&self) -> &EventRegistry {
        &self.events
    }

    /// Borrow the sendable window command handle.
    #[must_use]
    pub fn window_handle(&self) -> &WindowHandle {
        &self.window_handle
    }

    /// Create a desktop frame with default shell options and deny-all IPC permissions.
    ///
    /// # Errors
    ///
    /// Returns an error if the IPC schema is invalid or window styling differs
    /// from the already-rendered runtime.
    pub fn new(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<Self> {
        #[cfg(feature = "application-ipc")]
        {
            Self::with_ipc_options(runtime, window, IpcOptions::default())
        }
        #[cfg(not(feature = "application-ipc"))]
        {
            runtime.validate_window(&window)?;
            Ok(Self::from_parts(runtime, window))
        }
    }

    /// Create a frame with explicit application IPC permissions and resource limits.
    ///
    /// # Errors
    ///
    /// Returns an error if the IPC schema or limits are invalid, or window
    /// styling differs from the already-rendered runtime.
    #[cfg(feature = "application-ipc")]
    pub fn with_ipc_options(
        runtime: Arc<DesktopRuntime>,
        window: WindowOptions,
        options: IpcOptions,
    ) -> Result<Self> {
        runtime.validate_window(&window)?;
        let origin = if cfg!(target_os = "windows") {
            "https://app.webui.localhost"
        } else {
            "webui://app"
        }
        .to_string();
        let host = if runtime.is_development() {
            IpcHost::Source { origin }
        } else {
            IpcHost::Packaged { origin }
        };
        let ipc_owner = IpcWindowOwner::new(runtime.ipc_registry(), options, host)?;
        Ok(Self::from_parts(runtime, window, ipc_owner))
    }

    fn from_parts(
        runtime: Arc<DesktopRuntime>,
        window: WindowOptions,
        #[cfg(feature = "application-ipc")] ipc_owner: IpcWindowOwner,
    ) -> Self {
        Self {
            #[cfg(feature = "native")]
            executor: Arc::default(),
            app_id: None,
            runtime,
            window,
            shell: DesktopShellConfig::default(),
            events: EventRegistry::default(),
            window_handle: WindowHandle::default(),
            #[cfg(feature = "application-ipc")]
            ipc_owner,
        }
    }

    /// Return a weak handle to this window's document-scoped application IPC.
    #[must_use]
    #[cfg(feature = "application-ipc")]
    pub fn ipc(&self) -> IpcWindow {
        self.ipc_owner.window()
    }

    /// Return the native transport facade for a custom backend.
    ///
    /// Adapters must supply trusted main-frame navigation and origin information
    /// and retain the owning frame until callbacks and the native loop finish.
    #[must_use]
    #[cfg(feature = "application-ipc")]
    pub fn ipc_bridge(&self) -> IpcBridge {
        self.ipc_owner.bridge()
    }

    /// Register a non-blocking callback for the frame's lifetime.
    ///
    /// # Errors
    ///
    /// Returns an error when registration is closed or its bounded capacity is full.
    pub fn on_event<F>(&self, handler: F) -> std::result::Result<(), EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.events.on_event(handler)
    }

    /// Register a callback until the returned subscription is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when registration is closed or its bounded capacity is full.
    pub fn subscribe<F>(
        &self,
        handler: F,
    ) -> std::result::Result<EventSubscription, EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.events.subscribe(handler)
    }

    /// Set the application's stable identity for native services.
    #[must_use]
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Attach shell options to the frame.
    #[must_use]
    pub fn with_shell(mut self, shell: DesktopShellConfig) -> Self {
        self.shell = shell;
        self
    }
}

impl Drop for DesktopFrame {
    fn drop(&mut self) {
        #[cfg(feature = "native")]
        self.executor.close();
        // Callback captures may own window handles. Stop commands before
        // releasing those captures so their destructors cannot enqueue work.
        #[cfg(feature = "application-ipc")]
        self.ipc_owner.close();
        self.window_handle.close();
        self.events.close();
    }
}

/// Cross-platform shell capabilities supported by the active backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DesktopFrameCapabilities {
    /// Backend supports authenticated, document-scoped application IPC.
    pub application_ipc: bool,
    /// Backend can install an application menu.
    pub app_menu: bool,
    /// Backend supports non-native titlebar styles.
    pub titlebar_styles: bool,
    /// Platform window effects this backend actually applies.
    ///
    /// A single boolean would claim support for every [`WindowEffect`] variant,
    /// which is never true: no backend implements all of them. Listing the
    /// implemented variants lets `validate_frame_capabilities` reject an
    /// unsupported effect loudly instead of letting it silently no-op.
    pub supported_effects: &'static [WindowEffect],
    /// Backend supports tray icons.
    pub tray: bool,
    /// Backend supports queued native window controls.
    pub window_controls: bool,
    /// Backend emits lifecycle events.
    pub events: bool,
}

impl DesktopFrameCapabilities {
    /// Return whether the backend applies `effect` natively.
    ///
    /// [`WindowEffect::None`] is always supported because it requests nothing.
    #[must_use]
    pub fn supports_effect(&self, effect: WindowEffect) -> bool {
        matches!(effect, WindowEffect::None) || self.supported_effects.contains(&effect)
    }
}

/// Native desktop frame backend contract.
///
/// Platform modules implement this trait so app code can use one API regardless
/// of target OS. Backend-specific code should stay inside the platform module.
pub trait DesktopFrameBackend {
    /// Return the shell capabilities supported by this backend.
    #[must_use]
    fn capabilities(&self) -> DesktopFrameCapabilities {
        DesktopFrameCapabilities::default()
    }

    /// Run the desktop frame until the native app exits.
    ///
    /// Call this through [`run_frame_with`] so capability validation happens
    /// before ownership is transferred to the backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the native shell cannot initialize or exits with a
    /// platform-specific failure.
    fn run_frame(&self, frame: DesktopFrame) -> Result<()>;
}

/// Backend that dispatches to the current target OS implementation.
#[cfg(feature = "native")]
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformFrameBackend;

#[cfg(feature = "native")]
impl PlatformFrameBackend {
    /// Create the current-platform backend dispatcher.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(feature = "native")]
impl DesktopFrameBackend for PlatformFrameBackend {
    fn capabilities(&self) -> DesktopFrameCapabilities {
        platform_capabilities()
    }

    fn run_frame(&self, frame: DesktopFrame) -> Result<()> {
        validate_frame(&frame, self.capabilities())?;
        platform_run_frame(frame)
    }
}

/// Run a prebuilt desktop runtime in the current platform backend.
///
/// # Errors
///
/// Returns an error if the current platform shell cannot initialize.
#[cfg(feature = "native")]
pub fn run_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    run_frame(DesktopFrame::new(runtime, window)?)
}

/// Run a prebuilt desktop frame in the current platform backend.
///
/// # Errors
///
/// Returns an error if the current platform shell cannot initialize.
#[cfg(feature = "native")]
pub fn run_frame(frame: DesktopFrame) -> Result<()> {
    run_frame_with(frame, &PlatformFrameBackend::new())
}

/// Validate and run a frame with a caller-supplied backend.
///
/// This entry point is available without the `native` feature, allowing custom
/// hosts and headless tests to use the same capability and ownership contract.
/// The backend must retain the frame until its event loop and callbacks finish.
///
/// # Errors
///
/// Returns an error before invoking the backend for unsupported configuration,
/// or propagates the backend's startup or runtime failure.
pub fn run_frame_with<B: DesktopFrameBackend + ?Sized>(
    frame: DesktopFrame,
    backend: &B,
) -> Result<()> {
    validate_frame(&frame, backend.capabilities())?;
    backend.run_frame(frame)
}

fn validate_frame(frame: &DesktopFrame, capabilities: DesktopFrameCapabilities) -> Result<()> {
    frame.runtime.validate_window(&frame.window)?;
    validate_frame_capabilities(&frame.window, &frame.shell, capabilities)?;
    #[cfg(feature = "application-ipc")]
    if frame.runtime.ipc_registry().is_enabled() && !capabilities.application_ipc {
        return Err(DesktopError::UnsupportedRuntime {
            message: "the selected backend does not support application IPC".to_string(),
            help: "use a native WebUI backend or implement the frame-scoped IPC adapter contract"
                .to_string(),
        });
    }
    Ok(())
}

/// Validate requested window and shell features before the native shell starts.
///
/// # Errors
///
/// Returns actionable diagnostics when a requested feature is not supported.
pub fn validate_frame_capabilities(
    window: &WindowOptions,
    shell: &DesktopShellConfig,
    capabilities: DesktopFrameCapabilities,
) -> Result<()> {
    let unsupported =
        if !matches!(window.titlebar, TitlebarStyle::Native) && !capabilities.titlebar_styles {
            Some((
                "titlebar style",
                "Use the native titlebar or a backend that advertises titlebar_styles",
            ))
        } else if !capabilities.supports_effect(window.effect) {
            Some((
                "window effect",
                "Use WindowEffect::None or an effect this backend advertises in supported_effects",
            ))
        } else if shell.tray.is_some() && !capabilities.tray {
            Some((
                "tray icon",
                "Remove shell.tray or select a backend that advertises tray",
            ))
        } else if !shell.menus.is_empty() && !capabilities.app_menu {
            Some((
                "application menu",
                "Remove shell.menus or select a backend that advertises app_menu",
            ))
        } else {
            None
        };
    unsupported.map_or(Ok(()), |(message, help)| {
        Err(DesktopError::UnsupportedRuntime {
            message: message.to_string(),
            help: help.to_string(),
        })
    })
}

/// Return the packaged bundle resource directory when the executable is running
/// from a desktop package layout.
#[must_use]
pub fn find_packaged_resources_dir() -> Option<PathBuf> {
    let resources = platform_packaged_resources_dir()?;
    resources
        .join("manifest.webui-desktop.json")
        .is_file()
        .then_some(resources)
}

#[cfg(all(feature = "native", target_os = "macos"))]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::macos::run_frame(frame).map_err(|source| DesktopError::Backend {
        source: source.into(),
    })
}

#[cfg(all(feature = "native", target_os = "linux"))]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::linux::run_frame(frame).map_err(|source| DesktopError::Backend {
        source: source.into(),
    })
}

#[cfg(all(feature = "native", target_os = "windows"))]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::windows::run_frame(frame).map_err(|source| DesktopError::Backend {
        source: source.into(),
    })
}

#[cfg(all(
    feature = "native",
    not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
))]
fn platform_run_frame(_frame: DesktopFrame) -> Result<()> {
    Err(DesktopError::UnsupportedRuntime {
        message: "no native backend for this platform".to_string(),
        help: "Use a supported desktop target or supply a DesktopFrameBackend".to_string(),
    })
}

/// Capabilities the macOS AppKit/WKWebView backend implements.
///
/// All four effects are implemented: `macos::effects::resolve_effect` maps the
/// three blur variants onto `NSVisualEffectView` vibrancy and `Tabbed` onto the
/// native tabbed titlebar treatment.
#[cfg(all(feature = "native", target_os = "macos"))]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        application_ipc: cfg!(feature = "application-ipc"),
        app_menu: true,
        titlebar_styles: true,
        supported_effects: &[
            WindowEffect::Vibrancy,
            WindowEffect::Acrylic,
            WindowEffect::Mica,
            WindowEffect::Tabbed,
        ],
        tray: true,
        window_controls: true,
        events: true,
    }
}

/// Capabilities the Windows Win32/WebView2 backend implements.
///
/// Tray support is deferred. `Acrylic` and `Mica` map onto the DWM system
/// backdrop types, and `Vibrancy` resolves to acrylic as its closest Windows
/// equivalent. `Tabbed` is a macOS titlebar treatment with no Windows analogue,
/// so it is rejected rather than silently ignored.
#[cfg(all(feature = "native", target_os = "windows"))]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        application_ipc: cfg!(feature = "application-ipc"),
        app_menu: false,
        titlebar_styles: true,
        supported_effects: &[
            WindowEffect::Vibrancy,
            WindowEffect::Acrylic,
            WindowEffect::Mica,
        ],
        tray: false,
        window_controls: true,
        events: true,
    }
}

/// Capabilities the Linux GTK4/WebKitGTK backend implements.
///
/// `supported_effects` is empty because GTK4 exposes no portable blur or
/// vibrancy, and tray is false because GTK4 removed `GtkStatusIcon`.
#[cfg(all(feature = "native", target_os = "linux"))]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        application_ipc: cfg!(feature = "application-ipc"),
        app_menu: false,
        titlebar_styles: true,
        supported_effects: &[],
        tray: false,
        window_controls: true,
        events: true,
    }
}

#[cfg(all(
    feature = "native",
    not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
))]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities::default()
}

#[cfg(target_os = "macos")]
fn platform_packaged_resources_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let contents = exe.parent().and_then(std::path::Path::parent)?;
    Some(contents.join("Resources").join("webui"))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn platform_packaged_resources_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("resources").join("webui"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_packaged_resources_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

    use crate::{
        BundleIntegrity, DesktopBundleConfig, DesktopBundleManifest, DesktopRuntime,
        DesktopShellConfig, WindowOptions,
    };

    pub(crate) fn bundle() -> (TempDir, DesktopBundleManifest) {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        // Frame tests must exercise the same bundle path without the source feature.
        WebUIProtocol::new(HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<main>Hello</main>")],
                ..Default::default()
            },
        )]))
        .to_protobuf_file(dir.path().join("protocol.bin"))
        .unwrap();
        let manifest = DesktopBundleManifest {
            manifest_version: DesktopBundleManifest::VERSION,
            app_id: "com.example.webui.frame-test".to_string(),
            app_name: "Frame Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            entry: "index.html".to_string(),
            plugin: None,
            protocol_path: PathBuf::from("protocol.bin"),
            state_path: None,
            assets_dir: PathBuf::from("assets"),
            ipc_schema: None,
            window: WindowOptions::default(),
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
            integrity: BundleIntegrity::default(),
        };
        std::fs::write(
            dir.path().join("manifest.webui-desktop.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        (dir, manifest)
    }

    pub(crate) fn runtime() -> (TempDir, Arc<DesktopRuntime>) {
        let (dir, manifest) = bundle();
        let runtime = DesktopRuntime::from_bundle_config_and_manifest(
            DesktopBundleConfig::new(dir.path().to_path_buf()),
            manifest,
        )
        .unwrap();
        (dir, Arc::new(runtime))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use crate::{WindowCommandError, MAX_EVENT_HANDLERS};

    #[test]
    fn frame_new_uses_default_shell() {
        let window = WindowOptions {
            title: "Frame Test".to_string(),
            ..WindowOptions::default()
        };

        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, window).unwrap();

        assert_eq!(frame.window().title, "Frame Test");
        assert!(frame.shell().icon_path.is_none());
        assert!(frame.shell().menus.is_empty());
        assert!(frame.shell().tray.is_none());
        assert!(frame.app_id().is_none());
        assert!(!frame.runtime().startup_html().is_empty());
        let _ = frame.events();
        let _ = frame.window_handle();
    }

    #[test]
    fn frame_rejects_window_css_that_disagrees_with_rendered_document() {
        let (_bundle, runtime) = test_support::runtime();
        let window = WindowOptions {
            titlebar: TitlebarStyle::HiddenInset,
            ..WindowOptions::default()
        };
        assert!(DesktopFrame::new(runtime, window).is_err());
    }

    #[cfg(feature = "native")]
    #[test]
    fn direct_platform_trait_entry_cannot_bypass_capability_validation() {
        let (_bundle, runtime) = test_support::runtime();
        let mut frame = DesktopFrame::new(runtime, WindowOptions::default()).unwrap();
        // Internal mutation simulates an invalid frame; external hosts only have
        // immutable accessors. Validation must happen before opening any window.
        frame.window.titlebar = TitlebarStyle::HiddenInset;
        assert!(PlatformFrameBackend::new().run_frame(frame).is_err());
    }

    #[test]
    fn frame_with_shell_replaces_shell() {
        let shell = DesktopShellConfig {
            icon_path: Some(PathBuf::from("assets/icon.png")),
            tray: Some(crate::TrayConfig {
                icon_path: PathBuf::from("assets/tray.png"),
                tooltip: None,
            }),
            ..DesktopShellConfig::default()
        };

        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default())
            .unwrap()
            .with_shell(shell);

        assert_eq!(
            frame.shell.icon_path.as_deref(),
            Some(std::path::Path::new("assets/icon.png"))
        );
        assert!(frame.shell.tray.is_some());
    }

    #[cfg(feature = "native")]
    #[test]
    fn platform_backend_advertises_only_implemented_capabilities() {
        let capabilities = PlatformFrameBackend::new().capabilities();

        // Every supported backend implements lifecycle events, non-native
        // titlebars, and host-driven window controls; a backend that cannot
        // must report false so `validate_frame_capabilities` rejects the
        // request instead of letting the feature silently no-op.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            assert_eq!(
                capabilities.application_ipc,
                cfg!(feature = "application-ipc")
            );
            assert!(capabilities.events);
            assert!(capabilities.titlebar_styles);
            assert!(capabilities.window_controls);
        }
        // `None` requests nothing, so every backend must accept it.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        assert!(capabilities.supports_effect(WindowEffect::None));
        // GTK4 exposes no portable blur or vibrancy and removed GtkStatusIcon.
        #[cfg(target_os = "linux")]
        {
            assert!(capabilities.supported_effects.is_empty());
            assert!(!capabilities.supports_effect(WindowEffect::Mica));
            assert!(!capabilities.tray);
        }
        // Only macOS implements a native application menu and a tray item.
        #[cfg(target_os = "macos")]
        {
            assert!(capabilities.app_menu);
            assert!(capabilities.tray);
            // `resolve_effect` covers every non-`None` variant.
            assert!(capabilities.supports_effect(WindowEffect::Vibrancy));
            assert!(capabilities.supports_effect(WindowEffect::Acrylic));
            assert!(capabilities.supports_effect(WindowEffect::Mica));
            assert!(capabilities.supports_effect(WindowEffect::Tabbed));
        }
        #[cfg(target_os = "windows")]
        {
            assert!(!capabilities.app_menu);
            assert!(!capabilities.tray);
            // DWM backdrops cover the blur variants; `Tabbed` has no analogue.
            assert!(capabilities.supports_effect(WindowEffect::Vibrancy));
            assert!(capabilities.supports_effect(WindowEffect::Acrylic));
            assert!(capabilities.supports_effect(WindowEffect::Mica));
            assert!(!capabilities.supports_effect(WindowEffect::Tabbed));
        }
        // An unsupported platform must advertise nothing.
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        assert_eq!(capabilities, DesktopFrameCapabilities::default());
    }

    struct RecordingBackend {
        calls: AtomicUsize,
        fail: bool,
    }

    impl DesktopFrameBackend for RecordingBackend {
        fn run_frame(&self, frame: DesktopFrame) -> Result<()> {
            assert_eq!(frame.app_id.as_deref(), Some("com.example.custom"));
            assert!(frame.runtime.startup_html().contains("<main>Hello</main>"));
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(DesktopError::Backend {
                    source: std::io::Error::other("fixture backend failure").into(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn custom_backend_runs_without_native_features_and_closes_the_frame() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default())
            .unwrap()
            .with_app_id("com.example.custom");
        let handle = frame.window_handle.clone();
        let backend = RecordingBackend {
            calls: AtomicUsize::new(0),
            fail: false,
        };

        run_frame_with(frame, &backend).unwrap();

        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(handle.request_close(), Err(WindowCommandError::Closed));
    }

    #[test]
    fn capability_rejection_skips_backend_and_closes_the_frame() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(
            runtime,
            WindowOptions {
                effect: WindowEffect::Mica,
                ..WindowOptions::default()
            },
        )
        .unwrap();
        let handle = frame.window_handle.clone();
        let backend = RecordingBackend {
            calls: AtomicUsize::new(0),
            fail: false,
        };

        let error = run_frame_with(frame, &backend).unwrap_err();

        assert!(matches!(error, DesktopError::UnsupportedRuntime { .. }));
        assert!(error.hint().is_some());
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
        assert_eq!(handle.focus(), Err(WindowCommandError::Closed));
    }

    #[test]
    fn backend_failure_is_propagated_and_closes_the_frame() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default())
            .unwrap()
            .with_app_id("com.example.custom");
        let handle = frame.window_handle.clone();
        let backend = RecordingBackend {
            calls: AtomicUsize::new(0),
            fail: true,
        };

        assert!(matches!(
            run_frame_with(frame, &backend),
            Err(DesktopError::Backend { .. })
        ));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(handle.request_close(), Err(WindowCommandError::Closed));
    }

    #[test]
    fn frame_supports_persistent_and_scoped_event_handlers() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default()).unwrap();
        let persistent = Arc::new(AtomicUsize::new(0));
        let scoped = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&persistent);
        frame
            .on_event(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                EventResponse::Continue
            })
            .unwrap();
        let seen = Arc::clone(&scoped);
        let subscription = frame
            .subscribe(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                EventResponse::Continue
            })
            .unwrap();

        let _ = frame.events.dispatch(&DesktopEvent::Ready);
        drop(subscription);
        let _ = frame.events.dispatch(&DesktopEvent::Ready);

        assert_eq!(persistent.load(Ordering::SeqCst), 2);
        assert_eq!(scoped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn frame_propagates_registration_capacity_errors() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default()).unwrap();
        for _ in 0..MAX_EVENT_HANDLERS {
            frame.on_event(|_| EventResponse::Continue).unwrap();
        }

        assert_eq!(
            frame.on_event(|_| EventResponse::Continue),
            Err(EventRegistrationError::Capacity)
        );
        assert!(matches!(
            frame.subscribe(|_| EventResponse::Continue),
            Err(EventRegistrationError::Capacity)
        ));
    }

    #[test]
    fn dropping_frame_releases_cyclic_callbacks_and_closes_retained_handles() {
        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default()).unwrap();
        let registry = frame.events.clone();
        let handle = frame.window_handle.clone();
        let capture = Arc::new(());
        let observed = Arc::downgrade(&capture);
        let captured_registry = registry.clone();
        frame
            .on_event(move |_| {
                let _ = (&captured_registry, &capture);
                EventResponse::Continue
            })
            .unwrap();
        handle.focus().unwrap();

        drop(frame);

        assert!(observed.upgrade().is_none());
        assert!(handle.drain_commands().is_empty());
        assert_eq!(handle.request_close(), Err(WindowCommandError::Closed));
        assert_eq!(
            registry.on_event(|_| EventResponse::Continue),
            Err(EventRegistrationError::Closed)
        );
        assert!(matches!(
            registry.subscribe(|_| EventResponse::Continue),
            Err(EventRegistrationError::Closed)
        ));
    }

    #[test]
    fn frame_closes_commands_before_releasing_callback_captures() {
        struct SendOnDrop {
            handle: WindowHandle,
            result: Arc<Mutex<Option<std::result::Result<(), WindowCommandError>>>>,
        }
        impl Drop for SendOnDrop {
            fn drop(&mut self) {
                *self.result.lock().unwrap() = Some(self.handle.request_close());
            }
        }

        let (_bundle, runtime) = test_support::runtime();
        let frame = DesktopFrame::new(runtime, WindowOptions::default()).unwrap();
        let result = Arc::new(Mutex::new(None));
        let capture = SendOnDrop {
            handle: frame.window_handle.clone(),
            result: Arc::clone(&result),
        };
        frame
            .on_event(move |_| {
                let _ = &capture;
                EventResponse::Continue
            })
            .unwrap();

        drop(frame);

        assert_eq!(
            *result.lock().unwrap(),
            Some(Err(WindowCommandError::Closed))
        );
    }
}
