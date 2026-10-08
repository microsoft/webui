// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Discovery contracts and the default filename-based implementation.

use crate::catalog::PackageCatalog;
use crate::npm::PackageContext;
use crate::DiscoveredComponent;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

mod fast;
pub use fast::FastDiscoveryPlugin;

/// Maps a resolved local or npm package layout to WebUI component registrations.
///
/// Package resolution supplies parsed metadata through [`PackageContext::manifest`]
/// and includes `package.json` in cache invalidation for every plugin.
pub trait DiscoveryPlugin {
    /// Stable cache namespace for this discovery layout.
    fn cache_namespace(&self) -> &'static str;

    /// Discover components below a local source root.
    ///
    /// # Errors
    ///
    /// Returns an error when a claimed component source cannot be read.
    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>>;

    /// Whether a package declares components using this discovery layout.
    ///
    /// Scope searches skip packages returning `false`, but propagate failures
    /// from packages that declare components. Explicit package requests still
    /// report invalid or missing component inputs. Custom plugins default to
    /// accepting every package.
    ///
    /// # Errors
    ///
    /// Returns an error when component-source presence cannot be determined.
    fn supports_package(&self, _package: PackageContext<'_>) -> Result<bool> {
        Ok(true)
    }

    /// Return every package file whose contents or existence affects discovery.
    ///
    /// Paths must be deterministic. Missing optional candidates should still be
    /// included so creating one invalidates a prior cache entry.
    ///
    /// # Errors
    ///
    /// Returns an error when package metadata needed to identify dependencies
    /// is invalid.
    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>>;

    /// Discover components in a validated npm package.
    ///
    /// # Errors
    ///
    /// Returns an error when required package metadata or component sources are
    /// missing or invalid.
    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>>;
}

/// Default discovery using component filenames and matching sibling files.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebUIDiscoveryPlugin;

impl WebUIDiscoveryPlugin {
    /// Create default filename-based discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DiscoveryPlugin for WebUIDiscoveryPlugin {
    fn cache_namespace(&self) -> &'static str {
        "webui-package-roots-v1"
    }

    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>> {
        crate::catalog::discover(&root.to_string_lossy(), root)
    }

    fn supports_package(&self, package: PackageContext<'_>) -> Result<bool> {
        PackageCatalog::new(package)?.has_templates(|_| true)
    }

    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
        PackageCatalog::new(package)?.cache_files(|_| true)
    }

    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>> {
        let catalog = PackageCatalog::new(package)?;
        let components = catalog.discover(|_| true)?;
        if components.is_empty() && !catalog.is_disabled() {
            bail!(
                "No component templates in {}. Add <component-name>.html files under \
                 components/ or declare webui.components in package.json; \
                 the filename is the custom element name.",
                package.root.display()
            );
        }
        Ok(components)
    }
}
