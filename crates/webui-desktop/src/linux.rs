// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod backend;
mod ipc;
mod ipc_control;
mod ipc_input;
mod ipc_message;
mod ipc_scheme;
mod ipc_wake;
mod protocol;
mod response;
mod state;

/// Scheme and authority the Linux backend serves app content from, with no
/// trailing slash. WebKitGTK registers `webui` as a custom scheme, so the app
/// origin is expressed directly rather than through a virtual host.
pub(super) const APP_ORIGIN: &str = "webui://app";

pub(crate) use backend::run_frame;
pub use backend::{run_packaged_app, run_runtime};
