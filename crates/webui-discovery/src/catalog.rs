// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::{DirEntry, WalkDir};

use crate::npm::{
    package_asset_metadata, read_optional_file, read_required_file, validate_package_asset_path,
};
use crate::{has_sibling_script, DiscoveredComponent};

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
    if template_tag(&template).is_some() && include(&template) && template.is_file() {
        validate_package_asset_path(root, path, "component catalog asset")?;
    }
    Ok(())
}

fn append_cache_files(
    root: &Path,
    include: impl Fn(&Path) -> bool,
    package_root: Option<&Path>,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for template in templates(root, include, package_root) {
        let template = template?;
        for extension in ["css", "ts", "js"] {
            files.push(template.with_extension(extension));
        }
        files.push(template);
    }
    Ok(())
}

fn has_templates_matching(root: &Path, include: impl Fn(&Path) -> bool) -> Result<bool> {
    templates(root, include, None)
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
    for template in templates(root, include, package_root) {
        let template = template?;
        let tag_name = template
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("Component template has no valid tag name")?;
        components.push(DiscoveredComponent {
            tag_name: tag_name.to_string(),
            html_content: read_required_file(&template, "component template")?,
            css_content: read_styles(&template.with_extension("css"), package_root)?,
            is_client_owned: has_sibling_script(&template, package_root)?,
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
        let mut files = Vec::new();
        append_cache_files(&package.join("components"), |_| true, None, &mut files)?;
        let cache = DiscoveryCache::open()?;
        for namespace in ["webui", "webui-v2"] {
            cache.put(
                &CacheKey {
                    namespace,
                    source: "fixture-catalog",
                    package_json: &package_json,
                    fingerprint: DiscoveryCache::fingerprint(Some(&package_json), &files)?,
                },
                &[DiscoveredComponent {
                    tag_name: "test-text".to_string(),
                    html_content: "<span>Static</span>".to_string(),
                    css_content: None,
                    is_client_owned: true,
                    source: "fixture-catalog".to_string(),
                }],
            )?;
        }

        let result = crate::discover_source("fixture-catalog", root.path())?;
        assert_eq!(result.components.len(), 1);
        assert!(!result.components[0].is_client_owned);
        Ok(())
    }
}
