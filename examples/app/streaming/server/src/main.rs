// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTTP server for the streaming priority-hydration example.
//!
//! The whole point is [`render_page`] below: it is the host-driven
//! `WebUIHandler::stream_response` path over a real `StreamingWriter`,
//! following the boundary contract in DESIGN.md ("Progressive Streaming
//! Hydration"). Everything else here is ordinary
//! server wiring, and the two demo-only concerns are quarantined:
//!
//! - [`pacing`] races independently ready weather and feed work, then sends
//!   bounded commands to the one blocking worker that owns the response.
//! - [`app`] holds the protocol build and sample feed data, which look the
//!   same in every WebUI example.

mod app;
mod assets;
mod jitter;
mod pacing;
mod test_controls;
mod weather;

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use actix_web::cookie::Cookie;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::{mpsc, Semaphore};
use tokio_stream::wrappers::ReceiverStream;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{BoundaryMode, HandlerError, Protocol, RenderOptions, WebUIHandler};
use webui_example_dist_assets::{load_dist_assets, CachedAsset};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

use crate::assets::{asset_response, insert_generated_css};
use crate::pacing::RenderCommand;
use crate::test_controls::TestControls;

const ENTRY: &str = "index.html";
const TEST_SESSION_COOKIE: &str = "webui-stream-test";

struct AppCtx {
    protocol: Arc<Protocol>,
    state: Arc<Value>,
    pool: Arc<ChunkPool>,
    feed_delay_min_ms: u64,
    feed_delay_max_ms: u64,
    render_permits: Arc<Semaphore>,
    test_controls: Option<Arc<TestControls>>,
    assets: HashMap<String, CachedAsset>,
}

/// Stream `GET /` through a host-controlled response session.
///
/// A bounded async producer races feed and forecast readiness. One blocking
/// worker owns both the renderer and transport, so chunks stay ordered without
/// locks and the browser receives backpressure through `StreamingWriter`.
async fn render_page(req: HttpRequest, ctx: web::Data<AppCtx>) -> HttpResponse {
    let render_permit = match Arc::clone(&ctx.render_permits).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return HttpResponse::ServiceUnavailable()
                .insert_header(("Retry-After", "1"))
                .body("streaming render capacity is temporarily exhausted");
        }
    };
    let protocol = Arc::clone(&ctx.protocol);
    // Clone the `Arc`, not the JSON tree: every request shares the one
    // `Value` built at startup.
    let state = Arc::clone(&ctx.state);
    let pool = Arc::clone(&ctx.pool);
    let feed_delay_min_ms = ctx.feed_delay_min_ms;
    let feed_delay_max_ms = ctx.feed_delay_max_ms;
    let test_id = test_session_id(&req).map(str::to_owned);
    let test_session = test_id
        .as_deref()
        .and_then(|id| ctx.test_controls.as_ref()?.session(id));
    // Both channels are bounded: transport backpressure caps response bytes,
    // and command backpressure caps ready backend results.
    let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let (command_tx, mut command_rx) = mpsc::channel(1);
    let _driver = actix_web::rt::spawn(pacing::drive(
        command_tx,
        test_session.clone(),
        feed_delay_min_ms,
        feed_delay_max_ms,
    ));
    actix_web::rt::task::spawn_blocking(move || {
        let _render_permit = render_permit;
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let mut writer =
            StreamingWriter::new_pooled(tx, pool).with_flush_timeout(Duration::from_secs(30));
        let result = (|| {
            let opts = RenderOptions::new(ENTRY, "/");
            let mut response = handler.stream_response(&protocol, &opts, &mut writer)?;
            let weather = response.boundary("weather-shell")?;
            let composer = response.boundary("composer-ready")?;
            let feed = [
                response.boundary("feed-batch-1")?,
                response.boundary("feed-batch-2")?,
                response.boundary("feed-batch-3")?,
            ];

            response.write_shell(&state)?;
            response.write_boundary(weather, &state, BoundaryMode::Updatable)?;
            response.write_boundary(composer, &state, BoundaryMode::Final)?;

            while let Some(command) = command_rx.blocking_recv() {
                match command {
                    RenderCommand::Weather(forecast) => response.update(weather, &forecast)?,
                    RenderCommand::Feed(index) => {
                        let boundary = feed.get(index).copied().ok_or_else(|| {
                            HandlerError::Invariant(format!(
                                "streaming example received invalid feed batch {index}"
                            ))
                        })?;
                        response.write_boundary(boundary, &state, BoundaryMode::Final)?;
                    }
                    RenderCommand::Finish => return response.finish(&state),
                }
            }
            Err(HandlerError::Writer(
                "streaming example command producer stopped before finish".to_owned(),
            ))
        })();
        if let Err(err) = result {
            // The response's status/headers are already on the wire by the
            // time this can fail — we cannot turn this into an HTTP error.
            // Log for operators; the client simply sees a truncated body.
            eprintln!("streaming render failed: {err}");
        }
    });

    let stream = ReceiverStream::new(rx).map(Ok::<Bytes, actix_web::Error>);
    let mut response = HttpResponse::Ok();
    response
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"));
    if test_session.is_some() {
        if let Some(id) = test_id {
            response.cookie(
                Cookie::build(TEST_SESSION_COOKIE, id)
                    .path("/")
                    .http_only(true)
                    .same_site(actix_web::cookie::SameSite::Strict)
                    .finish(),
            );
        }
    }
    response.streaming(stream)
}

