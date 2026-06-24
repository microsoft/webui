// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native desktop runners for `microsoft-webui-desktop`.

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;
