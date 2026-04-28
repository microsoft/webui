// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

mod api;
mod health;
mod process;
mod proxy;
mod registry;
mod shell;

use actix_web::{web, App, HttpServer};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "demo-shell",
    about = "WebUI demo shell — hosts all example apps"
)]
struct Args {
    /// Port to listen on (the single exposed port).
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// Directory containing app subdirectories with `demo.toml` files.
    #[arg(long, default_value = "./apps")]
    apps_dir: PathBuf,

    /// Base port for dynamically assigned app ports.
    #[arg(long, default_value_t = 3100)]
    base_port: u16,

    /// Directory of the shell WebUI app (containing `src/index.html` and
    /// `dist/index.js`). Defaults to `./examples/demo` for local dev; the
    /// container image overrides this to `./shell`.
    #[arg(long, default_value = "./examples/demo")]
    shell_dir: PathBuf,
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    log::info!("Discovering apps in {:?}", args.apps_dir);
    let apps = registry::discover(&args.apps_dir, args.base_port)?;
    log::info!("Found {} app(s)", apps.len());

    // Compile the shell WebUI app once at startup.
    let shell_state = shell::ShellState::build(&args.shell_dir)?;

    // Build the proxy routing table
    let routes: HashMap<String, u16> = apps.iter().map(|a| (a.slug.clone(), a.port)).collect();

    let proxy_state = web::Data::new(proxy::ProxyState { routes });
    let apps_data = web::Data::new(apps.clone());
    let shell_data = web::Data::new(shell_state);

    // Spawn all child app servers
    log::info!("Starting app servers…");
    let mut pm = process::ProcessManager::spawn_all(&apps)?;

    // Give child servers a moment to start
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    log::info!("Starting demo shell on port {}", args.port);

    let result = HttpServer::new(move || {
        let client = awc::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .finish();

        App::new()
            .app_data(proxy_state.clone())
            .app_data(apps_data.clone())
            .app_data(shell_data.clone())
            .app_data(web::Data::new(client))
            // Shell UI (rendered from examples/demo/src via WebUIHandler)
            .route("/", web::get().to(shell::shell_page))
            // Shell static assets (bundled JS, sourcemaps, etc.)
            .route("/_shell/{tail:.*}", web::get().to(shell::shell_asset))
            // API
            .route("/api/apps", web::get().to(api::apps_list))
            // Health
            .route("/health", web::get().to(health::health))
            .route("/health/apps", web::get().to(health::health_apps))
            // Proxy: redirect bare slug to slug/ for correct relative paths
            .route("/{slug}", web::get().to(proxy::slug_redirect))
            // Proxy: forward /{slug}/{tail} to internal app server
            .route("/{slug}/{tail:.*}", web::to(proxy::proxy_handler))
    })
    .bind(("0.0.0.0", args.port))?
    .run()
    .await;

    // Clean up child processes
    pm.kill_all();

    result.map_err(Into::into)
}
