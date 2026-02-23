use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use http_body_util::Full;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;

#[path = "../../../../examples/shared/rust/config.rs"]
mod config;
#[path = "../../../../examples/shared/rust/output.rs"]
mod output;
mod render;
mod routes {
    pub mod assets;
    pub mod hmr;
    pub mod index;
    pub mod not_found;
}
mod watcher;

use crate::config::AppPaths;
use crate::output::Printer;
use crate::render::render_to_index_html;
use crate::watcher::start_file_watcher;

#[derive(Parser)]
#[command(
    name = "webui-hyper",
    about = "Serve a WebUI app with hyper (performance-focused)"
)]
struct Cli {
    /// App name inside examples/app/ (defaults to hello-world)
    #[arg(long, default_value = "hello-world")]
    app: String,
}

fn main() {
    let cli = Cli::parse();

    let result = run(&cli);
    if result.is_err() {
        std::process::exit(1);
    }
}

#[tokio::main]
async fn run(cli: &Cli) -> Result<()> {
    let printer = Printer::new();
    let app_dir = PathBuf::from(format!("../../app/{}", cli.app));

    let app_dir = app_dir
        .canonicalize()
        .with_context(|| format!("App directory not found: {}", app_dir.display()))
        .inspect_err(|err| {
            printer.error(err);
            printer.hint("Check that the app name matches a folder under examples/app/");
        })?;

    let paths = Arc::new(AppPaths::from_app_dir(&app_dir));

    printer.header("WebUI Hyper Server");
    printer.field("App", &cli.app);
    printer.field("Directory", &app_dir.display());

    // Initial render to index.html
    render_to_index_html(&paths)
        .context("Failed initial render")
        .inspect_err(|err| {
            printer.error(err);
        })?;
    printer.success("Initial render complete");

    // File watcher thread: re-render when template or data change
    start_file_watcher((*paths).clone());
    printer.success("File watcher started");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    let listener = TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;

    printer.field("URL", &"http://127.0.0.1:8080/");
    printer.finish("Server is running — press Ctrl+C to stop");

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let paths = Arc::clone(&paths);

        tokio::task::spawn(async move {
            if let Err(err) = http2::Builder::new(TokioExecutor::new())
                .serve_connection(
                    io,
                    service_fn(|req| handle_request(req, Arc::clone(&paths))),
                )
                .await
            {
                eprintln!("Connection error: {err}");
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    paths: Arc<AppPaths>,
) -> std::result::Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let path = req.uri().path();

    let response = match path {
        "/" | "/index.html" => routes::index::handle_index(),
        p if p.starts_with("/assets/") => routes::assets::handle_asset(p, &paths),
        "/hmr" => routes::hmr::handle_hmr(&paths),
        _ => routes::not_found::handle_not_found(),
    };

    Ok(response)
}
