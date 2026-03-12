//! SSR Performance Test — WebUI Framework
//!
//! Rust server that loads a pre-built protocol.bin, computes a spiral
//! pattern of tiles per-request, and renders them through the WebUI
//! handler — comparable to the `fastify-html` entry in the
//! ssr-performance-showdown benchmark.
//!
//! Prerequisites:
//!   cargo run -p webui-cli -- build app --out dist
//!
//! Usage:
//!   cargo run                     # listen on :3000
//!   cargo run -- --port 3001      # custom port

use actix_web::{web, App, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use std::path::Path;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::WebUIProtocol;

// ── Spiral parameters (match ssr-performance-showdown) ──────────────────

const WRAPPER_WIDTH: f64 = 960.0;
const WRAPPER_HEIGHT: f64 = 720.0;
const CELL_SIZE: f64 = 10.0;
const ANGLE_STEP: f64 = 0.2;
const RADIUS_STEP: f64 = CELL_SIZE * 0.015;

// ── In-memory response writer ───────────────────────────────────────────

struct MemoryWriter {
    buf: String,
}

impl MemoryWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }
}

impl ResponseWriter for MemoryWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

// ── Spiral tile computation (runs per-request) ──────────────────────────

fn compute_tiles() -> Vec<serde_json::Value> {
    let center_x = WRAPPER_WIDTH / 2.0;
    let center_y = WRAPPER_HEIGHT / 2.0;
    let max_radius = WRAPPER_WIDTH.min(WRAPPER_HEIGHT) / 2.0;

    let mut angle: f64 = 0.0;
    let mut radius: f64 = 0.0;
    let mut tiles = Vec::with_capacity(2400);

    while radius < max_radius {
        let x = center_x + angle.cos() * radius;
        let y = center_y + angle.sin() * radius;

        if (0.0..=WRAPPER_WIDTH - CELL_SIZE).contains(&x)
            && (0.0..=WRAPPER_HEIGHT - CELL_SIZE).contains(&y)
        {
            tiles.push(serde_json::Value::Object({
                let mut m = serde_json::Map::with_capacity(2);
                m.insert(
                    "left".to_owned(),
                    serde_json::Value::String(format!("{x:.2}px")),
                );
                m.insert(
                    "top".to_owned(),
                    serde_json::Value::String(format!("{y:.2}px")),
                );
                m
            }));
        }

        angle += ANGLE_STEP;
        radius += RADIUS_STEP;
    }

    tiles
}

// ── Route handler ───────────────────────────────────────────────────────

async fn handle_index(protocol: web::Data<WebUIProtocol>) -> HttpResponse {
    let tiles_value = serde_json::Value::Array(compute_tiles());
    let mut state_map = serde_json::Map::with_capacity(1);
    state_map.insert("tiles".to_owned(), tiles_value);
    let state = serde_json::Value::Object(state_map);

    let mut writer = MemoryWriter::with_capacity(256 * 1024);
    let mut handler = WebUIHandler::new();

    if let Err(e) = handler.handle(
        &protocol,
        &state,
        &RenderOptions::new("index.html", "/"),
        &mut writer,
    ) {
        return HttpResponse::InternalServerError().body(format!("Render error: {e}"));
    }

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(writer.buf)
}

// ── Startup ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let port: u16 = std::env::args()
        .position(|a| a == "--port")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);

    let protocol_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/protocol.bin");
    let protocol = WebUIProtocol::from_protobuf_file(&protocol_path)
        .with_context(|| format!("Failed to load {}", protocol_path.display()))?;
    let protocol_data = web::Data::new(protocol);

    println!("Listening on http://localhost:{port}");

    actix_web::rt::System::new()
        .block_on(async move {
            HttpServer::new(move || {
                App::new()
                    .app_data(protocol_data.clone())
                    .route("/", web::get().to(handle_index))
                    .route("/index.html", web::get().to(handle_index))
                    .default_service(
                        web::route().to(|| async { HttpResponse::NotFound().body("Not Found") }),
                    )
            })
            .bind(format!("0.0.0.0:{port}"))
            .with_context(|| format!("Failed to bind to port {port}"))?
            .run()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
        })
        .context("Server error")?;

    Ok(())
}
