// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{http::Method, web, App, HttpRequest, HttpResponse, HttpServer};
use std::io;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

struct Runtime {
    protocol: Protocol,
    handler: WebUIHandler,
    assets: PathBuf,
}

struct Html(String);

impl ResponseWriter for Html {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn text(status: actix_web::http::StatusCode, message: &'static str) -> HttpResponse {
    HttpResponse::build(status)
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .content_type("text/plain; charset=utf-8")
        .body(message)
}

fn mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "js" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "txt" => Some("text/plain; charset=utf-8"),
        "map" => Some("application/json"),
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "ico" => Some("image/x-icon"),
        "woff2" => Some("font/woff2"),
        _ => None,
    }
}

async fn asset(runtime: &Runtime, pathname: &str) -> io::Result<Option<HttpResponse>> {
    let Some(relative) = pathname.strip_prefix("/assets/") else {
        return Ok(None);
    };
    let mut file = runtime.assets.clone();
    for part in relative.split('/') {
        if part.is_empty() || part.starts_with('.') || part.contains(['\\', '\0']) {
            return Ok(None);
        }
        file.push(part);
        if tokio::fs::symlink_metadata(&file)
            .await?
            .file_type()
            .is_symlink()
        {
            return Ok(None);
        }
    }
    let Some(content_type) = mime(&file) else {
        return Ok(None);
    };
    let canonical = tokio::fs::canonicalize(&file).await?;
    if !canonical.starts_with(&runtime.assets) {
        return Ok(None);
    }
    let file = tokio::fs::File::open(canonical).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let body = futures_util::stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0_u8; 16 * 1024];
        let length = file.read(&mut buffer).await?;
        if length == 0 {
            return Ok::<_, io::Error>(None);
        }
        buffer.truncate(length);
        Ok(Some((web::Bytes::from(buffer), file)))
    });
    Ok(Some(
        HttpResponse::Ok()
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .insert_header(("Cache-Control", "no-cache"))
            .insert_header(("Content-Length", metadata.len()))
            .content_type(content_type)
            .streaming(body),
    ))
}

async fn request(request: HttpRequest, runtime: web::Data<Runtime>) -> HttpResponse {
    use actix_web::http::StatusCode;
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return HttpResponse::MethodNotAllowed()
            .insert_header(("Allow", "GET, HEAD"))
            .finish();
    }
    let Ok(pathname) = percent_encoding::percent_decode_str(request.path()).decode_utf8() else {
        return text(StatusCode::NOT_FOUND, "Not found");
    };
    if pathname.contains(['\\', '\0']) || pathname.split('/').any(|p| p == "." || p == "..") {
        return text(StatusCode::NOT_FOUND, "Not found");
    }
    // Use the same validated literal route for state and rendering. The generic
    // matcher expects encoded input for dynamic segments, not these fixed pages.
    let route = pathname.as_ref();
    let state = match route {
        "/" => Some(serde_json::json!({"title":"Hello WebUI","count":0,"isHome":true})),
        "/about" => Some(serde_json::json!({"title":"About WebUI","isHome":false})),
        _ => None,
    };
    if let Some(state) = state {
        let mut html = Html(String::with_capacity(8192));
        match runtime.handler.render(
            &runtime.protocol,
            &state,
            &RenderOptions::new("index.html", route),
            &mut html,
        ) {
            Ok(()) => {
                return HttpResponse::Ok()
                    .insert_header(("X-Content-Type-Options", "nosniff"))
                    .insert_header(("Cache-Control", "no-store"))
                    .content_type("text/html; charset=utf-8")
                    .body(html.0);
            }
            Err(error) => eprintln!("Rendering failed: {error}"),
        }
        return text(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    match asset(&runtime, &pathname).await {
        Ok(Some(response)) => response,
        Ok(None) => text(StatusCode::NOT_FOUND, "Not found"),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::InvalidFilename
            ) =>
        {
            text(StatusCode::NOT_FOUND, "Not found")
        }
        Err(error) => {
            eprintln!("Asset request failed: {error}");
            text(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    let protocol =
        Protocol::from_protobuf(&std::fs::read("dist/protocol.bin")?).map_err(io::Error::other)?;
    for path in ["dist", "dist/public"] {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(io::Error::other(
                "dist and dist/public must not be symlinks",
            ));
        }
    }
    let runtime = web::Data::new(Runtime {
        protocol,
        handler: WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
        assets: std::fs::canonicalize("dist/public")?,
    });
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse::<u16>()
        .map_err(io::Error::other)?;
    if port == 0 {
        return Err(io::Error::other("PORT must be from 1 to 65535"));
    }
    let server = HttpServer::new(move || {
        App::new()
            .app_data(runtime.clone())
            .default_service(web::to(request))
    })
    .bind((host.as_str(), port))?
    .shutdown_timeout(5)
    .run();
    if std::env::var("_WEBUI_APP_CONTROL").as_deref() == Ok("stdin-v1") {
        let handle = server.handle();
        let runtime = tokio::runtime::Handle::current();
        std::thread::Builder::new()
            .name("webui-supervisor".into())
            .spawn(move || {
                let mut command = [0_u8; 1];
                match std::io::Read::read_exact(&mut io::stdin(), &mut command) {
                    Ok(()) if command == *b"S" => {}
                    Ok(()) => eprintln!("Invalid WebUI supervisor command; shutting down"),
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                        eprintln!("WebUI supervisor disconnected; shutting down")
                    }
                    Err(error) => {
                        eprintln!("WebUI supervisor control failed: {error}; shutting down")
                    }
                }
                let _ = runtime.spawn(async move {
                    handle.stop(true).await;
                });
            })?;
    }
    println!("Listening on http://{host}:{port}");
    server.await
}
