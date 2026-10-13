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

use actix_http::{HttpService, Protocol};
use actix_server::Server;
use actix_service::{fn_service, map_config, IntoServiceFactory, ServiceFactoryExt};
use actix_tls::accept::native_tls::{reexports::TlsAcceptor, Acceptor as NativeTlsAcceptor};
use actix_web::{dev::AppConfig, web, App, HttpServer};
use anyhow::{Context, Result};
use clap::Parser;
use native_tls::{Identity, TlsAcceptor as PlatformTlsAcceptor};

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

    /// Disable TLS and serve plain HTTP.  Use behind a TLS-terminating
    /// reverse proxy (e.g. Azure Container Apps ingress).
    #[arg(long)]
    no_tls: bool,

    /// Path to a TLS certificate PEM file. Required unless --no-tls is set.
    #[arg(long)]
    tls_cert: Option<String>,

    /// Path to a TLS private key PEM file. Required unless --no-tls is set.
    #[arg(long)]
    tls_key: Option<String>,

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
    let no_tls = args.no_tls;
    let tls_acceptor = if no_tls {
        None
    } else {
        Some(build_tls_acceptor(
            args.tls_cert.as_deref(),
            args.tls_key.as_deref(),
        )?)
    };

    actix_web::rt::System::new().block_on(async move {
        if let Some(tls) = tls_acceptor {
            eprintln!("  HTTPS (HTTP/2) listening on https://localhost:{port}");
            let tls_acceptor = TlsAcceptor::from(tls);
            let native_tls_acceptor = NativeTlsAcceptor::new(tls_acceptor);
            let server_state = state.clone();

            Server::build()
                .shutdown_timeout(5)
                .bind("marketplace-api", ("0.0.0.0", port), move || {
                    let factory = App::new()
                        .wrap(actix_web::middleware::Compress::default())
                        .wrap(security_headers())
                        .app_data(server_state.clone())
                        .configure(configure_app)
                        .into_factory()
                        .map_err(|error: actix_web::Error| error.error_response());

                    let http_service = HttpService::build()
                        .keep_alive(std::time::Duration::from_secs(75))
                        .secure()
                        .finish(map_config(factory, move |_| {
                            AppConfig::__priv_test_new(
                                true,
                                format!("localhost:{port}"),
                                std::net::SocketAddr::from(([0, 0, 0, 0], port)),
                            )
                        }))
                        .map_err(|_| ());

                    native_tls_acceptor
                        .clone()
                        .map_err(|_| ())
                        .and_then(fn_service(
                            |stream: actix_tls::accept::native_tls::TlsStream<
                                actix_web::rt::net::TcpStream,
                            >| async move {
                                let alpn = stream.get_ref().negotiated_alpn().map_err(|_| ())?;
                                Ok::<_, ()>((stream, protocol_from_alpn(alpn.as_deref()), None))
                            },
                        ))
                        .and_then(http_service)
                })?
                .run()
                .await
        } else {
            let server = HttpServer::new(move || {
                App::new()
                    .wrap(actix_web::middleware::Compress::default())
                    .wrap(security_headers())
                    .app_data(state.clone())
                    .configure(configure_app)
            })
            .keep_alive(std::time::Duration::from_secs(75))
            .shutdown_timeout(5);

            eprintln!("  HTTP (plain) listening on http://0.0.0.0:{port}");
            server.bind(("0.0.0.0", port))?.run().await
        }
    })?;

    Ok(())
}

/// Build a platform-native TLS acceptor from caller-provided PEM files.
fn build_tls_acceptor(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<PlatformTlsAcceptor> {
    let (certificate, key) = match (cert_path, key_path) {
        (Some(cert), Some(key)) => load_pem_files(cert, key)?,
        (None, None) => anyhow::bail!(
            "TLS certificate and key are required unless --no-tls is set; \
             provide --tls-cert <path> and --tls-key <path>"
        ),
        _ => anyhow::bail!("--tls-cert and --tls-key must be provided together, or use --no-tls"),
    };

    let identity = Identity::from_pkcs8(&certificate, &key).context(
        "Failed to build platform-native TLS identity; the private key must be PKCS#8 PEM",
    )?;

    #[cfg(target_vendor = "apple")]
    {
        let _ = identity;
        anyhow::bail!(
            "Direct HTTPS is unavailable on Apple platforms because native-tls does not support \
             server-side ALPN there; use --no-tls behind an HTTP/2 TLS-terminating proxy"
        );
    }

    #[cfg(not(target_vendor = "apple"))]
    {
        let mut builder = PlatformTlsAcceptor::builder(identity);
        builder.accept_alpn(&["h2", "http/1.1"]);
        builder
            .build()
            .context("Failed to create platform-native TLS acceptor")
    }
}

fn protocol_from_alpn(alpn: Option<&[u8]>) -> Protocol {
    if alpn == Some(b"h2") {
        Protocol::Http2
    } else {
        Protocol::Http1
    }
}

fn load_pem_files(cert_path: &str, key_path: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let cert_data = std::fs::read(cert_path)
        .with_context(|| format!("Failed to read TLS certificate from {cert_path}"))?;
    let key_data = std::fs::read(key_path)
        .with_context(|| format!("Failed to read TLS key from {key_path}"))?;

    Ok((cert_data, key_data))
}
#[cfg(test)]
mod tests {
    use super::{build_tls_acceptor, protocol_from_alpn, ApiArgs};
    use actix_http::Protocol;
    use clap::Parser;

    #[test]
    fn parses_custom_port() {
        let args = ApiArgs::parse_from(["marketplace-api", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }

    #[test]
    fn requires_certificate_and_key_for_tls() {
        let error = build_tls_acceptor(None, None)
            .err()
            .expect("TLS configuration without certificate inputs must fail");
        assert!(error.to_string().contains("--no-tls"));
    }

    #[test]
    fn rejects_partial_certificate_inputs() {
        let error = build_tls_acceptor(Some("certificate.pem"), None)
            .err()
            .expect("TLS configuration with only one PEM path must fail");
        assert!(error.to_string().contains("provided together"));
    }

    #[test]
    fn selects_http2_for_h2_alpn() {
        assert_eq!(protocol_from_alpn(Some(b"h2")), Protocol::Http2);
    }

    #[test]
    fn selects_http1_without_h2_alpn() {
        assert_eq!(protocol_from_alpn(Some(b"http/1.1")), Protocol::Http1);
        assert_eq!(protocol_from_alpn(None), Protocol::Http1);
    }
}
