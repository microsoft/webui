// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native desktop runners for `microsoft-webui-desktop`.

mod frame;
pub use frame::{
    find_packaged_resources_dir, run_frame, run_packaged_app, run_runtime, DesktopFrame,
    DesktopFrameBackend, DesktopFrameCapabilities, PlatformFrameBackend,
};

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;
