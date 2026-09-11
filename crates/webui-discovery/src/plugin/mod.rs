// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Discovery contracts and the default filename-based implementation.

use crate::npm::PackageContext;
use crate::DiscoveredComponent;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

mod fast;
pub use fast::FastDiscoveryPlugin;

/// Maps a resolved local or npm package layout to WebUI component registrations.
pub trait DiscoveryPlugin {
    /// Stable cache namespace for this discovery layout.
    fn cache_namespace(&self) -> &'static str;

    /// Opt into reading, parsing, and cache-hashing `package.json` contents.
    ///
    /// Filename-only discovery does not need package metadata. Plugins that
    /// interpret package fields must opt in before accessing `PackageContext::manifest`.
    #[must_use]
    fn requires_package_metadata(&self) -> bool {
        false
    }

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
        "webui-filenames"
    }

    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>> {
        crate::catalog::discover(&root.to_string_lossy(), root)
    }

    fn supports_package(&self, package: PackageContext<'_>) -> Result<bool> {
        crate::catalog::has_templates(&crate::catalog::root(package)?)
    }

    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
        crate::catalog::cache_files(&crate::catalog::root(package)?)
    }

    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>> {
        let root = crate::catalog::root(package)?;
        let components = crate::catalog::discover(package.name, &root)?;
        if components.is_empty() {
            bail!(
                "No component templates in {}. Add <component-name>.html files; \
                 the filename is the custom element name.",
                root.display()
            );
        }
        Ok(components)
    }
}
