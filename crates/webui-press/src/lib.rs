// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! High-performance documentation site builder powered by WebUI Framework.
//!
//! Builds static HTML documentation sites from markdown content using the
//! WebUI template engine and parallel rendering.

pub mod build;
pub mod content;
pub mod error;
pub mod markdown;
pub mod serve;
pub mod types;

pub use build::build_docs;
pub use serve::run_serve;
pub use types::DocsConfig;
