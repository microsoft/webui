// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use crate::catalog::Catalog;
use crate::frontend::FrontendRuntime;
use crate::image_proxy::ImageCache;
use crate::rate_limit::RateLimiter;

pub(crate) struct AppState {
    catalog: Catalog,
    frontend: FrontendRuntime,
    rate_limiter: RateLimiter,
    image_cache: ImageCache,
    base_path: String,
    /// Shared lock-free chunk-buffer pool used by every streaming
    /// response. One pool per server; recycles chunk Vec across all
    /// concurrent renders. Sized for ~256 in-flight chunks ≈ 1.25 MiB
    /// peak pool memory; bounded.
    chunk_pool: Arc<webui::streaming::ChunkPool>,
}

impl AppState {
    pub(crate) fn load(app_root: &Path, css: webui::CssStrategy, base_path: &str) -> Result<Self> {
        let frontend = FrontendRuntime::load(app_root, css)?;
        Self::with_frontend(app_root, base_path, frontend)
    }

    #[cfg(test)]
    fn load_for_tests(app_root: &Path, css: webui::CssStrategy, base_path: &str) -> Result<Self> {
        let frontend = FrontendRuntime::load_for_tests(app_root, css)?;
        Self::with_frontend(app_root, base_path, frontend)
    }

    fn with_frontend(app_root: &Path, base_path: &str, frontend: FrontendRuntime) -> Result<Self> {
        let catalog = Catalog::generate();
        // 60 mutation requests per IP per minute
        let rate_limiter = RateLimiter::new(60, 60);
        let image_cache = ImageCache::load(&app_root.join("images"))?;
        let chunk_pool = Arc::new(webui::streaming::ChunkPool::new(
            256,
            webui::streaming::StreamingWriter::CHUNK_TARGET + 1024,
        ));
        Ok(Self {
            catalog,
            frontend,
            rate_limiter,
            image_cache,
            base_path: base_path.to_string(),
            chunk_pool,
        })
    }

    #[must_use]
    pub(crate) fn base_path(&self) -> &str {
        &self.base_path
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

    /// Cheap-cloneable handle to the shared chunk pool.
    #[must_use]
    pub(crate) fn chunk_pool(&self) -> Arc<webui::streaming::ChunkPool> {
        Arc::clone(&self.chunk_pool)
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
    let state = match AppState::load_for_tests(app_root, css, "/") {
        Ok(state) => state,
        Err(error) => panic!("Failed to build the commerce WebUI protocol: {error:#}"),
    };
    actix_web::web::Data::new(state)
}
