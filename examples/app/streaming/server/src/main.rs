// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTTP server for the streaming priority-hydration example.
//!
//! The whole point is [`render_page`] below: it is the opt-in
//! `WebUIHandler::render_streaming` path over a real `StreamingWriter`,
//! following the Phase 1 boundary contract in DESIGN.md ("Progressive
//! Streaming Hydration — Phase 1"). Everything else here is ordinary
//! server wiring, and the two demo-only concerns are quarantined:
//!
//! - [`pacing`] holds the flush schedule and the [`FlushWriter`] adapter
//!   that makes this app's composer / weather / feed priority ordering
//!   observable over real network timing instead of completing in one
//!   scheduler tick.
//! - [`app`] holds the protocol build and sample feed data, which look the
//!   same in every WebUI example.

mod app;
mod assets;
mod jitter;
mod pacing;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use futures_util::StreamExt;
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{Protocol, WebUIHandler};
use webui_example_dist_assets::{load_dist_assets, CachedAsset};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::RenderOptions;

use crate::assets::{asset_response, insert_generated_css};
use crate::jitter::Jitter;
use crate::pacing::{gap_is_paced, CheckpointPacedWriter};

const ENTRY: &str = "index.html";

struct AppCtx {
    protocol: Arc<Protocol>,
    state: Arc<Value>,
    pool: Arc<ChunkPool>,
    feed_delay_min_ms: u64,
    feed_delay_max_ms: u64,
    assets: HashMap<String, CachedAsset>,
}

/// Stream `GET /` through `render_streaming`.
///
/// The two lines that matter are the `StreamingWriter` construction and the
/// `render_streaming` call. The rest is the actix plumbing any streaming
/// response needs, plus this demo's artificial pacing.
async fn render_page(ctx: web::Data<AppCtx>) -> HttpResponse {
    let protocol = Arc::clone(&ctx.protocol);
    // Clone the `Arc`, not the JSON tree: every request shares the one
    // `Value` built at startup.
    let state = Arc::clone(&ctx.state);
    let pool = Arc::clone(&ctx.pool);
    let feed_delay_min_ms = ctx.feed_delay_min_ms;
    let feed_delay_max_ms = ctx.feed_delay_max_ms;

    // Bounded channel: backpressure when the client is slow, no unbounded
    // memory growth. The blocking render task is spawned but *not* awaited
    // here — awaiting it would buffer the whole paced render (every
    // checkpoint delay) before the response could start, defeating the
    // entire point of streaming. Instead the task runs concurrently,
    // pushing each boundary's bytes onto `tx` as `render_streaming` commits
    // it, while the response below starts draining `rx` immediately.
    let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    actix_web::rt::task::spawn_blocking(move || {
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let inner =
            StreamingWriter::new_pooled(tx, pool).with_flush_timeout(Duration::from_secs(30));
        // One generator per request, so a reload re-orders how the page
        // fills in instead of replaying one hard-coded timeline.
        let mut jitter = Jitter::from_clock();
        let mut writer = CheckpointPacedWriter::new(inner, move |index| {
            if gap_is_paced(index) {
                jitter.delay_ms(feed_delay_min_ms, feed_delay_max_ms)
            } else {
                Duration::ZERO
            }
        });
        let opts = RenderOptions::new(ENTRY, "/");
        if let Err(err) = handler.render_streaming(&protocol, &state, &opts, &mut writer) {
            // The response's status/headers are already on the wire by the
            // time this can fail — we cannot turn this into an HTTP error.
            // Log for operators; the client simply sees a truncated body.
            eprintln!("streaming render failed: {err}");
        }
    });

    let stream = ReceiverStream::new(rx).map(Ok::<Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .streaming(stream)
}

/// Simulated backend latency bounds for `GET /api/weather`.
///
/// Deliberately slower than a single feed gap so the forecast lands *between*
/// feed batches, which is the whole point of the panel: it proves the weather
/// resolves independently of the response stream rather than riding along with
/// it.
const WEATHER_DELAY_MIN_MS: u64 = 700;
const WEATHER_DELAY_MAX_MS: u64 = 1_400;

/// Rotating sample forecasts, so a reload visibly returns new data rather
/// than looking like a cached response.
const FORECASTS: [(&str, &str); 4] = [
    ("68°F", "Partly cloudy"),
    ("54°F", "Light rain"),
    ("72°F", "Clear"),
    ("61°F", "Overcast"),
];

/// Where this demo pretends to be.
const FORECAST_LOCATION: &str = "Redmond, WA";

/// The weather panel's own data source, deliberately slow.
///
/// This is *not* part of the streamed response. The forecast is not ready in
/// document order, and native HTML streaming cannot reach back into a header
/// it has already streamed past, so the panel fetches it from the client
/// instead — see `src/weather-panel/weather-panel.ts`. Because streaming
/// hydration makes the panel interactive while the response is still open,
/// this request overlaps the remaining feed chunks rather than queueing
/// behind them.
async fn weather_api() -> HttpResponse {
    let mut jitter = Jitter::from_clock();
    let delay = jitter.delay_ms(WEATHER_DELAY_MIN_MS, WEATHER_DELAY_MAX_MS);
    tokio::time::sleep(delay).await;

    let (temperature, condition) = FORECASTS[jitter.index(FORECASTS.len())];

    // Built as a `Map` rather than with `serde_json::json!`, because that
    // macro expands to an `unwrap` the workspace lint bans.
    let mut forecast = Map::with_capacity(3);
    forecast.insert("location".to_owned(), Value::from(FORECAST_LOCATION));
    forecast.insert("temperature".to_owned(), Value::from(temperature));
    forecast.insert("condition".to_owned(), Value::from(condition));

    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .json(Value::Object(forecast))
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
        assets,
    });

    let port = args.port;
    println!("streaming-example-server listening on http://127.0.0.1:{port}");
    HttpServer::new(move || {
        App::new()
            .app_data(ctx.clone())
            .route("/", web::get().to(render_page))
            .route("/api/weather", web::get().to(weather_api))
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
    use super::{Args, FORECASTS, WEATHER_DELAY_MIN_MS};
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
    fn every_forecast_sample_is_populated() {
        for (temperature, condition) in FORECASTS {
            assert!(!temperature.is_empty(), "a forecast has no temperature");
            assert!(!condition.is_empty(), "a forecast has no condition");
        }
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
