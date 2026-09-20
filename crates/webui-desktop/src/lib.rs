// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Runtime-neutral primitives for WebUI desktop applications.
//!
//! The webview backend lives in the desktop binary crate. This crate keeps the
//! high-sensitivity behavior testable without creating native windows:
//! safe asset path resolution, custom-protocol routing, startup SSR rendering,
//! bundle metadata, and protobuf IPC dispatch.

mod app;
mod bundle;
mod error;
mod event;
mod ipc;
mod navigation;
mod package;
mod path;
mod protocol;
mod runtime;
mod window;
mod window_state;

pub use app::{DesktopApp, DesktopAppBuilder};
pub use bundle::{
    build_desktop_bundle, BundleAsset, BundleIntegrity, DesktopBundleManifest,
    DesktopBundleOptions, DesktopDownloadPolicy, DesktopJumpListItem, DesktopMenu, DesktopMenuItem,
    DesktopPackageTarget, DesktopPopoverPolicy, DesktopShellConfig, TrayConfig,
};
pub use error::{DesktopError, Result};
pub use event::{
    DesktopEvent, DesktopHostMessage, DesktopHostMessageError, EventHandler, EventJavascriptError,
    EventRegistry, EventResponse, WindowCommand, WindowCommandError, WindowHandle, WindowId,
    DRAG_REGION_SCRIPT, MAX_HOST_MESSAGE_BYTES,
};
pub use ipc::{
    DesktopIpcError, DesktopIpcRequest, DesktopIpcResponse, IpcHandlerError, IpcRegistry,
    DEFAULT_MAX_IPC_PAYLOAD_BYTES, IPC_VERSION,
};
pub use navigation::is_allowed_navigation_url;
pub use package::{package_desktop_bundle, DesktopPackageOptions, DesktopPackageResult};
pub use protocol::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DEFAULT_MAX_ASSET_BYTES,
    IPC_ENDPOINT,
};
pub use runtime::{
    ApiContext, ApiRouteRegistry, DesktopBundleConfig, DesktopRuntime, DesktopSourceConfig,
};
pub use runtime::{RouteContext, RouteStateRegistry};
pub use window::{
    apply_window_css, window_css_block, DesktopPlatform, Rgba, RgbaParseError, TitlebarStyle,
    WindowEffect, WindowInsets, WindowOptions,
};
pub use window_state::{DisplayBounds, WindowState, WindowStateError, WindowStateStore};
