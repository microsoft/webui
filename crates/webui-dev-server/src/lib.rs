// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared dev-server primitives for WebUI tooling.
//!
//! See the crate README for the high-level overview. The crate exposes:
//!
//! - [`LiveReload`] / [`livereload::sse_handler`] — Server-Sent Events
//!   broadcaster and actix handler for browser auto-reload.
//! - [`watch::spawn_watcher`] — debounced filesystem watcher.
//! - [`path`] — segment-aware basePath stripping and traversal-safe path
//!   resolution.
//! - [`reporter::RebuildReporter`] — terminal UX for rebuild loops
//!   (rolling line, timestamps, success/error formatting).
//! - [`worker::spawn_rebuild_worker`] — generic background worker that
//!   coalesces watcher ticks, runs a user-supplied rebuild closure,
//!   and reports the result.
//! - [`serve::serve_static_file`] — actix-friendly static file
//!   responder with basePath-aware routing and HTML livereload
//!   injection.

pub mod inject;
pub mod livereload;
pub mod path;
pub mod reporter;
pub mod serve;
pub mod watch;
pub mod worker;

pub use livereload::{sse_handler, LiveReload, ReloadEvent};
pub use reporter::{local_hms, RebuildReporter};
pub use serve::{serve_file_response, serve_static_file, NotFoundStrategy, StaticServeConfig};
pub use watch::{default_ignore_paths, spawn_watcher, WatchConfig, WatcherHandle};
pub use worker::{spawn_rebuild_worker, RebuildError, TickSender};
