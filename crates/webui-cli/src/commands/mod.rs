pub mod build;
pub mod common;
pub mod inspect;
pub mod start;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Build a WebUI application from an app folder
    Build(build::BuildArgs),
    /// Inspect a protocol.bin file and output JSON to stdout
    Inspect(inspect::InspectArgs),
    /// Start a development server with live reload
    Start(start::StartArgs),
}
