// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub mod build;
pub mod common;
pub mod inspect;
pub mod serve;
pub mod sidecar;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Build a WebUI application from an app folder
    Build(build::BuildArgs),
    /// Run WebUI desktop tooling through the desktop sidecar backend
    Desktop(sidecar::SidecarArgs),
    /// Build and serve sites through the native WebUI Press sidecar
    Press(sidecar::SidecarArgs),
    /// Inspect a protocol.bin file and output JSON to stdout
    Inspect(inspect::InspectArgs),
    /// Start a development server with live reload
    Serve(serve::ServeArgs),
}
