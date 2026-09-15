// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod build;
pub mod common;
pub mod dev;
pub mod inspect;
pub mod serve;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Build a WebUI application from an app folder
    Build(build::BuildArgs),
    /// Develop a WebUI app with warm JS/TS builds, native SSR, and live reload
    Dev(dev::DevArgs),
    /// Inspect a protocol.bin file and output JSON to stdout
    Inspect(inspect::InspectArgs),
    /// Serve templates with native rendering and optional live reload
    Serve(serve::ServeArgs),
}
