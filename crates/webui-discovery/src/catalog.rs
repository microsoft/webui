// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use walkdir::WalkDir;

use crate::npm::{read_optional_file, read_required_file, PackageContext};
use crate::{has_sibling_script, DiscoveredComponent};

pub(crate) fn root(package: PackageContext<'_>) -> Result<PathBuf> {
    let root = package.root.join("components");
    match fs::metadata(&root) {
        Ok(metadata) if metadata.is_dir() => Ok(root),
        Ok(_) => bail!("Component catalog must be a directory: {}", root.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(package.root.to_path_buf())
        }
        Err(error) => Err(error).with_context(|| format!("Cannot inspect {}", root.display())),
    }
}

pub(crate) fn template_tag(path: &Path) -> Option<&str> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| stem.contains('-'))
}

fn templates(root: &Path) -> impl Iterator<Item = Result<PathBuf>> + '_ {
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
            Ok(entry) => {
                let path = entry.path();
                (path.extension().is_some_and(|ext| ext == "html")
                    && template_tag(path).is_some()
                    && path.is_file())
                .then(|| Ok(entry.into_path()))
            }
        })
}

pub(crate) fn cache_files(root: &Path) -> Result<Vec<PathBuf>> {
    cache_files_matching(root, |_| true)
}

pub(crate) fn cache_files_matching(
    root: &Path,
    include: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for template in templates(root) {
        let template = template?;
        if !include(&template) {
            continue;
        }
        for extension in ["css", "ts", "js"] {
            files.push(template.with_extension(extension));
        }
        files.push(template);
    }
    Ok(files)
}

pub(crate) fn has_templates(root: &Path) -> Result<bool> {
    templates(root)
        .next()
        .transpose()
        .map(|path| path.is_some())
}

pub(crate) fn has_templates_matching(root: &Path, include: impl Fn(&Path) -> bool) -> Result<bool> {
    for path in templates(root) {
        if include(&path?) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn discover(source: &str, root: &Path) -> Result<Vec<DiscoveredComponent>> {
    discover_matching(source, root, |_| true)
}

pub(crate) fn discover_matching(
    source: &str,
    root: &Path,
    include: impl Fn(&Path) -> bool,
) -> Result<Vec<DiscoveredComponent>> {
    let mut components = Vec::new();
    for template in templates(root) {
        let template = template?;
        if !include(&template) {
            continue;
        }
        let tag_name = template
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("Component template has no valid tag name")?;
        components.push(DiscoveredComponent {
            tag_name: tag_name.to_string(),
            html_content: read_required_file(&template, "component template")?,
            css_content: read_optional_file(
                Some(&template.with_extension("css")),
                "component styles",
            )?,
            is_client_owned: has_sibling_script(&template)?,
            source: source.to_string(),
        });
    }
    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CacheKey, DiscoveryCache};

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
        let files = cache_files(&package.join("components"))?;
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
