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

<webui-press-tabs>
<webui-press-tab slot="tab" active>Actix Web</webui-press-tab>
<webui-press-tab slot="tab">Axum</webui-press-tab>
<webui-press-tab slot="tab">Hyper</webui-press-tab>
<webui-press-tab-panel active>

```rust
use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse};
use webui::{Protocol, WebUIHandler, RenderOptions, ResponseWriter};
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
    let protocol = Protocol::from_protobuf(&protocol_bytes).unwrap();
    let protocol = web::Data::new(protocol);

    HttpServer::new(move || {
        App::new()
            .app_data(protocol.clone())
            .route("/{path:.*}", web::get().to(|proto: web::Data<Protocol>, req: HttpRequest| async move {
                let state = json!({ "title": "Home" });
                let mut writer = StringWriter(String::new());
                let handler = WebUIHandler::new();
                let options = RenderOptions::new("index.html", req.path());
                handler.render(proto.get_ref(), &state, &options, &mut writer).unwrap();
                HttpResponse::Ok().content_type("text/html").body(writer.0)
            }))
    })
    .bind("127.0.0.1:3000")?
    .run()
    .await
}
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```rust
use axum::{routing::get, Router, extract::{State, Request}};
use webui::{Protocol, WebUIHandler, RenderOptions, ResponseWriter};
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
    let protocol = Arc::new(Protocol::from_protobuf(&protocol_bytes).unwrap());

    let app = Router::new()
        .route("/{*path}", get(|State(proto): State<Arc<Protocol>>, req: Request| async move {
            let state = json!({ "title": "Home" });
            let mut writer = StringWriter(String::new());
            let handler = WebUIHandler::new();
            let options = RenderOptions::new("index.html", req.uri().path());
            handler.render(proto.as_ref(), &state, &options, &mut writer).unwrap();
            axum::response::Html(writer.0)
        }))
        .with_state(protocol);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```rust
use hyper::{server::conn::http1, service::service_fn, body::Bytes, Request, Response};
use hyper_util::rt::TokioIo;
use http_body_util::Full;
use webui::{Protocol, WebUIHandler, RenderOptions, ResponseWriter};
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
    let protocol = Arc::new(Protocol::from_protobuf(&protocol_bytes).unwrap());

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
                        let handler = WebUIHandler::new();
                        let options = RenderOptions::new("index.html", req.uri().path());
                        handler.render(proto.as_ref(), &state, &options, &mut writer).unwrap();
                        Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from(writer.0))))
                    }
                }))
                .await
                .ok();
        });
    }
}
```

</webui-press-tab-panel>
</webui-press-tabs>

## Streaming SSR

`webui::streaming::StreamingWriter` coalesces small writes, sends them over a
bounded `tokio::mpsc` channel for backpressure, and can recycle buffers through
a shared `ChunkPool`. You can use it with `WebUIHandler::render` for transport
streaming. To make authored `<boundary>` checkpoints hydrate before the
response completes, call the opt-in `WebUIHandler::render_streaming` API shown
below. It commits every boundary as final. Use `stream_response` when backend
readiness controls checkpoint timing or an island needs later server state.

```rust
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::{mpsc, Semaphore};
use tokio_stream::StreamExt;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{WebUIHandler, RenderOptions, ResponseWriter};

// One shared pool per server (constructed at startup, lives forever).
let chunk_pool = Arc::new(ChunkPool::new(
    256,                                       // ~1.25 MiB peak pool memory
    StreamingWriter::CHUNK_TARGET + 1024,
));
let render_permits = Arc::new(Semaphore::new(4));

// Per request:
// Acquire before `spawn_blocking`; its internal queue is not an admission limit.
let render_permit = match Arc::clone(&render_permits).try_acquire_owned() {
    Ok(permit) => permit,
    Err(_) => {
        return HttpResponse::ServiceUnavailable()
            .insert_header(("Retry-After", "1"))
            .body("streaming render capacity is temporarily exhausted");
    }
};
let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
actix_web::rt::task::spawn_blocking({
    let chunk_pool = Arc::clone(&chunk_pool);
    move || {
        let _render_permit = render_permit;
        // `with_flush_timeout` bounds the slow-loris DoS surface to
        // `30s × concurrent_renders`. `end()` returns the typed error
        // from the final flush. Log truncated streams at debug.
        let mut writer = StreamingWriter::new_pooled(tx, chunk_pool)
            .with_flush_timeout(Duration::from_secs(30));
        let options = RenderOptions::new("index.html", &request_path)
            .with_nonce(&csp_nonce)
            .with_body_inject(&livereload_script); // per-request inject
        if let Err(e) = handler.render_streaming(&proto, &state, &options, &mut writer) {
            log::error!("render failed: {e}");
            if let Err(flush_error) = ResponseWriter::end(&mut writer) {
                log::debug!("stream truncated: {flush_error}");
            }
        }
    }
});
HttpResponse::Ok()
    .content_type("text/html; charset=utf-8")
    .streaming(tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, actix_web::Error>))
```

