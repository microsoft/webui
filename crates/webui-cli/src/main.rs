mod commands;
mod output;

use clap::Parser;
use commands::Commands;

#[derive(Parser)]
#[command(name = "webui", about = "WebUI build tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Build(args) => commands::build::execute(args),
        Commands::Inspect(args) => commands::inspect::execute(args),
    };

    if result.is_err() {
        std::process::exit(1);
    }
}
