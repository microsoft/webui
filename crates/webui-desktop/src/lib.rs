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
mod ipc;
mod package;
mod path;
mod protocol;
mod runtime;

pub use app::{DesktopApp, DesktopAppBuilder};
pub use bundle::{
    build_desktop_bundle, BundleAsset, BundleIntegrity, DesktopBundleManifest,
    DesktopBundleOptions, DesktopDownloadPolicy, DesktopJumpListItem, DesktopMenu, DesktopMenuItem,
    DesktopPackageTarget, DesktopPopoverPolicy, DesktopShellConfig, WindowOptions,
};
pub use error::{DesktopError, Result};
pub use ipc::{
    DesktopIpcError, DesktopIpcRequest, DesktopIpcResponse, IpcHandlerError, IpcRegistry,
    IPC_VERSION,
};
pub use package::{package_desktop_bundle, DesktopPackageOptions, DesktopPackageResult};
pub use protocol::{
    DesktopHttpMethod, DesktopProtocolRequest, DesktopProtocolResponse, DEFAULT_MAX_ASSET_BYTES,
    IPC_ENDPOINT,
};
pub use runtime::{
    ApiContext, ApiRouteRegistry, DesktopBundleConfig, DesktopRuntime, DesktopSourceConfig,
};
pub use runtime::{RouteContext, RouteStateRegistry};
