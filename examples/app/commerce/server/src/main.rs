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
#[cfg(not(target_vendor = "apple"))]
use native_tls::Identity;
use native_tls::TlsAcceptor as PlatformTlsAcceptor;

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
            eprintln!("  HTTPS (HTTP/2, HTTP/1.1 fallback) listening on https://localhost:{port}");
            let listener = std::net::TcpListener::bind(("0.0.0.0", port))?;
            build_https_server(listener, port, state, tls)?.await
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

fn build_https_server(
    listener: std::net::TcpListener,
    port: u16,
    state: web::Data<AppState>,
    tls: PlatformTlsAcceptor,
) -> std::io::Result<Server> {
    let tls_acceptor = TlsAcceptor::from(tls);
    let native_tls_acceptor = NativeTlsAcceptor::new(tls_acceptor);

    let server = Server::build()
        .shutdown_timeout(5)
        .listen("marketplace-api", listener, move || {
            let factory = App::new()
                .wrap(actix_web::middleware::Compress::default())
                .wrap(security_headers())
                .app_data(state.clone())
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
        .run();
    Ok(server)
}

/// Build a platform-native TLS acceptor from caller-provided PEM files.
fn build_tls_acceptor(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<PlatformTlsAcceptor> {
    #[cfg(target_vendor = "apple")]
    {
        let _ = (cert_path, key_path);
        anyhow::bail!(
            "Direct HTTPS is unavailable on Apple platforms because native-tls does not support \
             server-side ALPN there; use --no-tls behind an HTTP/2 TLS-terminating proxy"
        );
    }

    #[cfg(not(target_vendor = "apple"))]
    {
        let (certificate, key) = match (cert_path, key_path) {
            (Some(cert), Some(key)) => load_pem_files(cert, key)?,
            (None, None) => anyhow::bail!(
                "TLS certificate and key are required unless --no-tls is set; \
                 provide --tls-cert <path> and --tls-key <path>"
            ),
            _ => {
                anyhow::bail!("--tls-cert and --tls-key must be provided together, or use --no-tls")
            }
        };

        build_tls_acceptor_from_pem(&certificate, &key)
    }
}

#[cfg(not(target_vendor = "apple"))]
fn build_tls_acceptor_from_pem(certificate: &[u8], key: &[u8]) -> Result<PlatformTlsAcceptor> {
    let identity = Identity::from_pkcs8(certificate, key).context(
        "Failed to build platform-native TLS identity; the private key must be PKCS#8 PEM",
    )?;
    let mut builder = PlatformTlsAcceptor::builder(identity);
    builder.accept_alpn(&["h2", "http/1.1"]);
    builder
        .build()
        .context("Failed to create platform-native TLS acceptor")
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
    #[cfg(not(target_vendor = "apple"))]
    use super::{build_https_server, build_tls_acceptor_from_pem};
    use super::{build_tls_acceptor, protocol_from_alpn, ApiArgs};
    use actix_http::Protocol;
    #[cfg(not(target_vendor = "apple"))]
    use actix_service::Service as _;
    #[cfg(not(target_vendor = "apple"))]
    use actix_web::web;
    use anyhow::{Context, Result};
    use clap::Parser;
    #[cfg(not(target_vendor = "apple"))]
    use native_tls::Certificate;
    #[cfg(not(target_vendor = "apple"))]
    use std::path::Path;

    // Test-only self-signed localhost identity generated with .NET CertificateRequest.
    #[cfg(not(target_vendor = "apple"))]
    const TEST_CERTIFICATE_PEM: &[u8] = br#"-----BEGIN CERTIFICATE-----
MIICyDCCAbCgAwIBAgIIfMjByGjAEVUwDQYJKoZIhvcNAQELBQAwFDESMBAGA1UE
AxMJbG9jYWxob3N0MB4XDTI2MTAxMjIxMDY0M1oXDTM2MTAxMzIxMDY0M1owFDES
MBAGA1UEAxMJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKC
AQEAygXnxYp/M6kbCsMapHjwv5amvkdC8082o+IBmcSLAbiXq7xyVkm1c87pgna/
KBZZjMRdjcxlsQtbxqzImWLolBW5ntupJ4o/EtQ4RZykAH/SBSU8HXHwRoVJIkpA
Zo5udLHIJ92ZjBJ30ByWT3qFlWNjGRwtO83VO9ql+MUwyS2xBDnEOdUBBXxTLAKM
CDse+0Ze2vsNyQJoSUwvshAwFThqAHbKsYsfeT21/WwxWfNJmTvrcAm2zHNtIl0r
isBTsmKiTlBlDUHdEEKrqo2NmAUlpSmB685in07/gwh9x9wt/EKBdI7RtdpRNxH4
98fN6/T1qem4i44TKsiatwJB0QIDAQABox4wHDAaBgNVHREEEzARgglsb2NhbGhv
c3SHBH8AAAEwDQYJKoZIhvcNAQELBQADggEBAADUm6L2fcV5VuqQNi9dNBWAdZpK
aOakgZXN0VsZsVq7vX+lqXKbDSbvaU8CZ47nDMxOWFyRPhnwN6Bkukdjfarng5+Y
tNKPn0p4DCW3eiZMgro0YRxZMKZ0pzzeSyyjSoOpdckXynhKAHm/WF8BsyZcuC1K
n4Ej5cvQipfn6/L7UAnmK4QyzSylXzh65r+GUU6kM0qd+B2+vfh2/HSrHl0qZFts
2DMdrfLAqDWY9L/0hvRcJflbdg3qHuSJJFQtxeF5nZxPtmmHfWxW/+XSpToJygE9
ARsd6aCM6/oMqCXX8X5PwzhnUPe/U0XCOvjXup4FkPoy0o+xgynR8hEAuYs=
-----END CERTIFICATE-----
"#;

    #[cfg(not(target_vendor = "apple"))]
    const TEST_PRIVATE_KEY_PEM: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDKBefFin8zqRsK
wxqkePC/lqa+R0LzTzaj4gGZxIsBuJervHJWSbVzzumCdr8oFlmMxF2NzGWxC1vG
rMiZYuiUFbme26knij8S1DhFnKQAf9IFJTwdcfBGhUkiSkBmjm50scgn3ZmMEnfQ
HJZPeoWVY2MZHC07zdU72qX4xTDJLbEEOcQ51QEFfFMsAowIOx77Rl7a+w3JAmhJ
TC+yEDAVOGoAdsqxix95PbX9bDFZ80mZO+twCbbMc20iXSuKwFOyYqJOUGUNQd0Q
QquqjY2YBSWlKYHrzmKfTv+DCH3H3C38QoF0jtG12lE3Efj3x83r9PWp6biLjhMq
yJq3AkHRAgMBAAECggEBAMMmHzRgS+3QXdHwk+zq8sxvr5TA67hy/XojmJ48EHdS
JBj+CYi23PQInOKs6IzFwi1Lhn+yQU1bhoUNZR1eQQLDVbJRyD3sum7q14zO+lGN
J4hW5pIe/RZyLCpWhY1L6wEItytjUa7QLDK5Wq1d0mpzPkgEQuGxPFj8JjV/7ld1
9yx4MRRc/85igSN9TuwynOBR3Z2NvtS2taNKFSeuF0l1RyO23ehTUkATKDonUD78
bgw6y62u9V4qoUwrHRssklPp9eshbPcUmdcvOPwG2YSqikpt+tJnIKI2os9Ek4fI
nAMLWz9iZ1pvr5wSk+OCf8xYqD3ubav558syFcIUxz0CgYEA3PFzuxBCRjGAR9qD
wb28ep+aCjJobLfHfAoccB9nEQDdjfwLgl4C2Sz3N1x090UtoSetz8c0Nq6HXz3B
XwjlOvYcmd7jn7e70xSloKNDOCcNAS1vT1fApj0Vs9cfapVrZLLpsPHC2YYvNgi0
mA/L1Vir879iq1+4JC8s/CyoIbsCgYEA6hPuwzkNbgK/60iSp9kXtg/jvSoW+glu
es1sfgcykNhd+pSKI12FAUCzTbrkbgOn88KupAOZJplc250GGMWbU//Msh3ASnMO
n0voGFbxBkctMWyRxl10ZCvQRjmVPGjsBHf+m8sKWI/6pbO2P74kQgwqDMeGntCN
jgIlravu++MCgYBijW0zBtiZGOGAq47T779P7zs1yzBbNkrMIHJ47Pj7U6h/+epk
V2zr1KvdTOd7mKb22I0mPlQjBAysRg6oRn6Bo2A0noBoMPud9vq+QbTtKjYZzVt2
R81OGbdBhfakg0vpGgk1fVGN19XdQ1rS7VDdtHTXAAqSKNspgMWPIphImwKBgCUS
+yIYizP9Y4BUgkXrcIH5ikyPugvoKoNwvdpIMY4XdusVU7u+OC62nK+XBk8WbSeC
kj6QA1XFvJtcB3cs9SkcvsMHS0l/Z7V7REt2mb4wz8ugMoaEjsa+2cIFpMlopNkg
WEc8pWrZgw1zpU37Q6riWlaj04Kb7Y+sb2NOdA+ZAoGBAKXgQRVBUECfqYuHaSgP
BkF6SEnD+tCmEnp90NHpSswHUqeEGxbqPwj4zpxaAw6UBdGFtDgREmDNFeyrJMBj
koxpNyC2ZWgNDOJSxSCCnC4BSMeJNC1mOKmvx8HSXk0fLjzqymLfJI691B38GXyq
oIU51zOAgvknQMLr2aNPzPZJ
-----END PRIVATE KEY-----
"#;

    #[test]
    fn parses_custom_port() {
        let args = ApiArgs::parse_from(["marketplace-api", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn requires_certificate_and_key_for_tls() {
        let error = build_tls_acceptor(None, None)
            .err()
            .expect("TLS configuration without certificate inputs must fail");
        assert!(error.to_string().contains("--no-tls"));
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn rejects_partial_certificate_inputs() {
        let error = build_tls_acceptor(Some("certificate.pem"), None)
            .err()
            .expect("TLS configuration with only one PEM path must fail");
        assert!(error.to_string().contains("provided together"));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn rejects_apple_https_before_validating_inputs() {
        let error = build_tls_acceptor(Some("missing-certificate.pem"), Some("missing-key.pem"))
            .err()
            .expect("direct HTTPS must fail on Apple targets");
        assert!(error.to_string().contains("unavailable on Apple platforms"));
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

    #[cfg(not(target_vendor = "apple"))]
    #[actix_web::test]
    async fn serves_http2_through_native_tls_service() -> Result<()> {
        let commerce_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("commerce server crate must have a parent directory")?;
        let app_state =
            crate::app::AppState::load(commerce_root, webui::CssStrategy::Module, false, "/")?;
        let state = web::Data::new(app_state);
        let tls = build_tls_acceptor_from_pem(TEST_CERTIFICATE_PEM, TEST_PRIVATE_KEY_PEM)?;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let server = build_https_server(listener, port, state, tls)?;
        let handle = server.handle();
        actix_web::rt::spawn(server);

        let mut connector = native_tls::TlsConnector::builder();
        connector.add_root_certificate(Certificate::from_pem(TEST_CERTIFICATE_PEM)?);
        connector.request_alpns(&["h2"]);
        let connector = actix_tls::connect::native_tls::TlsConnector::new(connector.build()?);
        let tcp = actix_web::rt::net::TcpStream::connect(("127.0.0.1", port)).await?;
        let connection = actix_tls::connect::Connection::new(String::from("localhost"), tcp);
        let tls = connector.call(connection).await?.into_parts().0;
        assert_eq!(tls.get_ref().negotiated_alpn()?, Some(b"h2".to_vec()));

        let (mut client, connection) = h2::client::handshake(tls).await?;
        let connection_task = actix_web::rt::spawn(connection);
        let request = http::Request::builder()
            .uri(format!("https://localhost:{port}/"))
            .body(())?;
        let (response, _) = client.send_request(request, true)?;
        let response = response.await?;
        assert!(response.status().is_success());

        connection_task.abort();
        handle.stop(true).await;
        Ok(())
    }
}
