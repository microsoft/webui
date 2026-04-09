// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod app;
mod cart;
mod catalog;
mod error;
mod extractors;
mod frontend;
mod rate_limit;
mod security;
mod server;
mod state;

use actix_web::{web, App, HttpServer};
use anyhow::{Context, Result};
use clap::Parser;

use crate::app::AppState;
use crate::security::security_headers;
use crate::server::configure_app;
use webui::CssStrategy;

#[derive(Debug, Parser)]
#[command(name = "marketplace-api")]
struct ApiArgs {
    #[arg(long, default_value_t = 3004)]
    port: u16,

    /// CSS delivery strategy: link, style, or module.
    #[arg(long, default_value = "link")]
    css: String,
}

fn main() -> Result<()> {
    let args = ApiArgs::parse();
    let port = args.port;
    let css = match args.css.as_str() {
        "link" => CssStrategy::Link,
        "style" => CssStrategy::Style,
        "module" => CssStrategy::Module,
        other => {
            anyhow::bail!("Unknown CSS strategy: {other}. Use \"link\", \"style\", or \"module\".")
        }
    };
    let app_root = std::env::current_dir().context("Failed to determine commerce app directory")?;

    let app_state = AppState::load(&app_root, css)?;

    eprintln!(
        "Commerce server: {} products, listening on :{}",
        app_state.product_count(),
        port
    );

    let state = web::Data::new(app_state);

    actix_web::rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                .wrap(actix_web::middleware::Compress::default())
                .wrap(security_headers())
                .app_data(state.clone())
                .configure(configure_app)
        })
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
