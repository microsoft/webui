// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTTP server for the streaming priority-hydration example.
//!
//! Builds the real WebUI protocol from `src/index.html` via `webui::build`,
//! then serves `GET /` through the opt-in `WebUIHandler::render_streaming`
//! path over a real `StreamingWriter`, using the Progressive Streaming
//! Hydration Phase 1 boundary contract from DESIGN.md ("Progressive
//! Streaming Hydration — Phase 1"). Boundary flushes for the composer and
//! the first two feed batches are deliberately paced by
//! [`paced_writer::CheckpointPacedWriter`] so this app's own composer /
//! weather / feed priority ordering is observable over real network timing
//! rather than completing instantly in one scheduler tick.

mod assets;
mod jitter;
mod paced_writer;

use std::collections::HashMap;
use std::path::Path;
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
use webui::{
    build, load_token_file, resolve_theme_path, BuildOptions, CssStrategy, Plugin,
    ProjectionManifestSource, Protocol, WebUIHandler,
};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::RenderOptions;
use webui_tokens::{inject_into_state, resolve_tokens};

use crate::assets::{asset_response, insert_generated_css};
use crate::jitter::Jitter;
use crate::paced_writer::CheckpointPacedWriter;
use webui_example_dist_assets::{load_dist_assets, CachedAsset};

const THEME: &str = "@microsoft/webui-examples-theme";
const ENTRY: &str = "index.html";

/// Zero-based index of the flush whose pause precedes feed batch 1.
///
/// Flush 0 commits the weather shell, which carries no server data — the
/// composer must follow it immediately, so that gap is never paced. Flush 1
/// commits the composer, and its pause is the wait before the first feed
/// batch arrives.
const FIRST_FEED_GAP: usize = 1;

/// Feed batches, and therefore jittered gaps: one before each batch.
const FEED_BATCH_COUNT: usize = 3;

/// Simulated backend latency bounds for `GET /api/weather`.
///
/// Deliberately slower than a single feed gap so the forecast lands *between*
/// feed batches, which is the whole point of the panel: it proves the weather
/// resolves independently of the response stream rather than riding along with
/// it.
const WEATHER_DELAY_MIN_MS: u64 = 700;
const WEATHER_DELAY_MAX_MS: u64 = 1_400;

/// Whether the pause *after* flush `index` should be paced.
///
/// Flush 0 commits the weather shell, which carries no server data, so the
/// composer must follow it immediately. Flushes 1 through 3 commit the
/// composer and the first two feed batches, and each of their pauses is the
/// wait before the next feed batch. Everything after that — the last batch,
/// the implicit tail checkpoint, and the terminal record — closes the
/// response without further delay.
fn gap_is_paced(index: usize) -> bool {
    (FIRST_FEED_GAP..FIRST_FEED_GAP + FEED_BATCH_COUNT).contains(&index)
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

struct AppCtx {
    protocol: Arc<Protocol>,
    state: Arc<Value>,
    pool: Arc<ChunkPool>,
    feed_delay_min_ms: u64,
    feed_delay_max_ms: u64,
    assets: HashMap<String, CachedAsset>,
}

fn css_strategy(value: &str) -> Result<CssStrategy> {
    match value {
        "link" => Ok(CssStrategy::Link),
        "style" => Ok(CssStrategy::Style),
        "module" => Ok(CssStrategy::Module),
        other => {
            anyhow::bail!("Unknown CSS strategy: {other}. Use \"link\", \"style\", or \"module\".")
        }
    }
}

fn feed_post(post_id: &str, author: &str, text: &str, like_count: &str) -> Value {
    let mut post = Map::with_capacity(4);
    post.insert("postId".to_owned(), Value::String(post_id.to_owned()));
    post.insert("author".to_owned(), Value::String(author.to_owned()));
    post.insert("text".to_owned(), Value::String(text.to_owned()));
    post.insert("likeCount".to_owned(), Value::String(like_count.to_owned()));
    Value::Object(post)
}

/// Three explicit, complete feed batches (DESIGN.md's Phase 1 "Future
/// work" names an open-ended `<webui-stream>` append/feed-batch directive
/// as future work, not implemented here). Each batch is rendered by its
/// own `<boundary>` in `src/index.html` and committed with its own
/// checkpoint and flush.
fn feed_batches() -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let batch_1 = vec![
        feed_post(
            "1",
            "Ada",
            "Streaming boundaries keep the composer interactive immediately.",
            "4",
        ),
        feed_post(
            "2",
            "Grace",
            "This is feed batch one — it just committed.",
            "1",
        ),
    ];
    let batch_2 = vec![feed_post(
        "3",
        "Alan",
        "Feed batch two arrived after its own checkpoint.",
        "9",
    )];
    let batch_3 = vec![feed_post(
        "4",
        "Barbara",
        "Feed batch three is the lowest-priority chunk.",
        "0",
    )];
    (batch_1, batch_2, batch_3)
}

