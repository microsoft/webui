// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod app;
mod cart;
mod catalog;
mod dist_assets;
mod error;
mod extractors;
mod frontend;
mod image_proxy;
mod rate_limit;
mod security;
mod server;
mod state;

use actix_web::{dev::Server, web, App, HttpServer};
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

    /// Group component stylesheets into per-tree bundles, splitting shared
    /// sheets into their own chunk. Not supported with `--css module`.
    #[arg(long)]
    css_bundle: bool,

    /// Base path for sub-path deployment (e.g., `/commerce/`).
    /// Injected as `basePath` in template state for `<base href="{{basePath}}">`.
    #[arg(long, default_value = "/")]
    base_path: String,
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

    let app_state = AppState::load(&app_root, css, args.css_bundle, &args.base_path)?;

    eprintln!(
        "Commerce server: {} products, {} images ({} variants)",
        app_state.product_count(),
        app_state.image_count(),
        app_state.image_cache().variant_count(),
    );

    let state = web::Data::new(app_state);

    actix_web::rt::System::new().block_on(async move {
        let listener = std::net::TcpListener::bind(("0.0.0.0", port))?;
        eprintln!("  HTTP/1.1 + HTTP/2 (h2c) listening on http://0.0.0.0:{port}");
        build_server(listener, state)?.await
    })?;

    Ok(())
}

/// Serve cleartext HTTP/1.1 and HTTP/2 prior-knowledge (h2c) on `listener`.
fn build_server(
    listener: std::net::TcpListener,
    state: web::Data<AppState>,
) -> std::io::Result<Server> {
    Ok(HttpServer::new(move || {
        App::new()
            .wrap(actix_web::middleware::Compress::default())
            .wrap(security_headers())
            .app_data(state.clone())
            .configure(configure_app)
    })
    .keep_alive(std::time::Duration::from_secs(75))
    .shutdown_timeout(5)
    .listen_auto_h2c(listener)?
    .run())
}

#[cfg(test)]
mod tests {
    use super::{build_server, ApiArgs};
    use actix_web::{dev::ServerHandle, web};
    use anyhow::{Context, Result};
    use clap::Parser;
    use std::path::Path;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn parses_custom_port() {
        let args = ApiArgs::parse_from(["marketplace-api", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }

    fn start_server() -> Result<(u16, ServerHandle)> {
        let commerce_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("commerce server crate must have a parent directory")?;
        let app_state =
            crate::app::AppState::load(commerce_root, webui::CssStrategy::Module, false, "/")?;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let server = build_server(listener, web::Data::new(app_state))?;
        let handle = server.handle();
        actix_web::rt::spawn(server);
        Ok((port, handle))
    }

    #[actix_web::test]
    async fn serves_http2_prior_knowledge_over_cleartext() -> Result<()> {
        let (port, handle) = start_server()?;

        let tcp = actix_web::rt::net::TcpStream::connect(("127.0.0.1", port)).await?;
        let (mut client, connection) = h2::client::handshake(tcp).await?;
        let connection_task = actix_web::rt::spawn(connection);
        let request = http::Request::builder()
            .uri(format!("http://127.0.0.1:{port}/"))
            .body(())?;
        let (response, _) = client.send_request(request, true)?;
        let response = response.await?;
        assert!(response.status().is_success());
        assert_eq!(response.version(), http::Version::HTTP_2);

        connection_task.abort();
        handle.stop(true).await;
        Ok(())
    }

    #[actix_web::test]
    async fn serves_http1_over_cleartext() -> Result<()> {
        let (port, handle) = start_server()?;

        let mut tcp = actix_web::rt::net::TcpStream::connect(("127.0.0.1", port)).await?;
        tcp.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await?;
        let mut status_line = [0_u8; 12];
        tcp.read_exact(&mut status_line).await?;
        assert_eq!(&status_line, b"HTTP/1.1 200");

        handle.stop(true).await;
        Ok(())
    }
}
