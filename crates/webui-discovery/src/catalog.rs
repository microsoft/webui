// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use walkdir::{DirEntry, WalkDir};

use crate::npm::{
    package_asset_metadata, read_optional_file, read_required_file, validate_package_asset_path,
};
use crate::{has_sibling_script, ComponentFile, ComponentFileSource, DiscoveredComponent};

mod package;
pub(crate) use package::PackageCatalog;

pub(crate) fn template_tag(path: &Path) -> Option<&str> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| stem.contains('-'))
}

fn templates<'a>(
    root: &'a Path,
    include: impl Fn(&Path) -> bool + 'a,
    package_root: Option<&'a Path>,
) -> impl Iterator<Item = Result<PathBuf>> + 'a {
    WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || (entry.file_name() != "node_modules"
                    && !entry.file_name().to_string_lossy().starts_with('.'))
        })
        .filter_map(move |entry| match entry {
            Err(error) => {
                Some(Err(error).with_context(|| format!("Cannot scan {}", root.display())))
            }
            Ok(entry) => template_entry(entry, &include, package_root).transpose(),
        })
}

fn template_entry(
    entry: DirEntry,
    include: &impl Fn(&Path) -> bool,
    package_root: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let path = entry.path();
    let is_template = path.extension().is_some_and(|ext| ext == "html")
        && template_tag(path).is_some()
        && include(path);
    // WalkDir already supplies the non-following file type. Only links need
    // containment resolution; nested directory links are never traversed.
    if entry.file_type().is_symlink() {
        if let Some(root) = package_root {
            validate_catalog_link(path, root, include)?;
        }
        return Ok((is_template && path.is_file()).then(|| entry.into_path()));
    }
    Ok((is_template && entry.file_type().is_file()).then(|| entry.into_path()))
}

fn validate_catalog_link(path: &Path, root: &Path, include: &impl Fn(&Path) -> bool) -> Result<()> {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return Ok(());
    };
    if !matches!(extension, "html" | "css" | "ts" | "js") {
        return Ok(());
    }
    let template = path.with_extension("html");
    if template_tag(&template).is_some()
        && include(&template)
        && (extension == "html" || template.is_file())
    {
        validate_package_asset_path(root, path, "component catalog asset")?;
    }
    Ok(())
}

fn prepare_components(
    root: &Path,
    include: impl Fn(&Path) -> bool,
    package_root: &Arc<Path>,
    components: &mut Vec<ComponentFileSource>,
) -> Result<()> {
    let mut sibling = PathBuf::new();
    for template in templates(root, include, Some(package_root)) {
        let template = template?;
        sibling.clear();
        sibling.push(&template);
        sibling.set_extension("css");
        let css = if package_asset_metadata(package_root, &sibling)?
            .is_some_and(|metadata| metadata.is_file())
        {
            Some(ComponentFile::new(Arc::clone(package_root), &sibling)?)
        } else {
            None
        };
        let tag_name = template_tag(&template)
            .context("Component template has no valid tag name")?
            .to_string();
        components.push(ComponentFileSource {
            tag_name,
            css,
            is_client_owned: has_sibling_script(&mut sibling, Some(package_root))?,
            html: ComponentFile::new(Arc::clone(package_root), &template)?,
        });
    }
    Ok(())
}

fn has_templates_matching(
    root: &Path,
    include: impl Fn(&Path) -> bool,
    package_root: &Path,
) -> Result<bool> {
    templates(root, include, Some(package_root))
        .next()
        .transpose()
        .map(|template| template.is_some())
}

pub(crate) fn discover(source: &str, root: &Path) -> Result<Vec<DiscoveredComponent>> {
    let mut components = Vec::new();
    append_components(source, root, |_| true, None, &mut components)?;
    Ok(components)
}

fn append_components(
    source: &str,
    root: &Path,
    include: impl Fn(&Path) -> bool,
    package_root: Option<&Path>,
    components: &mut Vec<DiscoveredComponent>,
) -> Result<()> {
    let mut sibling = PathBuf::new();
    for template in templates(root, include, package_root) {
        let template = template?;
        sibling.clear();
        sibling.push(&template);
        sibling.set_extension("css");
        let tag_name = template
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("Component template has no valid tag name")?;
        components.push(DiscoveredComponent {
            tag_name: tag_name.to_string(),
            html_content: read_required_file(&template, "component template")?,
            css_content: read_styles(&sibling, package_root)?,
            is_client_owned: has_sibling_script(&mut sibling, package_root)?,
            source: source.to_string(),
        });
    }
    Ok(())
}

fn read_styles(path: &Path, package_root: Option<&Path>) -> Result<Option<String>> {
    let Some(root) = package_root else {
        return read_optional_file(Some(path), "component styles");
    };
    if package_asset_metadata(root, path)?.is_some_and(|metadata| metadata.is_file()) {
        read_required_file(path, "component styles").map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CacheKey, DiscoveryCache};
    use crate::{prepared, ComponentFileSource};
    use std::fs;

    #[test]
    fn package_wide_catalog_ownership_cache_is_not_reused() -> Result<()> {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/fixture-catalog");
        let component = package.join("components/test-text");
        fs::create_dir_all(&component)?;
        fs::write(component.join("test-text.html"), "<span>Static</span>")?;
        fs::write(
            package.join("package.json"),
            r#"{"exports":{"./button.js":"./dist/button.js"}}"#,
        )?;
        let package = package.canonicalize()?;
        let package_json = package.join("package.json");
        let inputs = vec![ComponentFileSource {
            tag_name: "test-text".to_string(),
            html: ComponentFile {
                package_root: package.as_path().into(),
                path: package.join("components/test-text/test-text.html"),
            },
            css: None,
            is_client_owned: true,
        }];
        let loaded = prepared::load(
            "fixture-catalog",
            &fs::read_to_string(&package_json)?,
            inputs,
        )?;
        let cache = DiscoveryCache::open()?;
        for namespace in ["webui", "webui-v2"] {
            cache.put(
                &CacheKey {
                    namespace,
                    source: "fixture-catalog",
                    package_json: &package_json,
                },
                &loaded,
            )?;
        }

        let result = crate::discover_source("fixture-catalog", root.path())?;
        assert_eq!(result.components.len(), 1);
        assert!(!result.components[0].is_client_owned);
        Ok(())
    }
}
