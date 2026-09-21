// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use webui_desktop::{
    DesktopBundleConfig, DesktopBundleManifest, DesktopEvent, DesktopRuntime, DesktopShellConfig,
    EventRegistry, EventResponse, TitlebarStyle, WindowEffect, WindowHandle, WindowOptions,
};

/// Cross-platform desktop frame owned by a native shell backend.
///
/// App runners construct one frame and let [`PlatformFrameBackend`] select the
/// platform implementation. Future shell features should be added to this
/// surface first, then implemented by each backend behind the same contract.
#[derive(Clone)]
pub struct DesktopFrame {
    /// Runtime-neutral WebUI request dispatcher.
    pub runtime: Arc<DesktopRuntime>,
    /// Cross-platform window options.
    pub window: WindowOptions,
    /// Cross-platform native shell options from the desktop manifest.
    pub shell: DesktopShellConfig,
    /// UI-thread lifecycle event registry.
    pub events: EventRegistry,
    /// Sendable command queue for native window control.
    pub window_handle: WindowHandle,
}

impl DesktopFrame {
    /// Create a desktop frame with default shell options.
    #[must_use]
    pub fn new(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Self {
        Self {
            runtime,
            window,
            shell: DesktopShellConfig::default(),
            events: EventRegistry::default(),
            window_handle: WindowHandle::default(),
        }
    }

    /// Register a non-blocking lifecycle callback invoked on the backend UI thread.
    pub fn on_event<F>(&self, handler: F)
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.events.on_event(handler);
    }

    /// Attach shell options to the frame.
    #[must_use]
    pub fn with_shell(mut self, shell: DesktopShellConfig) -> Self {
        self.shell = shell;
        self
    }
}

/// Cross-platform shell capabilities supported by the active backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DesktopFrameCapabilities {
    /// Backend can install an application menu.
    pub app_menu: bool,
    /// Backend can expose Windows-style jump lists or an equivalent launcher menu.
    pub jump_list: bool,
    /// Backend can open native popover/popup windows.
    pub popovers: bool,
    /// Backend can broker app-controlled downloads.
    pub downloads: bool,
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
    /// # Errors
    ///
    /// Returns an error if the native shell cannot initialize or exits with a
    /// platform-specific failure.
    fn run_frame(&self, frame: DesktopFrame) -> Result<()>;
}

/// Backend that dispatches to the current target OS implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformFrameBackend;

impl PlatformFrameBackend {
    /// Create the current-platform backend dispatcher.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopFrameBackend for PlatformFrameBackend {
    fn capabilities(&self) -> DesktopFrameCapabilities {
        platform_capabilities()
    }

    fn run_frame(&self, frame: DesktopFrame) -> Result<()> {
        platform_run_frame(frame)
    }
}

/// Run a prebuilt desktop runtime in the current platform backend.
///
/// # Errors
///
/// Returns an error if the current platform shell cannot initialize.
pub fn run_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    run_frame(DesktopFrame::new(runtime, window))
}

/// Run a prebuilt desktop frame in the current platform backend.
///
/// # Errors
///
/// Returns an error if the current platform shell cannot initialize.
pub fn run_frame(frame: DesktopFrame) -> Result<()> {
    validate_frame_capabilities(
        &frame.window,
        &frame.shell,
        PlatformFrameBackend::new().capabilities(),
    )?;
    PlatformFrameBackend::new().run_frame(frame)
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
    let unsupported = if !matches!(window.titlebar, TitlebarStyle::Native)
        && !capabilities.titlebar_styles
    {
        Some("titlebar style; use the native titlebar or select a backend that advertises titlebar_styles")
    } else if !capabilities.supports_effect(window.effect) {
        Some("window effect; use WindowEffect::None or an effect this backend advertises in supported_effects")
    } else if shell.tray.is_some() && !capabilities.tray {
        Some("tray icon; remove shell.tray or select a backend that advertises tray")
    } else if !shell.menus.is_empty() && !capabilities.app_menu {
        Some("application menu; remove shell.menus or select a backend that advertises app_menu")
    } else if !shell.jump_list.is_empty() && !capabilities.jump_list {
        Some("jump list; remove shell.jump_list or select a backend that advertises jump_list")
    } else if shell.popovers.enabled && !capabilities.popovers {
        Some("popovers; disable shell.popovers or select a backend that advertises popovers")
    } else if shell.downloads.enabled && !capabilities.downloads {
        Some("downloads; disable shell.downloads or select a backend that advertises downloads")
    } else {
        None
    };
    unsupported.map_or(Ok(()), |message| {
        Err(anyhow::anyhow!("unsupported desktop runtime: {message}"))
    })
}