fn test_session_id(req: &HttpRequest) -> Option<&str> {
    req.query_string()
        .split('&')
        .find_map(|part| part.strip_prefix("test="))
        .filter(|id| !id.is_empty())
}

async fn release_test_feed(path: web::Path<String>, ctx: web::Data<AppCtx>) -> HttpResponse {
    let Some(session) = ctx
        .test_controls
        .as_ref()
        .and_then(|controls| controls.existing_session(path.as_str()))
    else {
        return HttpResponse::NotFound().finish();
    };
    session.release_next_feed_gap();
    HttpResponse::NoContent().finish()
}

async fn release_test_weather(path: web::Path<String>, ctx: web::Data<AppCtx>) -> HttpResponse {
    let Some(session) = ctx
        .test_controls
        .as_ref()
        .and_then(|controls| controls.existing_session(path.as_str()))
    else {
        return HttpResponse::NotFound().finish();
    };
    session.release_weather();
    HttpResponse::NoContent().finish()
}

async fn release_test_all(path: web::Path<String>, ctx: web::Data<AppCtx>) -> HttpResponse {
    let Some(session) = ctx
        .test_controls
        .as_ref()
        .and_then(|controls| controls.existing_session(path.as_str()))
    else {
        return HttpResponse::NotFound().finish();
    };
    session.release_all();
    HttpResponse::NoContent().finish()
}

/// Serve any file this app's client build produced under `dist/` — the
/// entry bundle and every hashed shared chunk esbuild's code splitting
/// emits. The exact chunk filenames are not known until the client build
/// runs, so this is a single catch-all lookup against the map loaded once
/// at startup in [`main`], not a fixed list of routes.
async fn serve_asset(req: HttpRequest, ctx: web::Data<AppCtx>) -> HttpResponse {
    let relative = req.path().trim_start_matches('/');
    asset_response(&ctx.assets, relative).unwrap_or_else(|| HttpResponse::NotFound().finish())
}

#[derive(Debug, Parser)]
#[command(name = "streaming-example-server")]
struct Args {
    /// Port to listen on.
    #[arg(long, default_value_t = 3020)]
    port: u16,

    /// CSS delivery strategy: link, style, or module.
    #[arg(long, default_value = "style")]
    css: String,

    /// Base path for sub-path deployment (e.g., `/streaming/`). Injected
    /// as `basePath` in template state for `<base href="{{basePath}}">`.
    #[arg(long, default_value = "/")]
    base_path: String,

    /// Lower bound, in milliseconds, of the randomized pause before each
    /// feed batch is flushed.
    #[arg(long, default_value_t = 500)]
    feed_delay_min_ms: u64,

    /// Upper bound, in milliseconds, of the randomized pause before each
    /// feed batch is flushed. Set equal to the lower bound for a fixed,
    /// fully deterministic cadence.
    #[arg(long, default_value_t = 1_000)]
    feed_delay_max_ms: u64,

