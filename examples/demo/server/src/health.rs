// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{web, HttpResponse};

use crate::registry::AppEntry;

/// Simple liveness check for the shell itself.
pub(crate) async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

/// Per-app health status: probes each app's internal port.
pub(crate) async fn health_apps(apps: web::Data<Vec<AppEntry>>) -> HttpResponse {
    let mut statuses = serde_json::Map::new();

    for app in apps.iter() {
        let url = format!("http://127.0.0.1:{}/", app.port);
        let reachable = reqwest_probe(&url).await;
        statuses.insert(
            app.slug.clone(),
            serde_json::json!({
                "port": app.port,
                "healthy": reachable,
            }),
        );
    }

    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "apps": statuses,
    }))
}

/// Quick TCP-level probe to check if a port is listening.
async fn reqwest_probe(url: &str) -> bool {
    // Use a lightweight TCP connect instead of a full HTTP request
    // to avoid depending on reqwest.
    let addr = url
        .strip_prefix("http://")
        .unwrap_or(url)
        .trim_end_matches('/');
    tokio::net::TcpStream::connect(addr).await.is_ok()
}
