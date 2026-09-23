// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use webui_desktop_build::{GenerateConfig, GenerateError};

#[derive(Args)]
pub(super) struct IpcArgs {
    #[command(subcommand)]
    command: IpcCommand,
}

#[derive(Subcommand)]
enum IpcCommand {
    /// Generate bindings from one shared application proto3 contract
    Generate(GenerateArgs),
}

#[derive(Args)]
struct GenerateArgs {
    /// Application proto3 root files
    #[arg(required = true, num_args = 1..)]
    roots: Vec<PathBuf>,

    /// Additional protobuf import directories
    #[arg(long = "include", short = 'I')]
    includes: Vec<PathBuf>,

    /// Directory for generated Rust bindings
    #[arg(long)]
    rust_out: PathBuf,

    /// Directory for generated TypeScript bindings
    #[arg(long)]
    ts_out: PathBuf,

    /// Compatibility lock recording method and event identifiers
    #[arg(long = "lock", default_value = "ipc-schema.lock.json")]
    lock_file: PathBuf,

    /// Check generated files and compatibility lock without writing them
    #[arg(long)]
    check: bool,

    /// Protobuf compiler executable, if it is not on PATH
    #[arg(long)]
    protoc: Option<PathBuf>,
}

pub(super) fn execute(args: IpcArgs) -> Result<()> {
    match args.command {
        IpcCommand::Generate(args) => generate(args),
    }
}

fn generate(args: GenerateArgs) -> Result<()> {
    let config = GenerateConfig {
        roots: args.roots,
        includes: args.includes,
        rust_out: args.rust_out,
        ts_out: args.ts_out,
        lock_file: args.lock_file,
        check: args.check,
        protoc: args.protoc,
    };
    webui_desktop_build::generate(&config)
        .with_context(|| "Desktop IPC binding generation failed")?;
    super::print_header("WebUI Desktop IPC");
    super::print_field("Rust", &config.rust_out.display());
    super::print_field("TypeScript", &config.ts_out.display());
    super::print_field("Lock", &config.lock_file.display());
    let message = if config.check {
        "Desktop IPC bindings are up to date"
    } else {
        "Desktop IPC bindings generated"
    };
    if super::is_json() {
        let mut output = serde_json::Map::new();
        output.insert("status".to_string(), "success".into());
        output.insert("checked".to_string(), config.check.into());
        output.insert(
            "rustOut".to_string(),
            config.rust_out.display().to_string().into(),
        );
        output.insert(
            "tsOut".to_string(),
            config.ts_out.display().to_string().into(),
        );
        println!("{}", serde_json::Value::Object(output));
    } else {
        super::print_finish(message);
    }
    Ok(())
}

pub(super) fn write_error_details(
    error: &GenerateError,
    output: &mut serde_json::Map<String, serde_json::Value>,
) {
    output.insert("code".to_string(), error.code().into());
    let help = match error {
        GenerateError::Schema { help, .. } | GenerateError::Tool { help, .. } => help.as_str(),
        GenerateError::Io { path, .. } => {
            output.insert("file".to_string(), path.display().to_string().into());
            "check the path and filesystem permissions"
        }
        GenerateError::Drift { .. } => "regenerate and commit all outputs together",
    };
    output.insert("help".to_string(), help.into());
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_shared_contract_generation_and_check_mode() {
        let cli = crate::Cli::try_parse_from([
            "webui-desktop",
            "ipc",
            "generate",
            "app.proto",
            "-I",
            "schemas",
            "--rust-out",
            "generated/rust",
            "--ts-out",
            "generated/ts",
            "--lock",
            "schema.lock.json",
            "--check",
        ])
        .unwrap();
        let Some(crate::Commands::Ipc(IpcArgs {
            command: IpcCommand::Generate(args),
        })) = cli.command
        else {
            panic!("expected IPC generation command");
        };
        assert_eq!(args.roots, [PathBuf::from("app.proto")]);
        assert_eq!(args.includes, [PathBuf::from("schemas")]);
        assert_eq!(args.lock_file, PathBuf::from("schema.lock.json"));
        assert!(args.check);
    }

    #[test]
    fn requires_explicit_binding_outputs() {
        assert!(
            crate::Cli::try_parse_from(["webui-desktop", "ipc", "generate", "app.proto"]).is_err()
        );
    }

    #[test]
    fn generation_json_errors_preserve_typed_diagnostics() {
        let cases = [
            (
                GenerateError::Schema {
                    code: "ipc-name-collision",
                    context: "example.Renderer.New".to_string(),
                    message: "generated constructor name is reserved".to_string(),
                    help: "rename the renderer method".to_string(),
                },
                "ipc-name-collision",
                "rename the renderer method",
                None,
            ),
            (
                GenerateError::Tool {
                    tool: "protoc".to_string(),
                    message: "compiler not found".to_string(),
                    help: "install protoc or pass --protoc".to_string(),
                },
                "ipc-tool",
                "install protoc or pass --protoc",
                None,
            ),
            (
                GenerateError::Io {
                    path: PathBuf::from("missing.proto"),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                },
                "ipc-io",
                "check the path and filesystem permissions",
                Some("missing.proto"),
            ),
            (
                GenerateError::Drift {
                    paths: vec![PathBuf::from("generated/ipc.rs")],
                },
                "ipc-drift",
                "regenerate and commit all outputs together",
                None,
            ),
        ];
        for (source, code, help, file) in cases {
            let error = anyhow::Error::new(source).context("Desktop IPC binding generation failed");
            let diagnostic = crate::error_json(&error);
            assert_eq!(diagnostic["code"], code);
            assert_eq!(diagnostic["help"], help);
            assert_eq!(diagnostic["file"].as_str(), file);
            assert_eq!(
                diagnostic["chain"][0],
                "Desktop IPC binding generation failed"
            );
            assert!(!diagnostic.to_string().contains("\\u001b"));
        }
    }
}