### Host-driven boundaries and state updates

`stream_response` returns a synchronous response session. Resolve authored names
once, then use integer handles for every write:

```rust
use webui::{BoundaryMode, RenderOptions, WebUIHandler};

let options = RenderOptions::new("index.html", "/");
let mut response = handler.stream_response(&protocol, &options, &mut writer)?;
let weather = response.boundary("weather-shell")?;
let composer = response.boundary("composer-ready")?;
let feed_1 = response.boundary("feed-batch-1")?;

response.write_shell(&page_state)?;
response.write_boundary(weather, &loading_weather, BoundaryMode::Updatable)?;
response.write_boundary(composer, &composer_state, BoundaryMode::Final)?;

// Await or receive backend work between these synchronous calls.
response.update(weather, &ready_weather)?;
response.write_boundary(feed_1, &feed_state, BoundaryMode::Final)?;
response.finish(&tail_state)?;
```

Boundary HTML must be written once in declaration order. `update` can be called
between any two boundary writes, but only for a committed `Updatable` boundary.
It applies the same compiled state projection as that boundary's initial
checkpoint, requires a JSON object, emits no marker range, and flushes
immediately. `finish` requires all compiled boundaries to be committed.

The session borrows each state value only for its call. It does not await,
allocate a task, or synchronize concurrent callers. An async server should use a
bounded command channel and one admitted blocking worker that owns the session
and `StreamingWriter`; `examples/app/streaming` is the reference implementation.

The public writer contract is:

```rust
pub trait FlushWriter: ResponseWriter {
    fn flush(&mut self) -> HandlerResult<()>;
}
```

`render_streaming` and `stream_response` accept a `FlushWriter`;
`StreamingWriter` implements that trait. Each explicit boundary is completed,
followed by its hydration checkpoint and a semantic flush. At `body_end`, any
native or scriptless tail HTML is followed by one empty markerless terminal
record and one final flush. The terminal record never repeats state or template
metadata. The normal `render` method still accepts any `ResponseWriter` and does
not progressively hydrate authored boundaries.

