// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{web, HttpResponse};

use crate::registry::AppEntry;

/// JSON representation of an app for the `/api/apps` endpoint.
#[derive(serde::Serialize)]
struct AppInfo {
    name: String,
    slug: String,
    description: String,
    backend: String,
    source_url: String,
}

/// Returns the list of discovered apps as JSON.
pub(crate) async fn apps_list(apps: web::Data<Vec<AppEntry>>) -> HttpResponse {
    let list: Vec<AppInfo> = apps
        .iter()
        .map(|a| AppInfo {
            name: a.name.clone(),
            slug: a.slug.clone(),
            description: a.description.clone(),
            backend: a.backend.clone(),
            source_url: a.source_url(),
        })
        .collect();

    HttpResponse::Ok().json(list)
}
