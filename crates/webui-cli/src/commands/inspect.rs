use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;
use webui_protocol::WebUIProtocol;

#[derive(Args)]
pub struct InspectArgs {
    /// Path to a protocol.bin file
    pub file: PathBuf,
}

pub fn execute(args: &InspectArgs) -> Result<()> {
    let protocol = WebUIProtocol::from_protobuf_file(&args.file)
        .with_context(|| format!("Failed to read {}", args.file.display()))?;

    let json = protocol
        .to_json_pretty()
        .context("Failed to serialize to JSON")?;

    println!("{json}");
    Ok(())
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod inspect_tests;
