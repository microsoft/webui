// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Typed, document-scoped binary application IPC.
//!
//! Handles are weak: only [`IpcWindowOwner`] owns a frame. Application futures
//! execute on bounded workers, never on native completion drivers. Cancellation
//! cannot interrupt blocking Rust code; its task and memory permits remain held
//! until that code actually returns.

mod admission;
mod bridge;
mod codec;
mod credentials;
mod engine;
mod error;
mod executor;
mod limits;
mod registry;
mod session;
mod validation;
/// Fixed-layout envelope wire types (see `DESIGN.md` for the byte layout).
pub mod wire;

pub use admission::DocumentActivation;
pub use bridge::*;
pub use codec::*;
pub use error::*;
pub use limits::*;
pub use registry::*;
pub use session::*;
pub use validation::*;

use std::future::Future;
use std::pin::Pin;

/// Current application envelope version (no v1 fallback). Bumped from `2` to
/// `3` when the wire format switched from protobuf to a fixed byte layout;
/// both ends of this transport ship in the same binary, so a hard version
/// bump with no dual-format negotiation is the correct, intentional contract.
pub const IPC_VERSION: u32 = 3;
/// Default complete encoded frame bound.
pub const DEFAULT_MAX_IPC_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Owned asynchronous IPC completion; polling this never polls application code.
pub type IpcFuture<T> = Pin<Box<dyn Future<Output = Result<T, IpcError>> + Send + 'static>>;
