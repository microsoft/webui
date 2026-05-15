// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! HTTP server for the browser-perceived metrics benchmark.
//!
//! Serves a representative SSR HTML page (~50 KB, with CSS and a
//! reasonable element count) via two routes:
//!
//! * `GET /buf?delay_us=N` — buffered render (whole body in one chunk)
//! * `GET /stream?delay_us=N` — streaming render (`StreamingWriter` +
//!   shared `ChunkPool`)
//!
//! `delay_us` injects a per-`write()` artificial sleep on the producer
//! side, simulating slower-rendering pages. Both endpoints serve
//! **identical HTML**; only the delivery mechanism differs. Browser-
//! perceived metrics are then captured via `PerformanceObserver`.

use actix_web::{web, App, HttpResponse, HttpServer};
use anyhow::Result;
use bytes::Bytes;
use clap::Parser;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui_handler::ResponseWriter;

#[derive(Parser, Debug)]
struct Args {
    /// Port to listen on.
    #[arg(long, default_value_t = 3099)]
    port: u16,
}

/// A representative SSR HTML page: ~50 KB, with `<head>` (CSS + meta),
/// a hero `<h1>`, ~200 list items, and a final `<script>`.
fn build_html_template() -> (String, String) {
    let head = r#"<!doctype html><html><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Streaming Bench</title>
<style>
body{font-family:-apple-system,system-ui,sans-serif;margin:0;padding:24px;background:#fafafa}
h1{font-size:48px;margin:0 0 24px;color:#0066cc}
.hero{padding:48px;background:linear-gradient(135deg,#667eea,#764ba2);color:#fff;border-radius:12px;margin-bottom:32px}
.hero p{font-size:20px;margin:8px 0}
ul{list-style:none;padding:0;display:grid;grid-template-columns:repeat(4,1fr);gap:12px}
li{padding:16px;background:#fff;border-radius:8px;box-shadow:0 1px 3px rgba(0,0,0,0.1)}
li h3{margin:0 0 8px;color:#333}
li p{margin:4px 0;color:#666;font-size:14px}
footer{margin-top:48px;padding:24px;text-align:center;color:#999}
</style>
</head><body>
<h1>Streaming Performance Bench</h1>
<div class="hero">
<p>This page is rendered via SSR — the entire HTML you see is produced server-side.</p>
<p>The buffered endpoint sends it as one chunk; the streaming endpoint sends it as ~12 chunks of 4 KB each as soon as they're produced.</p>
<p>Metrics: TTFB, FCP, LCP, domContentLoaded, load.</p>
</div>
<ul>
"#;
    let mut middle = String::with_capacity(40_000);
    for i in 0..200 {
        middle.push_str(&format!(
            r#"<li><h3>Item {i}</h3><p>Description of item number {i}.</p><p>Category: {cat}</p><p>Price: ${price}</p></li>"#,
            cat = ["Books", "Electronics", "Clothing", "Home"][i % 4],
            price = (i + 1) * 10,
        ));
    }
    let tail = r#"
</ul>
<footer>End of bench page. Total ~50 KB.</footer>
</body></html>"#;

    (head.to_string(), format!("{middle}{tail}"))
}

#[derive(Clone)]
struct AppCtx {
    head: Arc<str>,
    body: Arc<str>,
    pool: Arc<ChunkPool>,
}

#[derive(Deserialize)]
struct DelayQuery {
    delay_us: Option<u64>,
}

/// Common writer driver: emit head + body in 64-byte slices to mirror
/// the WebUI handler's slice frequency.
fn drive_writer(w: &mut dyn ResponseWriter, head: &str, body: &str, delay: Duration) {
    for chunk in head.as_bytes().chunks(64) {
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if let Ok(s) = std::str::from_utf8(chunk) {
            let _ = w.write(s);
        }
    }
    for chunk in body.as_bytes().chunks(64) {
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if let Ok(s) = std::str::from_utf8(chunk) {
            let _ = w.write(s);
        }
    }
    let _ = w.end();
}

// ── /buf — buffered ────────────────────────────────────────────────

struct StringWriter {
    buf: String,
}
impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

async fn handle_buf(ctx: web::Data<AppCtx>, query: web::Query<DelayQuery>) -> HttpResponse {
    let delay = Duration::from_micros(query.delay_us.unwrap_or(0));
    let head = Arc::clone(&ctx.head);
    let body = Arc::clone(&ctx.body);
    let html = match actix_web::rt::task::spawn_blocking(move || {
        let mut w = StringWriter {
            buf: String::with_capacity(64 * 1024),
        };
        drive_writer(&mut w, &head, &body, delay);
        w.buf
    })
    .await
    {
        Ok(s) => s,
        Err(_) => return HttpResponse::InternalServerError().body("join failed"),
    };
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .body(html)
}

// ── /stream — streaming + pool ─────────────────────────────────────

async fn handle_stream(ctx: web::Data<AppCtx>, query: web::Query<DelayQuery>) -> HttpResponse {
    let delay = Duration::from_micros(query.delay_us.unwrap_or(0));
    let head = Arc::clone(&ctx.head);
    let body = Arc::clone(&ctx.body);
    let pool = Arc::clone(&ctx.pool);

    let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    actix_web::rt::task::spawn_blocking(move || {
        // Bench writes directly to the streaming writer. Production
        // hosts using the real WebUI handler would pass inject content
        // via `RenderOptions::with_head_inject`/`with_body_inject` —
        // but this bench renders a hand-built HTML template, so no
        // handler-mediated injection is needed.
        let mut writer =
            StreamingWriter::new_pooled(tx, pool).with_flush_timeout(Duration::from_secs(30));
        drive_writer(&mut writer, &head, &body, delay);
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .streaming(stream)
}

// ── Main ───────────────────────────────────────────────────────────

#[actix_web::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let (head, body) = build_html_template();
    let ctx = AppCtx {
        head: Arc::from(head),
        body: Arc::from(body),
        pool: Arc::new(ChunkPool::new(256, StreamingWriter::CHUNK_TARGET + 1024)),
    };
    let data = web::Data::new(ctx);
    let port = args.port;
    println!("streaming-browser-bench-server listening on http://127.0.0.1:{port}");
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .route("/buf", web::get().to(handle_buf))
            .route("/stream", web::get().to(handle_stream))
    })
    .bind(("127.0.0.1", port))?
    .workers(2)
    .run()
    .await?;
    Ok(())
}
