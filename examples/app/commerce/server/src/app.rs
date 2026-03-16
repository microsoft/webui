// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;
use std::path::Path;

use crate::catalog::Catalog;
use crate::frontend::FrontendRuntime;

pub(crate) struct AppState {
    catalog: Catalog,
    frontend: FrontendRuntime,
}

impl AppState {
    pub(crate) fn load(app_root: &Path) -> Result<Self> {
        let frontend = FrontendRuntime::load(app_root)?;
        let catalog = Catalog::generate();
        Ok(Self { catalog, frontend })
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
}

#[cfg(test)]
pub(crate) fn test_state() -> actix_web::web::Data<AppState> {
    let app_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("server crate should live under the app directory");
    let state = match AppState::load(app_root) {
        Ok(state) => state,
        Err(error) => panic!("{error}"),
    };
    actix_web::web::Data::new(state)
}