/// Build the WebUI protocol and its render-time state from `app_root`.
///
/// The theme is loaded once and passed into `BuildOptions::theme` so the
/// build validates every CSS custom property this app's templates and
/// components reference (see `webui_tokens::validate_required_tokens`),
/// then re-resolved against the build's actual token usage to produce the
/// per-theme CSS injected into `state.tokens.{light,dark}` for the
/// `{{{tokens.light}}}` / `{{{tokens.dark}}}` bindings in `index.html`.
///
/// The client build's `dist/webui-projection.json` is passed through so the
/// protocol uses `InitialStateStrategy::Components`. This is load-bearing for
/// streaming: without it every boundary checkpoint falls back to serializing
/// the *entire* state object, so each of the five envelopes would re-ship all
/// three feed batches plus both resolved token sheets. With it, a checkpoint
/// carries only the hydration keys its own components declare, which is the
/// "boundary payload locality" contract in DESIGN.md ("Progressive Streaming
/// Hydration — Phase 1").
/// Everything `load_protocol` produces: the compiled protocol, its
/// render-time state, and the component stylesheets the build emitted.
///
/// `css_files` never reaches `dist/` — the client build only emits
/// JavaScript — so the caller must serve it for the `link` and `module` CSS
/// strategies.
struct LoadedApp {
    protocol: Arc<Protocol>,
    state: Value,
    css_files: Vec<(String, String)>,
}

fn load_protocol(app_root: &Path, css: CssStrategy, base_path: &str) -> Result<LoadedApp> {
    let theme_path = resolve_theme_path(THEME, app_root)
        .with_context(|| format!("Failed to resolve theme {THEME}"))?;
    let token_file = load_token_file(&theme_path)
        .with_context(|| format!("Failed to load theme tokens from {}", theme_path.display()))?;

    // `main` already requires `dist/index.js`, so a served build always has
    // the manifest. Tolerate its absence so `cargo check`/unit runs that
    // never execute the client build still succeed.
    let manifest_path = app_root.join("dist").join("webui-projection.json");
    let projection_manifests = if manifest_path.is_file() {
        vec![ProjectionManifestSource::Path(manifest_path)]
    } else {
        Vec::new()
    };

    let build_result = build(BuildOptions {
        app_dir: app_root.join("src"),
        entry: ENTRY.to_string(),
        css,
        plugin: Some(Plugin::WebUI),
        theme: Some(token_file.clone()),
        projection_manifests,
        ..BuildOptions::default()
    })
    .context("Failed to build the streaming example WebUI protocol")?;

    let resolved_tokens = resolve_tokens(&build_result.protocol.tokens, &token_file)
        .context("Failed to resolve design tokens for the loaded theme")?;
    let css_files = build_result.css_files;
    let protocol = Protocol::new(build_result.protocol);

    let (feed_batch_1, feed_batch_2, feed_batch_3) = feed_batches();
    let mut state_map = Map::with_capacity(4);
    state_map.insert("basePath".to_owned(), Value::String(base_path.to_owned()));
    state_map.insert("feedBatch1".to_owned(), Value::Array(feed_batch_1));
    state_map.insert("feedBatch2".to_owned(), Value::Array(feed_batch_2));
    state_map.insert("feedBatch3".to_owned(), Value::Array(feed_batch_3));
    let mut state = Value::Object(state_map);
    inject_into_state(&mut state, &resolved_tokens);

    Ok(LoadedApp {
        protocol: Arc::new(protocol),
        state,
        css_files,
    })
}

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

    let index = jitter.index(FORECASTS.len());
    let (temperature, condition) = FORECASTS[index];

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

#[actix_web::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let css = css_strategy(&args.css)?;
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

    let app = load_protocol(&app_root, css, &args.base_path)?;

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
    insert_generated_css(&mut assets, app.css_files);

    let ctx = web::Data::new(AppCtx {
        protocol: app.protocol,
        state: Arc::new(app.state),
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
    use super::{css_strategy, gap_is_paced, Args, FORECASTS, WEATHER_DELAY_MIN_MS};
    use clap::Parser;
    use webui::CssStrategy;

    #[test]
    fn parses_custom_port() {
        let args = Args::parse_from(["streaming-example-server", "--port", "4001"]);
        assert_eq!(args.port, 4001);
    }

    #[test]
    fn css_strategy_accepts_known_values() {
        assert_eq!(
            css_strategy("style").unwrap_or_else(|e| panic!("style: {e}")),
            CssStrategy::Style
        );
        assert_eq!(
            css_strategy("link").unwrap_or_else(|e| panic!("link: {e}")),
            CssStrategy::Link
        );
        assert_eq!(
            css_strategy("module").unwrap_or_else(|e| panic!("module: {e}")),
            CssStrategy::Module
        );
    }

    #[test]
    fn css_strategy_rejects_unknown_values() {
        assert!(css_strategy("bogus").is_err());
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

    /// The exact predicate `render_page` installs as its pacing schedule, so
    /// the flush-index-to-boundary mapping is asserted rather than implied by
    /// a comment.
    #[test]
    fn only_the_gaps_before_feed_batches_are_paced() {
        // Flush 0 commits the weather shell; the composer must follow it
        // immediately or the highest-priority island is delayed behind a
        // boundary that carries no server data at all.
        assert!(!gap_is_paced(0), "the weather-to-composer gap must be free");
        // Flushes 1, 2 and 3 precede feed batches 1, 2 and 3.
        assert!(gap_is_paced(1));
        assert!(gap_is_paced(2));
        assert!(gap_is_paced(3));
        // The tail checkpoint and terminal record close the response.
        assert!(!gap_is_paced(4), "the response must close promptly");
        assert!(!gap_is_paced(5));
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
