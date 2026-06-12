// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod commands;
mod utils;

use clap::{CommandFactory, Parser};
use commands::Commands;
use utils::output::OutputFormat;

#[derive(Parser)]
#[command(name = "webui", about = "WebUI build tool")]
struct Cli {
    /// Output format: `human` (colorized terminal, default) or `json`
    /// (machine-readable diagnostics on stdout for editors, CI, and tools).
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

fn main() {
    let cli = Cli::parse();
    utils::output::set_format(cli.format);

    let Some(command) = &cli.command else {
        Cli::command().print_help().ok();
        return;
    };

    let result = match command {
        Commands::Build(args) => commands::build::execute(args),
        Commands::Inspect(args) => commands::inspect::execute(args),
        Commands::Serve(args) => commands::serve::execute(args),
    };

    if let Err(err) = result {
        std::process::exit(utils::error::exit_code(&err));
    }
}
