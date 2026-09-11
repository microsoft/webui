// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use webui_discovery::{
    discover_source, discover_source_with_plugin, DiscoveredComponent, DiscoveryPlugin,
    FastDiscoveryPlugin, PackageContext, WebUIDiscoveryPlugin,
};

#[test]
fn default_discovery_does_not_read_package_or_fast_metadata(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/native-package");
    fs::create_dir_all(&package)?;
    fs::write(package.join("package.json"), [0xff])?;
    fs::write(package.join("custom-elements.json"), [0xff])?;
    fs::write(package.join("native-card.html"), "<span>Native</span>")?;
    fs::write(package.join("native-card.css"), "span { color: blue; }")?;

    let result = discover_source("native-package", root.path())?;
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].tag_name, "native-card");
    assert!(!result.components[0].is_client_owned);
    assert!(discover_source_with_plugin(
        "native-package",
        root.path(),
        &FastDiscoveryPlugin::new()
    )
    .is_err());
    Ok(())
}

struct MetadataFreePlugin {
    calls: AtomicUsize,
}

impl DiscoveryPlugin for MetadataFreePlugin {
    fn cache_namespace(&self) -> &'static str {
        "metadata-free-test"
    }
    fn discover_local(&self, root: &Path) -> anyhow::Result<Vec<DiscoveredComponent>> {
        WebUIDiscoveryPlugin::new().discover_local(root)
    }
    fn package_cache_files(&self, package: PackageContext<'_>) -> anyhow::Result<Vec<PathBuf>> {
        assert!(package.manifest.is_none());
        WebUIDiscoveryPlugin::new().package_cache_files(package)
    }
    fn discover_package(
        &self,
        package: PackageContext<'_>,
    ) -> anyhow::Result<Vec<DiscoveredComponent>> {
        assert!(package.manifest.is_none());
        self.calls.fetch_add(1, Ordering::Relaxed);
        WebUIDiscoveryPlugin::new().discover_package(package)
    }
}

#[test]
fn unused_metadata_does_not_parse_or_invalidate_filename_cache(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let package = root.path().join("node_modules/native-package");
    fs::create_dir_all(&package)?;
    fs::write(package.join("package.json"), "{}")?;
    fs::write(package.join("native-card.html"), "<span>Native</span>")?;
    let plugin = MetadataFreePlugin {
        calls: AtomicUsize::new(0),
    };
    assert!(!plugin.requires_package_metadata());
    assert!(FastDiscoveryPlugin::new().requires_package_metadata());
    discover_source_with_plugin("native-package", root.path(), &plugin)?;
    fs::write(package.join("package.json"), [0xff])?;
    discover_source_with_plugin("native-package", root.path(), &plugin)?;
    assert_eq!(plugin.calls.load(Ordering::Relaxed), 1);
    Ok(())
}
