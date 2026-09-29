// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod backend;
#[cfg(feature = "application-ipc")]
mod ipc;
#[cfg(feature = "application-ipc")]
mod ipc_control;
#[cfg(feature = "application-ipc")]
mod ipc_input;
#[cfg(feature = "application-ipc")]
mod ipc_message;
#[cfg(feature = "application-ipc")]
mod ipc_scheme;
#[cfg(feature = "application-ipc")]
mod ipc_wake;
#[cfg(all(feature = "local-server", feature = "application-ipc"))]
mod local_ipc;
#[cfg(feature = "local-server")]
mod local_server;
mod protocol;
mod response;
mod state;

/// Scheme and authority the Linux backend serves app content from, with no
/// trailing slash. WebKitGTK registers `webui` as a custom scheme, so the app
/// origin is expressed directly rather than through a virtual host.
pub(super) const APP_ORIGIN: &str = "webui://app";

pub(crate) use backend::run_frame;
pub use backend::{run_packaged_app, run_runtime};
#[cfg(feature = "local-server")]
pub(crate) use local_server::run_local_server_frame;
