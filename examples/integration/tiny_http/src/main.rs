use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use tiny_http::Server;

#[path = "../../../../examples/shared/rust/config.rs"]
mod config;
mod render;
mod routes {
    pub mod assets;
    pub mod hmr;
    pub mod index;
    pub mod not_found;
}
mod watcher;

use crate::config::AppPaths;
use crate::render::render_to_index_html;
use crate::watcher::start_file_watcher;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Parse --app <name> from command-line arguments, default to "hello-world"
    let app_name = parse_app_arg().unwrap_or_else(|| "hello-world".to_string());
    let app_dir = PathBuf::from(format!("../../app/{app_name}"));

    if !app_dir.exists() {
        eprintln!("Error: app directory not found: {}", app_dir.display());
        std::process::exit(1);
    }

    let paths = Arc::new(AppPaths::from_app_dir(&app_dir));

    println!("Using app: {app_name} ({})", app_dir.display());

    // Initial render to index.html
    render_to_index_html(&paths)?;

    // File watcher thread: re-render when template or data change
    start_file_watcher((*paths).clone());

    let server = Server::http("127.0.0.1:8080")?;

    println!("Serving index.html at http://127.0.0.1:8080/");
    println!("Press Ctrl+C to stop the server.");

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

/// Parse the `--app <name>` argument from command-line args.
fn parse_app_arg() -> Option<String> {
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--app" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}
