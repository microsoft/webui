// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod build;
pub mod common;
pub mod inspect;
pub mod serve;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Build a WebUI application from an app folder
    Build(build::BuildArgs),
    /// Inspect a protocol.bin file and output JSON to stdout
    Inspect(inspect::InspectArgs),
    /// Start a development server with live reload
    Serve(serve::ServeArgs),
}