    /// Maximum renders admitted to the blocking pool at once. Excess requests
    /// receive 503 instead of joining an unbounded `spawn_blocking` queue.
    #[arg(long, default_value = "4")]
    max_concurrent_renders: NonZeroUsize,

    /// Enable explicit feed/weather release endpoints for Playwright.
    #[arg(long)]
    test_controls: bool,
}

#[actix_web::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let css = app::css_strategy(&args.css)?;
    if args.feed_delay_max_ms < args.feed_delay_min_ms {
        anyhow::bail!(
            "--feed-delay-max-ms ({}) is below --feed-delay-min-ms ({}). \
             Pass a max at or above the min, or set both to the same value \
             for a fixed cadence.",
            args.feed_delay_max_ms,
            args.feed_delay_min_ms
        );
    }
    let app_root =
        std::env::current_dir().context("Failed to determine streaming example app directory")?;

    let loaded = app::load(&app_root, css, &args.base_path)?;

    let dist_dir = app_root.join("dist");
    let mut assets = load_dist_assets(&dist_dir)
        .with_context(|| format!("Failed to load client assets from {}", dist_dir.display()))?;
    if !assets.contains_key("index.js") {
        anyhow::bail!(
            "{} is missing index.js — run `pnpm build:client` first",
            dist_dir.display()
        );
    }
    // Component stylesheets exist only in the build result, never in
    // `dist/`. The `link` and `module` strategies reference them by URL, so
    // they must be served or every shadow root's stylesheet 404s.
    insert_generated_css(&mut assets, loaded.css_files);

    let ctx = web::Data::new(AppCtx {
        protocol: loaded.protocol,
        state: Arc::new(loaded.state),
        pool: Arc::new(ChunkPool::new(64, StreamingWriter::CHUNK_TARGET + 1024)),
        feed_delay_min_ms: args.feed_delay_min_ms,
        feed_delay_max_ms: args.feed_delay_max_ms,
        render_permits: Arc::new(Semaphore::new(args.max_concurrent_renders.get())),
        test_controls: args
            .test_controls
            .then(|| Arc::new(TestControls::default())),
        assets,
    });

    let port = args.port;
    println!("streaming-example-server listening on http://127.0.0.1:{port}");
    HttpServer::new(move || {
        App::new()
            .app_data(ctx.clone())
            .route("/", web::get().to(render_page))
            .route("/__test/{session}/feed", web::post().to(release_test_feed))
            .route(
                "/__test/{session}/weather",
                web::post().to(release_test_weather),
            )
            .route("/__test/{session}/all", web::post().to(release_test_all))
            .default_service(web::route().to(serve_asset))
    })
    .bind(("0.0.0.0", port))?
    .workers(2)
    .run()
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Args;
    use crate::weather::WEATHER_DELAY_MIN_MS;
    use clap::Parser;

    #[test]
    fn parses_custom_port() {
        let args = Args::parse_from(["streaming-example-server", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }

    #[test]
    fn feed_delays_default_to_a_half_to_one_second_range() {
        let args = Args::parse_from(["streaming-example-server"]);
        assert_eq!(args.feed_delay_min_ms, 500);
        assert_eq!(args.feed_delay_max_ms, 1_000);
        assert_eq!(args.max_concurrent_renders.get(), 4);
        assert!(!args.test_controls);
    }

    #[test]
    fn zero_render_capacity_is_rejected_by_clap() {
        assert!(Args::try_parse_from([
            "streaming-example-server",
            "--max-concurrent-renders",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn feed_delays_are_overridable() {
        let args = Args::parse_from([
            "streaming-example-server",
            "--feed-delay-min-ms",
            "50",
            "--feed-delay-max-ms",
            "75",
        ]);
        assert_eq!(args.feed_delay_min_ms, 50);
        assert_eq!(args.feed_delay_max_ms, 75);
    }

    #[test]
    fn the_weather_outlasts_a_single_feed_gap() {
        // The panel only demonstrates independence from the stream if its
        // data cannot land before the first batch it is racing.
        let args = Args::parse_from(["streaming-example-server"]);
        assert!(
            WEATHER_DELAY_MIN_MS > args.feed_delay_min_ms,
            "the forecast would resolve before the first feed batch"
        );
    }
}
