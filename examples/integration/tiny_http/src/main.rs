use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tiny_http::Server;

#[path = "../../../../examples/shared/rust/config.rs"]
mod config;
#[path = "../../../../examples/shared/rust/hmr.rs"]
mod hmr;
#[path = "../../../../examples/shared/rust/output.rs"]
mod output;
#[path = "../../../../examples/shared/rust/render.rs"]
mod render;
mod routes {
    pub mod assets;
    pub mod hmr;
    pub mod index;
    pub mod not_found;
}
#[path = "../../../../examples/shared/rust/watcher.rs"]
mod watcher;

use crate::config::AppPaths;
use crate::output::Printer;
use crate::render::render_to_index_html;
use crate::watcher::start_file_watcher;

#[derive(Parser)]
#[command(name = "webui-tiny-http", about = "Serve a WebUI app with tiny_http")]
struct Cli {
    /// App name inside examples/app/ (defaults to hello-world)
    #[arg(long, default_value = "hello-world")]
    app: String,
}

fn main() {
    let cli = Cli::parse();

    if let Err(err) = run(&cli) {
        eprintln!("\n  ✘ {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<()> {
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

    printer.header("WebUI Tiny HTTP Server");
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

    let server = Server::http("127.0.0.1:8080")
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to start server")?;

    printer.field("URL", &"http://127.0.0.1:8080/");
    printer.finish("Server is running — press Ctrl+C to stop");

    for request in server.incoming_requests() {
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("/");

        match path {
            "/" | "/index.html" => {
                routes::index::handle_index(request);
            }
            p if p.starts_with("/assets/") => {
                routes::assets::handle_asset(request, &paths);
            }
            "/hmr" => {
                routes::hmr::handle_hmr(request, &paths);
            }
            _ => {
                routes::not_found::handle_not_found(request);
            }
        }
    }

    Ok(())
}
