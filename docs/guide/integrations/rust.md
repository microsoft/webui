# WebUI Rust Handler

The `webui` crate provides high-performance build and rendering of WebUI protocols in Rust. It streams rendered HTML fragments via the `ResponseWriter` trait for progressive rendering with zero unnecessary allocations.

## Installation

```toml
[dependencies]
microsoft-webui = "*" # see https://crates.io/crates/microsoft-webui for latest version
serde_json = "1"
```

The crate is published as `microsoft-webui` on crates.io; the bare `webui` name is owned by an unrelated project. Cargo's default rename rules mean items remain importable as `use webui::...` because the crate sets `[lib] name = "webui"` internally.

## Examples

<webui-tabs>
<webui-tab slot="tab" active>Actix Web</webui-tab>
<webui-tab slot="tab">Axum</webui-tab>
<webui-tab slot="tab">Hyper</webui-tab>
<webui-tab-panel active>

```rust
use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse};
use webui::{WebUIHandler, RenderOptions, ResponseWriter, WebUIProtocol};
use serde_json::json;
use std::fs;

struct StringWriter(String);

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui::HandlerResult<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui::HandlerResult<()> { Ok(()) }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let protocol_bytes = fs::read("./dist/protocol.bin").unwrap();
    let protocol = WebUIProtocol::from_protobuf(&protocol_bytes).unwrap();
    let protocol = web::Data::new(protocol);

    HttpServer::new(move || {
        App::new()
            .app_data(protocol.clone())
            .route("/{path:.*}", web::get().to(|proto: web::Data<WebUIProtocol>, req: HttpRequest| async move {
                let state = json!({ "title": "Home" });
                let mut writer = StringWriter(String::new());
                let mut handler = WebUIHandler::new();
                let options = RenderOptions::new("index.html", req.path());
                handler.handle(&proto, &state, &options, &mut writer).unwrap();
                HttpResponse::Ok().content_type("text/html").body(writer.0)
            }))
    })
    .bind("127.0.0.1:3000")?
    .run()
    .await
}
```

</webui-tab-panel>
<webui-tab-panel>

```rust
use axum::{routing::get, Router, extract::{State, Request}};
use webui::{WebUIHandler, RenderOptions, ResponseWriter, WebUIProtocol};
use serde_json::json;
use std::{fs, sync::Arc};

struct StringWriter(String);

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui::HandlerResult<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui::HandlerResult<()> { Ok(()) }
}

#[tokio::main]
async fn main() {
    let protocol_bytes = fs::read("./dist/protocol.bin").unwrap();
    let protocol = Arc::new(WebUIProtocol::from_protobuf(&protocol_bytes).unwrap());

    let app = Router::new()
        .route("/{*path}", get(|State(proto): State<Arc<WebUIProtocol>>, req: Request| async move {
            let state = json!({ "title": "Home" });
            let mut writer = StringWriter(String::new());
            let mut handler = WebUIHandler::new();
            let options = RenderOptions::new("index.html", req.uri().path());
            handler.handle(&proto, &state, &options, &mut writer).unwrap();
            axum::response::Html(writer.0)
        }))
        .with_state(protocol);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

</webui-tab-panel>
<webui-tab-panel>

```rust
use hyper::{server::conn::http1, service::service_fn, body::Bytes, Request, Response};
use hyper_util::rt::TokioIo;
use http_body_util::Full;
use webui::{WebUIHandler, RenderOptions, ResponseWriter, WebUIProtocol};
use serde_json::json;
use std::{fs, sync::Arc};

struct StringWriter(String);

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui::HandlerResult<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui::HandlerResult<()> { Ok(()) }
}

#[tokio::main]
async fn main() {
    let protocol_bytes = fs::read("./dist/protocol.bin").unwrap();
    let protocol = Arc::new(WebUIProtocol::from_protobuf(&protocol_bytes).unwrap());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let proto = protocol.clone();
        tokio::spawn(async move {
            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service_fn(move |req: Request<_>| {
                    let proto = proto.clone();
                    async move {
                        let state = json!({ "title": "Home" });
                        let mut writer = StringWriter(String::new());
                        let mut handler = WebUIHandler::new();
                        let options = RenderOptions::new("index.html", req.uri().path());
                        handler.handle(&proto, &state, &options, &mut writer).unwrap();
                        Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from(writer.0))))
                    }
                }))
                .await
                .ok();
        });
    }
}
```

</webui-tab-panel>
</webui-tabs>

## Streaming SSR

For production, prefer the framework-provided `webui::streaming::StreamingWriter` over a hand-rolled `String` buffer. It coalesces small writes into ~4 KB chunks, ships them over a **bounded** `tokio::mpsc` channel (backpressure on slow clients), and recycles chunk buffers through a shared `ChunkPool` so steady-state RPS does zero per-flush allocation.

```rust
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{WebUIHandler, RenderOptions, ResponseWriter};

// One shared pool per server (constructed at startup, lives forever).
let chunk_pool = Arc::new(ChunkPool::new(
    256,                                       // ~1.25 MiB peak pool memory
    StreamingWriter::CHUNK_TARGET + 1024,
));

