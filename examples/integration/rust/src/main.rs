// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Minimal Rust example: load a pre-built protocol.bin, pass state JSON,
//! and print rendered HTML to stdout.
//!
//! Usage:
//!   # First, build the hello-world app
//!   cargo run -p microsoft-webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
//!
//!   # Then render it
//!   cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
//!
//!   # Render with WebUI Framework hydration markers
//!   cargo run -- ../../app/contact-book-manager/dist/protocol.bin ../../app/contact-book-manager/data/state.json --plugin=webui

use anyhow::{Context, Result};
use std::env;
use std::fs;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

struct StdoutWriter;

impl ResponseWriter for StdoutWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        print!("{content}");
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        println!();
        Ok(())
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: {} <protocol.bin> <state.json> [--plugin=webui]",
            args[0]
        );
        std::process::exit(1);
    }

    let protocol_path = &args[1];
    let state_path = &args[2];

    // Check for --plugin=<name> flag.
    let plugin_name = args.iter().find_map(|a| a.strip_prefix("--plugin="));

    let protocol_bytes = fs::read(protocol_path)
        .with_context(|| format!("Failed to load protocol: {protocol_path}"))?;
    let protocol = Protocol::from_protobuf(&protocol_bytes)
        .with_context(|| format!("Failed to decode protocol: {protocol_path}"))?;

    let state_json = fs::read_to_string(state_path)
        .with_context(|| format!("Failed to read state: {state_path}"))?;
    let state: serde_json::Value =
        serde_json::from_str(&state_json).context("Failed to parse state JSON")?;

    let handler = match plugin_name {
        Some("webui") => WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
        Some(unknown) => {
            anyhow::bail!("Unknown plugin: {unknown}. This example supports \"webui\".")
        }
        None => WebUIHandler::new(),
    };
    let mut writer = StdoutWriter;
    handler
        .render(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .context("Failed to render")?;

    Ok(())
}
