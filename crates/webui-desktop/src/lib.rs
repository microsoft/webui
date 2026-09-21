// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native desktop SDK for WebUI applications.
//!
//! Bundle loading, rendering, IPC, and frame configuration are available without
//! native dependencies. Enable `native` for the platform webview, `source` for
//! development compilation, and `cli` for the `webui-desktop` tooling binary.

mod app;
mod bundle;
mod error;
mod event;
mod frame;
mod hydration;
mod ipc;
mod navigation;
#[cfg(feature = "source")]
mod package;
mod path;
mod protocol;
mod runtime;
mod window;
mod window_state;

#[cfg(all(feature = "native", target_os = "linux"))]
#[allow(unsafe_code)]
pub mod linux;
#[cfg(all(feature = "native", target_os = "macos"))]
#[allow(unsafe_code)]
pub mod macos;
#[cfg(all(feature = "native", target_os = "windows"))]
#[allow(unsafe_code)]
pub mod windows;

#[cfg(feature = "native")]
pub use app::run_packaged_app;
pub use app::{DesktopApp, DesktopAppBuilder};
#[cfg(feature = "source")]
pub use bundle::{build_desktop_bundle, DesktopBundleOptions};
pub use bundle::{
    BundleAsset, BundleIntegrity, DesktopBundleManifest, DesktopDownloadPolicy,
    DesktopJumpListItem, DesktopMenu, DesktopMenuItem, DesktopPackageTarget, DesktopPopoverPolicy,
    DesktopShellConfig, TrayConfig,
};
pub use error::{DesktopError, Result};
pub use event::{
    DesktopEvent, DesktopHostMessage, DesktopHostMessageError, EventHandler, EventJavascriptError,
    EventRegistrationError, EventRegistry, EventResponse, EventSubscription, WindowCommand,
    WindowCommandError, WindowHandle, WindowId, DRAG_REGION_SCRIPT, MAX_EVENT_HANDLERS,
    MAX_HOST_MESSAGE_BYTES, MAX_QUEUED_WINDOW_COMMANDS, MAX_QUEUED_WINDOW_TITLE_BYTES,
    MAX_WINDOW_TITLE_BYTES,
};
pub use frame::{
    find_packaged_resources_dir, run_frame_with, validate_frame_capabilities, DesktopFrame,
    DesktopFrameBackend, DesktopFrameCapabilities,
};
#[cfg(feature = "native")]
pub use frame::{run_frame, run_runtime, PlatformFrameBackend};
pub use ipc::{
    desktop_ipc_response, DesktopIpcError, DesktopIpcRequest, DesktopIpcResponse, IpcHandlerError,
    IpcRegistry, DEFAULT_MAX_IPC_PAYLOAD_BYTES, IPC_VERSION,
};
pub use navigation::is_allowed_navigation_url;
#[cfg(feature = "source")]
pub use package::{package_desktop_bundle, DesktopPackageOptions, DesktopPackageResult};
pub use protocol::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DEFAULT_MAX_ASSET_BYTES,
    IPC_ENDPOINT,
};
#[cfg(feature = "source")]
pub use runtime::DesktopSourceConfig;
pub use runtime::{ApiContext, ApiRouteRegistry, DesktopBundleConfig, DesktopRuntime};
pub use runtime::{RouteContext, RouteStateRegistry};
pub use window::{
    apply_window_css, window_css_block, DesktopPlatform, Rgba, RgbaParseError, TitlebarStyle,
    WindowEffect, WindowInsets, WindowOptions,
};
pub use window_state::{DisplayBounds, WindowState, WindowStateError, WindowStateStore};

#[cfg(feature = "source")]
pub use webui::{
    BuildOptions, CssStrategy, DomStrategy, LegalComments, Plugin, DEFAULT_CSS_FILE_NAME_TEMPLATE,
};
