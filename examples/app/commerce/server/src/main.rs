// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod app;
mod cart;
mod catalog;
mod error;
mod extractors;
mod frontend;
mod server;
mod state;

use actix_web::{web, App, HttpServer};
use anyhow::{Context, Result};
use clap::Parser;

use crate::app::AppState;
use crate::server::configure_app;

#[derive(Debug, Parser)]
#[command(name = "marketplace-api")]
struct ApiArgs {
    #[arg(long, default_value_t = 3100)]
    port: u16,
}

fn main() -> Result<()> {
    let args = ApiArgs::parse();
    let port = args.port;
    let app_root = std::env::current_dir().context("Failed to determine commerce app directory")?;

    let app_state = AppState::load(&app_root)?;

    eprintln!(
        "Commerce server: {} products, listening on :{}",
        app_state.product_count(),
        port
    );

    let state = web::Data::new(app_state);

    actix_web::rt::System::new().block_on(async move {
        HttpServer::new(move || App::new().app_data(state.clone()).configure(configure_app))
            .bind(("0.0.0.0", port))?
            .run()
            .await
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ApiArgs;
    use clap::Parser;

    #[test]
    fn parses_custom_port() {
        let args = ApiArgs::parse_from(["marketplace-api", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }
}