// Per request:
let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
actix_web::rt::task::spawn_blocking({
    let chunk_pool = Arc::clone(&chunk_pool);
    move || {
        // `with_flush_timeout` bounds the slow-loris DoS surface to
        // `30s × concurrent_renders`. `end()` returns the typed error
        // from the final flush — log truncated streams at debug.
        let mut writer = StreamingWriter::new_pooled(tx, chunk_pool)
            .with_flush_timeout(Duration::from_secs(30));
        let options = RenderOptions::new("index.html", &request_path)
            .with_nonce(&csp_nonce)
            .with_body_inject(&livereload_script); // per-request inject
        if let Err(e) = handler.handle(&proto, &state, &options, &mut writer) {
            log::error!("render failed: {e}");
        }
        if let Err(e) = ResponseWriter::end(&mut writer) {
            log::debug!("stream truncated: {e}");
        }
    }
});
HttpResponse::Ok()
    .content_type("text/html; charset=utf-8")
    .streaming(tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, actix_web::Error>))
```

### Per-request HTML injection

`with_head_inject` / `with_body_inject` splice host-provided HTML at the parser-synthesized `head_end` / `body_end` structural boundaries — zero scan cost, and cannot mis-fire on `</head>` / `</body>` literals appearing inside HTML comments, `<iframe srcdoc>`, or inline `<script>`. Typical uses: per-request `<link rel="preload">` hints, dev livereload script, OpenTelemetry trace IDs.

> **Safety:** the HTML is written verbatim, no escaping. Untrusted input is a direct XSS vector. Pre-escape with `webui_handler::encode_safe` (re-exported for this purpose) if your content path may include user data.

### Typed streaming errors

`StreamingWriter` returns `HandlerError::ClientDisconnected` (receiver dropped) or `HandlerError::StreamTimeout` (flush deadline exceeded) from both `write()` and `end()`, so callers can distinguish "fully delivered" from "client cancelled" for correct telemetry.

## API Reference

### Build

| Function | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `BuildResult` |
| `build_to_disk(options, out_dir)` | Build and write `protocol.bin` + CSS files to disk |
| `inspect(path)` | Read a protocol file and return JSON |
| `inspect_bytes(bytes)` | Convert protocol bytes to JSON |

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `app_dir` | `PathBuf` | - | Path to app folder |
| `entry` | `String` | `"index.html"` | Entry file |
| `css` | `CssStrategy` | `Link` | CSS delivery: `Link`, `Style`, or `Module` |
| `plugin` | `Option<String>` | `None` | Parser plugin name (see [Plugins](/guide/concepts/plugins/) for the available identifiers) |
| `components` | `Vec<String>` | `[]` | External component sources |

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `duration` | `Duration` | Build time |
| `fragment_count` | `usize` | Total fragments |
| `component_count` | `usize` | Components registered |
| `css_file_count` | `usize` | CSS files produced |
| `protocol_size_bytes` | `usize` | Protocol binary size |
| `token_count` | `usize` | CSS tokens discovered |

### RenderOptions

| Field / builder | Type | Description |
|---|---|---|
| `RenderOptions::new(entry_id, request_path)` | constructor | Entry fragment + route-matching path |
| `with_nonce(&str)` | builder | CSP nonce reflected onto inline `<script>` / `<style type="module">`. Empty string normalises to `None`. |
| `with_head_inject(&str)` | builder | Raw HTML emitted immediately before `</head>` at the parser's structural boundary (see [Streaming SSR](#streaming-ssr)). |
| `with_body_inject(&str)` | builder | Raw HTML emitted immediately before `</body>`. Same structural-boundary contract. |

### HandlerError variants

| Variant | When |
|---|---|
| `ClientDisconnected` | Streaming receiver dropped; caller should abort the render. |
| `StreamTimeout` | `with_flush_timeout` deadline exceeded; ops should alert on slow-loris patterns. |
| `MissingFragment(String)` | `entry_id` not found in the protocol. |
| `TypeError(String)` / `Evaluation(String)` | Template/expression runtime errors. |

## Thread safety

`WebUIHandler` is `Send + Sync`. The handler is stateless: per-render state lives in a local context created inside `handle`, and the only stored field is a plugin factory function pointer. Construct one handler at startup, wrap it in an [`Arc`], and call `handler.handle(...)` from any request task without locking.

### Sharing a handler across tasks

The realistic pattern is "construct once, clone into many tasks":

```rust
use std::sync::Arc;
use webui::{RenderOptions, WebUIHandler, WebUIProtocol};
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let protocol = Arc::new(WebUIProtocol::from_protobuf_file("dist/protocol.bin")?);
    let handler = Arc::new(WebUIHandler::with_plugin(|| {
        Box::new(FastV3HydrationPlugin::new())
    }));

    while let Some(request) = accept_request().await {
        let handler = Arc::clone(&handler);
        let protocol = Arc::clone(&protocol);
        tokio::spawn(async move {
            let options = RenderOptions::new("index.html", &request.path);
            let mut writer = request.into_writer();
            if let Err(error) = handler.handle(&protocol, &request.state, &options, &mut writer) {
                tracing::error!(?error, "render failed");
            }
        });
    }
    Ok(())
}
```

The same shape applies to other async runtimes (`actix_web::rt::spawn`, `smol::spawn`, etc.) and to thread pools (`std::thread::spawn` with a `move` closure cloning the `Arc`). Because `handle` takes `&self`, no `Mutex` is needed; concurrent renders run in parallel.

[`Arc`]: https://doc.rust-lang.org/std/sync/struct.Arc.html
