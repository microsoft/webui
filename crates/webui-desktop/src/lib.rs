// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native desktop SDK for WebUI applications.
//!
//! Bundle loading, rendering, and frame configuration are available without
//! native dependencies. Enable `application-ipc` for application IPC,
//! `native` for the platform webview, `source` for
//! development compilation, and `cli` for the `webui-desktop` tooling binary.
//! Enable `packaging` to lay out an already-compiled host without source compilation.
//! Enable `verified-update` for host-only, staged-file byte integrity checks.

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
#[cfg(feature = "native-services")]
mod capture;
#[cfg(all(feature = "native", feature = "application-ipc"))]
mod document;
mod error;
mod event;
#[cfg(any(feature = "native", test))]
mod execution;
mod frame;
#[cfg(feature = "local-server")]
mod frame_policy;
mod hydration;
#[cfg(any(test, all(feature = "native", target_os = "macos")))]
mod icon_path;
#[cfg(feature = "application-ipc")]
pub mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_assets;
#[cfg(feature = "local-server")]
mod local_server;
#[cfg(all(feature = "native", feature = "application-ipc"))]
mod native_ipc;
#[cfg(feature = "native-services")]
mod native_services;
#[cfg(all(
    feature = "native",
    any(
        feature = "application-ipc",
        target_os = "macos",
        target_os = "windows"
    )
))]
mod native_tasks;
#[cfg(feature = "native-services")]
mod native_theme;
mod navigation;
#[cfg(any(feature = "source", feature = "packaging"))]
mod package;
#[cfg(feature = "packaging")]
mod package_precompiled;
mod path;
mod protocol;
mod response_content;
mod routes;
mod runtime;
#[cfg(feature = "verified-update")]
pub mod verified_update;
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
#[cfg(feature = "native-services")]
pub use capture::{
    CaptureError, CaptureOptions, CaptureRequest, CapturedContent, CapturedContentChunk,
    MAX_WEB_CAPTURE_CHUNK_BYTES, MAX_WEB_CAPTURE_HEIGHT, MAX_WEB_CAPTURE_PNG_BYTES,
    MAX_WEB_CAPTURE_RASTER_BYTES, MAX_WEB_CAPTURE_WIDTH,
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
#[cfg(feature = "local-server")]
pub use frame_policy::{FrameGrant, FramePolicyHandle, HttpFrameOrigin};
#[cfg(feature = "application-ipc")]
pub use ipc::{IpcRegistry, DEFAULT_MAX_IPC_PAYLOAD_BYTES, IPC_VERSION};
#[cfg(all(feature = "local-server", feature = "application-ipc"))]
pub use local_server::bind_owned_local_server;
#[cfg(all(
    feature = "local-server",
    feature = "application-ipc",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
pub use local_server::{local_ipc_runtime_asset, LOCAL_IPC_RUNTIME_PATH};
#[cfg(feature = "local-server")]
pub use local_server::{
    run_local_server_frame, HostCloseError, HostLifetime, HostLifetimeOwner, LocalServerAppBuilder,
    LocalServerFrame, LocalServerOptions, LoopbackOrigin, UrlActivation,
    UrlActivationRegistrationError, MAX_URL_ACTIVATIONS_PER_BATCH, MAX_URL_ACTIVATION_BYTES,
};
#[cfg(feature = "native-services")]
pub use native_services::{
    ContentGeometry, GeometryRequest, NativeOpen, NativeServiceError, NativeServices,
    ScreenRectPoints, MAX_NATIVE_DOCUMENT_PATH_BYTES, MAX_NATIVE_URL_BYTES,
};
#[cfg(feature = "native-services")]
pub use native_theme::{ThemeMode, ThemeRequest, ThemeState};
pub use navigation::is_allowed_navigation_url;
#[cfg(feature = "source")]
pub use package::{package_desktop_bundle, DesktopPackageOptions, DesktopPackageResult};
#[cfg(feature = "packaging")]
pub use package_precompiled::{
    package_precompiled_host, PrecompiledHostOptions, PrecompiledPackageResult,
    PrecompiledResource, ResourceKind,
};
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
