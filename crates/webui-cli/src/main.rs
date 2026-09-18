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
        Commands::Build(args) => commands::build::execute(args).map(|()| 0),
        Commands::Inspect(args) => commands::inspect::execute(args).map(|()| 0),
        Commands::Serve(args) => commands::serve::execute(args),
    };

    match result {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(err) => std::process::exit(utils::error::exit_code(&err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_timeout_is_opt_in_and_positive() -> Result<(), clap::Error> {
        for watch in [false, true] {
            let mut args = vec!["webui", "serve"];
            if watch {
                args.push("--watch");
            }
            let cli = Cli::try_parse_from(&args)?;
            let Some(Commands::Serve(serve)) = cli.command else {
                panic!("expected serve command");
            };
            assert_eq!(serve.shutdown_timeout, None);
            for value in ["1", "30"] {
                let cli =
                    Cli::try_parse_from(args.iter().copied().chain(["--shutdown-timeout", value]))?;
                let Some(Commands::Serve(serve)) = cli.command else {
                    panic!("expected serve command");
                };
                assert_eq!(
                    serve.shutdown_timeout.map(std::num::NonZeroU64::get),
                    value.parse().ok()
                );
            }
            for value in ["0", "-1", "1.5", "invalid"] {
                assert!(Cli::try_parse_from(
                    args.iter().copied().chain(["--shutdown-timeout", value])
                )
                .is_err());
            }
        }
        assert!(Cli::try_parse_from(["webui", "build", "--shutdown-timeout", "1"]).is_err());
        Ok(())
    }
}
