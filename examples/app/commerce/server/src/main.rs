// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod app;
mod cart;
mod catalog;
mod error;
mod extractors;
mod frontend;
mod image_proxy;
mod rate_limit;
mod security;
mod server;
mod state;

use actix_web::{web, App, HttpServer};
use anyhow::{Context, Result};
use clap::Parser;
use rustls::ServerConfig;

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

    /// Disable TLS and serve plain HTTP.  Use behind a TLS-terminating
    /// reverse proxy (e.g. Azure Container Apps ingress).
    #[arg(long)]
    no_tls: bool,

    /// Path to TLS certificate PEM file.  When omitted a self-signed
    /// certificate is generated for localhost development.
    #[arg(long)]
    tls_cert: Option<String>,

    /// Path to TLS private key PEM file.
    #[arg(long)]
    tls_key: Option<String>,
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
        "Commerce server: {} products, {} images ({} variants)",
        app_state.product_count(),
        app_state.image_count(),
        app_state.image_cache().variant_count(),
    );

    let state = web::Data::new(app_state);
    let no_tls = args.no_tls;
    let tls_config = if no_tls {
        None
    } else {
        Some(build_tls_config(
            args.tls_cert.as_deref(),
            args.tls_key.as_deref(),
        )?)
    };

    actix_web::rt::System::new().block_on(async move {
        let server = HttpServer::new(move || {
            App::new()
                .wrap(actix_web::middleware::Compress::default())
                .wrap(security_headers())
                .app_data(state.clone())
                .configure(configure_app)
        })
        .keep_alive(std::time::Duration::from_secs(75))
        .shutdown_timeout(5);

        if let Some(tls) = tls_config {
            eprintln!("  HTTPS (HTTP/2) listening on https://localhost:{port}");
            server.bind_rustls_0_23(("0.0.0.0", port), tls)?.run().await
        } else {
            eprintln!("  HTTP (plain) listening on http://0.0.0.0:{port}");
            server.bind(("0.0.0.0", port))?.run().await
        }
    })?;

    Ok(())
}

/// Build a rustls [`ServerConfig`] for HTTP/2.
///
/// When `cert_path` and `key_path` are provided, loads PEM files from
/// disk.  Otherwise, generates a self-signed certificate for `localhost`
/// using `rcgen` — suitable for local development.
fn build_tls_config(cert_path: Option<&str>, key_path: Option<&str>) -> Result<ServerConfig> {
    let (certs, key) = match (cert_path, key_path) {
        (Some(cert), Some(key)) => load_pem_files(cert, key)?,
        _ => generate_self_signed()?,
    };

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to build TLS config")?;

    Ok(config)
}

fn load_pem_files(
    cert_path: &str,
    key_path: &str,
) -> Result<(
    Vec<rustls::pki_types::CertificateDer<'static>>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let cert_data = std::fs::read(cert_path)
        .with_context(|| format!("Failed to read TLS certificate from {cert_path}"))?;
    let key_data = std::fs::read(key_path)
        .with_context(|| format!("Failed to read TLS key from {key_path}"))?;

    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&cert_data)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to parse TLS certificate PEM")?;

    let key = PrivateKeyDer::from_pem_slice(&key_data)
        .context("Failed to parse TLS key PEM")?;

    Ok((certs, key))
}

fn generate_self_signed() -> Result<(
    Vec<rustls::pki_types::CertificateDer<'static>>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .context("Failed to create certificate params")?;
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V6(
            std::net::Ipv6Addr::LOCALHOST,
        )));

    let key_pair = rcgen::KeyPair::generate().context("Failed to generate key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    let cert_der = cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
        .map_err(|e| anyhow::anyhow!("Failed to serialize private key: {e}"))?;

    eprintln!("  Generated self-signed TLS certificate for localhost");

    Ok((vec![cert_der], key_der))
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