The entry template must load its application module with an early
`<script type="module" async>` in `<head>`, before boundary content. See
[Progressive Streaming Hydration](/guide/concepts/hydration#progressive-streaming-hydration)
and the
[`<boundary>` directive](/guide/concepts/directives/boundary) for the
authoring and lifecycle contract. That application entry must import
`@microsoft/webui-framework/streaming.js` before component registration
modules. The default framework entry does not include the streaming coordinator.

Each checkpoint carries state and templates for the component surface reachable
from roots rendered since the previous checkpoint, including descendants behind
initially false conditions or empty repeats. Unrelated later boundaries remain
excluded. Template metadata is sent only when first reachable, inventory still
tracks only rendered SSR roots, and repeated instances receive checkpoint-local
state without duplicate metadata. The final terminal envelope is always
`[1,nextSequence,3,0,{}]`; its flush also commits preceding static tail bytes.

Host-driven sessions are currently Rust-only. Node and WASM `renderStream`
callbacks are synchronous whole-render APIs without writable backpressure, and
the C ABI returns buffered strings. They do not expose this session contract.

The bounded channel limits bytes retained by a running render, but it does not
bound how many requests can queue in Tokio's blocking pool. Acquire a
process-wide permit with `try_acquire_owned()` before `spawn_blocking`, return
HTTP 503 with `Retry-After` when saturated, and move the permit into the closure
so it is held for the render's full lifetime.

`FlushWriter::flush` means all currently buffered bytes were handed to the HTTP
transport. It cannot force an HTTP adapter, compressor, reverse proxy, or CDN to
deliver them immediately. Disable response buffering where applicable and test
the production delivery path. Checkpoints are strictly in document
order.

### Per-request HTML injection

`with_head_inject` / `with_body_inject` splice host-provided HTML at the
parser-synthesized `head_end` / `body_end` structural boundaries. They cannot
mis-fire on `</head>` / `</body>` literals appearing inside HTML comments,
`<iframe srcdoc>`, or inline `<script>`. Typical uses include per-request
`<link rel="preload">` hints, a development livereload script, and OpenTelemetry
trace IDs.

> **Safety:** the HTML is written verbatim, no escaping. Untrusted input is a direct XSS vector. Pre-escape with `webui_handler::encode_safe` (re-exported for this purpose) if your content path may include user data.

### Reserved `$webui` state channel

A reserved top-level `"$webui"` object in the render state carries the same
boundary HTML without a Rust-only builder, so non-Rust hosts get the capability
through the state JSON they already send:

```json
{
  "$webui": {
    "headEnd": "<link rel=\"preload\" as=\"image\" href=\"/hero.avif\">",
    "bodyStart": "<!-- after <body> -->",
    "bodyEnd": "<script src=\"/livereload.js\"></script>"
  }
}
```

Every member is optional and must be a string; anything else is ignored rather
than an error. The key is stripped from the hydration payload, so it never
reaches the client. Values are emitted after `with_head_inject` /
`with_body_inject` at the same boundary.

> **Safety:** values are written verbatim with no escaping, exactly like `with_head_inject`. Never let request-derived data reach `$webui`.

### Typed streaming errors

`StreamingWriter` returns `HandlerError::ClientDisconnected` (receiver dropped)
or `HandlerError::StreamTimeout` (flush deadline exceeded) from writes,
boundary flushes, and the final `end()`, so callers can distinguish a completed
delivery from a cancelled or stalled stream.

## API Reference

### Build

| Function | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `BuildResult` |
| `build_to_disk(options, out_dir)` | Build and write `protocol.bin`, CSS files, and static component assets to disk |
| `inspect(path)` | Read a protocol file and return JSON |
| `inspect_bytes(bytes)` | Convert protocol bytes to JSON |

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `app_dir` | `PathBuf` | - | Path to app folder |
| `entry` | `String` | `"index.html"` | Entry file |
| `css` | `CssStrategy` | `Link` | CSS delivery: `Link`, `Style`, or `Module` |
| `plugin` | `Option<Plugin>` | `None` | Parser plugin (see [Plugins](/guide/concepts/plugins/) for the available identifiers) |
| `components` | `Vec<String>` | `[]` | External component sources |
| `component_asset_roots` | `Vec<String>` | `[]` | Root component tags emitted as static `.webui.js` ESM assets |
| `metafile` | `bool` | `false` | Generate an esbuild-compatible component asset graph in the build result |
| `projection_manifests` | `Vec<ProjectionManifestSource>` | `[]` | Disk, inline, or prepared projection fragments; empty preserves full state |
| `css_file_name_template` | `String` | `"[name].[ext]"` | Emitted asset filename template for Link-mode CSS and component assets. Tokens: `[name]`, `[hash]`, `[ext]` |
| `css_public_base` | `Option<String>` | `None` | Public URL/path prefix for Link-mode CSS hrefs |
| `theme` | `Option<TokenFile>` | `None` | Loaded design-token theme used to validate unresolved CSS tokens during build |

`BuildResult::component_asset_files` contains root and shared chunk modules.
Entry-reachable dependencies remain in the protocol and are external
prerequisites; single-root dependencies stay inline; dependencies with an
identical multi-root consumer set are emitted once in a shared chunk. Asset-only
protocol records are pruned after the files are rendered. Component assets
cannot be combined with `<route>`.

Set `metafile: true` to populate
`BuildResult::metafile` with esbuild-compatible JSON:

```rust
let result = webui::build(BuildOptions {
    app_dir: "src".into(),
    plugin: Some(Plugin::WebUI),
    component_asset_roots: vec!["settings-dialog".into(), "mail-thread".into()],
    metafile: true,
    ..BuildOptions::default()
})?;

if let Some(metafile) = result.metafile {
    std::fs::write("dist/component-assets-meta.json", metafile)?;
}
```

Root outputs in the metafile have `entryPoint` records and
`dynamic-import` edges to shared chunks. `build_to_disk()` validates
`protocol.bin`, CSS files, and component assets as one output set before
writing, so filename collisions fail without leaving partial output.

Load themes with `webui::resolve_theme_path()` and `webui::load_token_file()`.
When `theme` is set, missing required CSS tokens fail as parser diagnostics
before the protocol is returned. Tokens used only with a literal `var()`
fallback (e.g. `var(--brand, #000)`) are exempt; if such a token is also absent
from every theme it is reported as a non-fatal advisory in
`BuildResult::warnings` (a likely typo) instead of failing the build.

Use `ProjectionManifestSource::Path` for normal builds. Orchestrators that build
many protocols against one client bundle can call
`prepare_projection_manifests()` once and reuse
`ProjectionManifestSource::Prepared`. Every non-empty source set is
hash-validated, merged by tag, and required to cover every compiled scripted
component.

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
| `with_nonce(&str)` | builder | CSP nonce reflected onto inline `<script>` tags (including the `<script type="importmap">` tags that register Module-strategy CSS). Empty string normalises to `None`. |
| `with_head_inject(&str)` | builder | Raw HTML emitted immediately before `</head>` at the parser's structural boundary (see [Streaming SSR](#streaming-ssr)). |
| `with_body_inject(&str)` | builder | Raw HTML emitted immediately before `</body>`. Same structural-boundary contract. |

### Host-driven streaming

| API | Description |
|---|---|
| `WebUIHandler::stream_response(protocol, options, writer)` | Starts one progressive response session |
| `StreamingResponse::boundary(name)` | Resolves a compiled name once to `BoundaryId` |
| `StreamingResponse::boundary_count()` | Returns the fixed compile-time boundary count |
| `StreamingResponse::write_shell(state)` | Renders and flushes the document prefix |
| `StreamingResponse::write_boundary(id, state, mode)` | Commits the next authored boundary as `Final` or `Updatable` |
| `StreamingResponse::update(id, state)` | Sends projected object state to a committed updatable boundary |
| `StreamingResponse::finish(state)` | Renders the tail, emits terminal, and ends the writer |

`StreamingResponse` borrows a `ResponseWriter` for the life of the response,
which is the cheapest shape when the transport lives in the same process and the
same language.

When you would rather own the bytes — for example to feed a channel, a test
harness, or a transport whose writer cannot be borrowed for that long —
`StreamingSession` offers the same six operations and returns a `Vec<u8>` per
call instead:

```rust
let mut session = StreamingSession::new(
    Arc::clone(&handler),
    Arc::clone(&protocol),
    SessionOptions::new("index.html", "/"),
)?;
let rows = session.boundary("rows")?;

sink.send(session.write_shell(&shell_state)?)?;
sink.send(session.write_boundary(rows, &rows_state, BoundaryMode::Final)?)?;
sink.send(session.finish(&tail_state)?)?;
```

The session holds its own `Arc` clones, so it may outlive the bindings you
created it from.

This is the same type Node, WASM, C, and C# drive, so behaviour is identical
across hosts. Prefer `stream_response` in Rust servers: it writes straight into
the writer and avoids the per-chunk buffer.

### HandlerError variants

| Variant | When |
|---|---|
| `ClientDisconnected` | Streaming receiver dropped; caller should abort the render. |
| `StreamTimeout` | `with_flush_timeout` deadline exceeded; ops should alert on slow-loris patterns. |
| `MissingFragment(String)` | `entry_id` not found in the protocol. |
| `TypeError(String)` / `Evaluation(String)` | Template/expression runtime errors. |
