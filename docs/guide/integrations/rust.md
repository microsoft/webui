# WebUI Rust Handler

The `webui` crate provides high-performance build and rendering of WebUI protocols in Rust. It streams rendered HTML fragments via the `ResponseWriter` trait for progressive rendering with zero unnecessary allocations.

## Installation

```toml
[dependencies]
webui = "*" # see https://crates.io/crates/webui for latest version
serde_json = "1"
```

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
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> { Ok(()) }
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
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> { Ok(()) }
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
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> { Ok(()) }
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
