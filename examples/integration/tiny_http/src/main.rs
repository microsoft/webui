use std::error::Error;
use tiny_http::Server;

mod routes {
    pub mod index;
    pub mod not_found;
    pub mod hmr;
    pub mod assets;
}
mod render;
mod watcher;

use crate::render::render_to_index_html;
use crate::watcher::start_file_watcher;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initial render to index.html
    render_to_index_html()?;

    // File watcher thread: re-render when template or data change
    start_file_watcher();

    let server = Server::http("127.0.0.1:8080")?;

    println!("Serving index.html at http://127.0.0.1:8080/");
    println!("Press Ctrl+C to stop the server.");

    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("/");

        match path {
            "/" | "/index.html" => {
                routes::index::handle_index(request);
            }
            path if path.starts_with("/assets/") => {
                routes::assets::handle_asset(request);
            }
            "/hmr" => {
                routes::hmr::handle_hmr(request);
            }
            _ => {
                routes::not_found::handle_not_found(request);
            }
        }
    }

    Ok(())
}