/// Run an app-specific packaged desktop executable.
///
/// # Errors
///
/// Returns an error if packaged resources cannot be found or the current
/// platform shell cannot initialize.
pub fn run_packaged_app() -> Result<()> {
    let resources = find_packaged_resources_dir().ok_or_else(|| {
        anyhow::anyhow!("failed to locate packaged desktop resources beside executable")
    })?;
    let manifest = DesktopBundleManifest::load(&resources.join("manifest.webui-desktop.json"))
        .with_context(|| "Failed to read packaged desktop manifest")?;
    let window = manifest.window.clone();
    let shell = manifest.shell.clone();
    let runtime = Arc::new(DesktopRuntime::from_bundle_config_and_manifest(
        DesktopBundleConfig::new(resources),
        manifest,
    )?);
    run_frame(DesktopFrame::new(runtime, window).with_shell(shell))
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

#[cfg(target_os = "macos")]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::macos::run_frame(frame)
}

#[cfg(target_os = "linux")]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::linux::run_frame(frame)
}

#[cfg(target_os = "windows")]
fn platform_run_frame(frame: DesktopFrame) -> Result<()> {
    crate::windows::run_frame(frame)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_run_frame(_frame: DesktopFrame) -> Result<()> {
    Err(anyhow::anyhow!(
        "desktop frame backend is not implemented on this platform yet"
    ))
}

/// Capabilities the macOS AppKit/WKWebView backend implements.
///
/// Jump lists, popovers, and downloads have no macOS backend implementation, so
/// they stay false and `validate_frame_capabilities` rejects them up front
/// rather than letting them silently no-op.
///
/// All four effects are implemented: `macos::effects::resolve_effect` maps the
/// three blur variants onto `NSVisualEffectView` vibrancy and `Tabbed` onto the
/// native tabbed titlebar treatment.
#[cfg(target_os = "macos")]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        app_menu: true,
        jump_list: false,
        popovers: false,
        downloads: false,
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
#[cfg(target_os = "windows")]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        app_menu: false,
        jump_list: false,
        popovers: false,
        downloads: false,
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
#[cfg(target_os = "linux")]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities {
        app_menu: false,
        jump_list: false,
        popovers: false,
        downloads: false,
        titlebar_styles: true,
        supported_effects: &[],
        tray: false,
        window_controls: true,
        events: true,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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
mod tests {
    use super::*;
    use tempfile::TempDir;
    use webui::{BuildOptions, CssStrategy, DomStrategy, LegalComments};
    use webui_desktop::DesktopSourceConfig;

    fn test_runtime() -> Arc<DesktopRuntime> {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<main>Hello</main>").unwrap();
        let runtime = DesktopRuntime::from_source(DesktopSourceConfig::new(BuildOptions {
            app_dir: dir.path().to_path_buf(),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            css_bundle: false,
            plugin: None,
            components: Vec::new(),
            component_asset_roots: Vec::new(),
            metafile: false,
            css_file_name_template: webui::DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            css_public_base: None,
            legal_comments: LegalComments::Inline,
            theme: None,
            projection_manifests: Vec::new(),
        }))
        .unwrap();
        Arc::new(runtime)
    }

    #[test]
    fn frame_new_uses_default_shell() {
        let window = WindowOptions {
            title: "Frame Test".to_string(),
            ..WindowOptions::default()
        };

        let frame = DesktopFrame::new(test_runtime(), window);

        assert_eq!(frame.window.title, "Frame Test");
        assert!(frame.shell.icon_path.is_none());
        assert!(frame.shell.menus.is_empty());
        assert!(frame.shell.jump_list.is_empty());
    }

    #[test]
    fn frame_with_shell_replaces_shell() {
        let mut shell = DesktopShellConfig {
            icon_path: Some(PathBuf::from("assets/icon.png")),
            ..DesktopShellConfig::default()
        };
        shell.downloads.enabled = true;

        let frame = DesktopFrame::new(test_runtime(), WindowOptions::default()).with_shell(shell);

        assert_eq!(
            frame.shell.icon_path.as_deref(),
            Some(std::path::Path::new("assets/icon.png"))
        );
        assert!(frame.shell.downloads.enabled);
    }

    #[test]
    fn platform_backend_advertises_only_implemented_capabilities() {
        let capabilities = PlatformFrameBackend::new().capabilities();

        // Every supported backend implements lifecycle events, non-native
        // titlebars, and host-driven window controls; a backend that cannot
        // must report false so `validate_frame_capabilities` rejects the
        // request instead of letting the feature silently no-op.
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            assert!(capabilities.events);
            assert!(capabilities.titlebar_styles);
            assert!(capabilities.window_controls);
            // No backend implements these yet.
            assert!(!capabilities.jump_list);
            assert!(!capabilities.popovers);
            assert!(!capabilities.downloads);
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
}
