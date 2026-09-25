// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native desktop SDK for WebUI applications.
//!
//! Bundle loading, rendering, and frame configuration are available without
//! native dependencies. Enable `application-ipc` for application IPC,
//! `native` for the platform webview, `source` for
//! development compilation, and `cli` for the `webui-desktop` tooling binary.

#[cfg(test)]
extern crate self as webui_desktop;

#[cfg(all(test, feature = "application-ipc"))]
#[path = "../tests/support/echo.rs"]
mod ipc_test_support;

#[cfg(all(test, feature = "application-ipc"))]
#[path = "../tests/ipc_contract.rs"]
mod ipc_contract_tests;

mod app;
mod asset_file;
#[cfg(any(all(windows, feature = "native"), test))]
mod browser_profile;
mod bundle;
#[cfg(all(feature = "native", feature = "application-ipc"))]
mod document;
mod error;
mod event;
#[cfg(any(feature = "native", test))]
mod execution;
mod frame;
mod hydration;
#[cfg(feature = "application-ipc")]
pub mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_assets;
#[cfg(all(feature = "native", feature = "application-ipc"))]
mod native_ipc;
#[cfg(all(
    feature = "native",
    any(
        feature = "application-ipc",
        target_os = "macos",
        target_os = "windows"
    )
))]
mod native_tasks;
mod navigation;
#[cfg(feature = "source")]
mod package;
mod path;
mod protocol;
mod response_content;
mod routes;
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
    BundleAsset, BundleIntegrity, DesktopBundleManifest, DesktopMenu, DesktopMenuItem,
    DesktopPackageTarget, DesktopShellConfig, TrayConfig,
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
#[cfg(feature = "application-ipc")]
pub use ipc::{IpcRegistry, DEFAULT_MAX_IPC_PAYLOAD_BYTES, IPC_VERSION};
pub use navigation::is_allowed_navigation_url;
#[cfg(feature = "source")]
pub use package::{package_desktop_bundle, DesktopPackageOptions, DesktopPackageResult};
#[cfg(feature = "application-ipc")]
pub use protocol::IPC_ENDPOINT;
pub use protocol::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DesktopResponseBody,
    DesktopResponseLease, DEFAULT_MAX_ASSET_BYTES, DEFAULT_MAX_REQUEST_BYTES,
};
pub use response_content::{DesktopResponseContent, DesktopResponseFile};
pub use routes::{ApiContext, ApiRouteRegistry, RouteContext, RouteStateRegistry};
#[cfg(feature = "source")]
pub use runtime::DesktopSourceConfig;
pub use runtime::{DesktopBundleConfig, DesktopRuntime};
pub use window::{
    apply_window_css, window_css_block, CaptionButtonSize, DesktopPlatform, Rgba, RgbaParseError,
    TitlebarStyle, WindowEffect, WindowInsets, WindowOptions,
};
pub use window_state::{DisplayBounds, WindowState, WindowStateError, WindowStateStore};

#[cfg(feature = "source")]
pub use webui::{
    BuildOptions, CssStrategy, DomStrategy, LegalComments, Plugin, DEFAULT_CSS_FILE_NAME_TEMPLATE,
};
