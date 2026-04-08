// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::path::Path;

use crate::catalog::Catalog;
use crate::frontend::FrontendRuntime;
use crate::rate_limit::RateLimiter;

pub(crate) struct AppState {
    catalog: Catalog,
    frontend: FrontendRuntime,
    rate_limiter: RateLimiter,
}

impl AppState {
    pub(crate) fn load(app_root: &Path, css: webui::CssStrategy) -> Result<Self> {
        let frontend = FrontendRuntime::load(app_root, css)?;
        let catalog = Catalog::generate();
        // 60 mutation requests per IP per minute
        let rate_limiter = RateLimiter::new(60, 60);
        Ok(Self {
            catalog,
            frontend,
            rate_limiter,
        })
    }

    #[must_use]
    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    #[must_use]
    pub(crate) fn frontend(&self) -> &FrontendRuntime {
        &self.frontend
    }

    #[must_use]
    pub(crate) fn product_count(&self) -> usize {
        self.catalog.product_count()
    }

    #[must_use]
    pub(crate) fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }
}

#[cfg(test)]
pub(crate) fn test_state() -> actix_web::web::Data<AppState> {
    let app_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("server crate should live under the app directory");
    let state = match AppState::load(app_root, webui::CssStrategy::Link) {
        Ok(state) => state,
        Err(error) => panic!("Failed to build the commerce WebUI protocol: {error:#}"),
    };
    actix_web::web::Data::new(state)
}
