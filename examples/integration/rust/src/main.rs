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
//!
//!   # Drive a progressive protocol through runtime boundary cursors
//!   cargo run -- streaming-protocol.bin state.json --plugin=webui --streaming

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::io::Write;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryMode, FlushWriter, Protocol, RenderOptions, ResponseWriter, WebUIHandler,
};

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

impl FlushWriter for StdoutWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        std::io::stdout().flush().map_err(Into::into)
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: {} <protocol.bin> <state.json> [--plugin=webui] [--streaming]",
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
    let options = RenderOptions::new("index.html", "/");
    if args.iter().any(|argument| argument == "--streaming") {
        render_streaming(&handler, &protocol, &state, &options, &mut writer)?;
    } else {
        handler
            .render(&protocol, &state, &options, &mut writer)
            .context("Failed to render")?;
    }

    Ok(())
}

fn render_streaming(
    handler: &WebUIHandler,
    protocol: &Protocol,
    state: &serde_json::Value,
    options: &RenderOptions<'_>,
    writer: &mut StdoutWriter,
) -> Result<()> {
    let mut session = handler
        .stream_response(protocol, options, writer)
        .context("Failed to open streaming session")?;
    let mut step = session.start(state).context("Failed to start stream")?;
    while !step.done {
        let boundary = step
            .boundary
            .as_ref()
            .context("Streaming step is unfinished but has no boundary descriptor")?;
        let instance_id = boundary.instance_id;
        let owner = boundary.owner.clone();
        let name = boundary.name.clone();
        step = session
            .resume(instance_id, state, BoundaryMode::Final)
            .with_context(|| format!("Failed to resume boundary {owner}/{name}"))?;
    }
    Ok(())
}
