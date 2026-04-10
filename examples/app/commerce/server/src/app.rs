// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::path::Path;

use crate::catalog::Catalog;
use crate::frontend::FrontendRuntime;
use crate::image_proxy::ImageCache;
use crate::rate_limit::RateLimiter;

pub(crate) struct AppState {
    catalog: Catalog,
    frontend: FrontendRuntime,
    rate_limiter: RateLimiter,
    image_cache: ImageCache,
}

impl AppState {
    pub(crate) fn load(app_root: &Path, css: webui::CssStrategy) -> Result<Self> {
        let frontend = FrontendRuntime::load(app_root, css)?;
        let catalog = Catalog::generate();
        // 60 mutation requests per IP per minute
        let rate_limiter = RateLimiter::new(60, 60);
        let image_cache = ImageCache::load(&app_root.join("images"))?;
        Ok(Self {
            catalog,
            frontend,
            rate_limiter,
            image_cache,
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
    pub(crate) fn image_count(&self) -> usize {
        self.image_cache.len()
    }

    #[must_use]
    pub(crate) fn image_cache(&self) -> &ImageCache {
        &self.image_cache
    }

    #[must_use]
    pub(crate) fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }
}

#[cfg(test)]
pub(crate) fn test_state() -> actix_web::web::Data<AppState> {
    test_state_with_css(webui::CssStrategy::Link)
}

#[cfg(test)]
pub(crate) fn test_state_with_css(css: webui::CssStrategy) -> actix_web::web::Data<AppState> {
    let app_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("server crate should live under the app directory");
    let state = match AppState::load(app_root, css) {
        Ok(state) => state,
        Err(error) => panic!("Failed to build the commerce WebUI protocol: {error:#}"),
    };
    actix_web::web::Data::new(state)
}
