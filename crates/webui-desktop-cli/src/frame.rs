// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use webui_desktop::{
    DesktopBundleConfig, DesktopBundleManifest, DesktopRuntime, DesktopShellConfig, WindowOptions,
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
}

impl DesktopFrame {
    /// Create a desktop frame with default shell options.
    #[must_use]
    pub fn new(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Self {
        Self {
            runtime,
            window,
            shell: DesktopShellConfig::default(),
        }
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
    PlatformFrameBackend::new().run_frame(frame)
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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn platform_capabilities() -> DesktopFrameCapabilities {
    DesktopFrameCapabilities::default()
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
            plugin: None,
            components: Vec::new(),
            component_asset_roots: Vec::new(),
            css_file_name_template: webui::DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            css_public_base: None,
            legal_comments: LegalComments::Inline,
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
    fn platform_backend_default_capabilities_are_explicit() {
        let backend = PlatformFrameBackend::new();

        assert_eq!(backend.capabilities(), DesktopFrameCapabilities::default());
    }
}
