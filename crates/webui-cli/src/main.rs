// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod commands;
mod utils;

use clap::{CommandFactory, Parser};
use commands::Commands;

#[derive(Parser)]
#[command(name = "webui", about = "WebUI build tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

fn main() {
    let cli = Cli::parse();

    let Some(command) = &cli.command else {
        Cli::command().print_help().ok();
        return;
    };

    let result = match command {
        Commands::Build(args) => commands::build::execute(args),
        Commands::Inspect(args) => commands::inspect::execute(args),
        Commands::Serve(args) => commands::serve::execute(args),
    };

    if result.is_err() {
        std::process::exit(1);
    }
}
