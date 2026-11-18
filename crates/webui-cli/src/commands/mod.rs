// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub(crate) mod app;
pub mod build;
pub(crate) mod bundle;
pub mod common;
mod component_asset_output;
pub(crate) mod init;
pub mod inspect;
mod metafile;
pub(crate) mod native;
pub(crate) mod sdk;
pub mod serve;
pub mod sidecar;
pub(crate) mod toolchain;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Check, build, develop and run a native application
    App(app::AppArgs),
    /// Build a WebUI application from an app folder
    Build(build::BuildArgs),
    /// Bundle browser assets and prove exact state projection natively
    Bundle(bundle::BundleArgs),
    /// Run WebUI desktop tooling through the desktop sidecar backend
    Desktop(sidecar::SidecarArgs),
    /// Build and serve sites through the native WebUI Press sidecar
    Press(sidecar::SidecarArgs),
    /// Inspect a protocol.bin file and output JSON to stdout
    Inspect(inspect::InspectArgs),
    /// Create an HTML-first WebUI application
    Init(init::InitArgs),
    /// Validate a standalone native SDK or run a command with its environment
    Sdk(sdk::SdkArgs),
    /// Read the version-matched offline application-authoring reference
    Reference(native::ReferenceArgs),
    /// Start a development server with live reload
    Serve(serve::ServeArgs),
}
