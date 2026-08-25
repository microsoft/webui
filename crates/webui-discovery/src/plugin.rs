// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Component discovery plugin contracts and built-in layouts.

use crate::npm::{
    package_component_declarations, read_optional_file, read_required_file, resolve_webui_assets,
    PackageContext,
};
use crate::{has_sibling_script, DiscoveredComponent};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Maps a resolved local or npm package layout to WebUI component registrations.
pub trait DiscoveryPlugin {
    /// Stable cache namespace for this discovery layout.
    fn cache_namespace(&self) -> &'static str;

    /// Discover components below a local source root.
    ///
    /// # Errors
    ///
    /// Returns an error when a claimed component source cannot be read.
    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>>;

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

/// Discovery for WebUI's native component package layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebUIDiscoveryPlugin;

impl WebUIDiscoveryPlugin {
    /// Create WebUI native discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DiscoveryPlugin for WebUIDiscoveryPlugin {
    fn cache_namespace(&self) -> &'static str {
        "webui"
    }

    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>> {
        discover_local_templates(root, |path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.contains('-'))
        })
    }

    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
        let assets = resolve_webui_assets(package)?;
        let mut files = Vec::with_capacity(3);
        files.push(assets.manifest_path);
        files.push(assets.template_path);
        if let Some(styles) = assets.styles_path {
            files.push(styles);
        }
        Ok(files)
    }

    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>> {
        let assets = resolve_webui_assets(package)?;
        let html_content = read_required_file(&assets.template_path, "component template")?;
        let css_content = read_optional_file(assets.styles_path.as_deref(), "component styles")?;
        let tag_names = package_component_declarations(package)?
            .into_iter()
            .map(|declaration| declaration.tag_name)
            .collect::<Vec<_>>();
        if tag_names.is_empty() {
            bail!(
                "No component tag names found in custom elements manifest: {}",
                assets.manifest_path.display()
            );
        }

        Ok(tag_names
            .into_iter()
            .map(|tag_name| DiscoveredComponent {
                tag_name,
                html_content: html_content.clone(),
                css_content: css_content.clone(),
                is_client_owned: package.is_client_owned,
                source: package.name.to_string(),
            })
            .collect())
    }
}

/// Discovery for FAST generated component layouts.
#[derive(Debug, Default, Clone, Copy)]
pub struct FastDiscoveryPlugin;

impl FastDiscoveryPlugin {
    /// Create FAST discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DiscoveryPlugin for FastDiscoveryPlugin {
    fn cache_namespace(&self) -> &'static str {
        "fast"
    }

    fn discover_local(&self, root: &Path) -> Result<Vec<DiscoveredComponent>> {
        discover_local_templates(root, |path| {
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                return false;
            };
            file_name.ends_with(".template.html")
                || path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.contains('-'))
        })
    }

    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
        let declarations = package_component_declarations(package)?;
        let mut files = Vec::with_capacity(1 + declarations.len() * 4);
        files.push(crate::npm::custom_elements_manifest_path(package)?);
        for declaration in declarations {
            let module_path = declaration.module_path.as_deref().with_context(|| {
                format!(
                    "FAST component <{}> in package '{}' has no CEM module path",
                    declaration.tag_name, package.name
                )
            })?;
            let declaration_name = declaration.name.as_deref().unwrap_or(&declaration.tag_name);
            for candidate in fast_template_candidates(package.root, module_path, declaration_name) {
                files.push(candidate.clone());
                files.extend(fast_style_candidates(&candidate));
            }
        }
        Ok(files)
    }

    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>> {
        let declarations = package_component_declarations(package)?;
        if declarations.is_empty() {
            bail!(
                "No component declarations found in package '{}'",
                package.name
            );
        }

        let mut components = Vec::with_capacity(declarations.len());
        let mut seen_templates = HashSet::with_capacity(declarations.len());
        for declaration in declarations {
            let module_path = declaration.module_path.as_deref().with_context(|| {
                format!(
                    "FAST component <{}> in package '{}' has no CEM module path",
                    declaration.tag_name, package.name
                )
            })?;
            let declaration_name = declaration.name.as_deref().unwrap_or(&declaration.tag_name);
            let template_path = resolve_fast_template(package.root, module_path, declaration_name)
                .with_context(|| {
                    format!(
                        "Failed to locate FAST template for <{}> in package '{}'",
                        declaration.tag_name, package.name
                    )
                })?;
            if !seen_templates.insert(template_path.clone()) {
                bail!(
                    "FAST template {} maps to multiple component declarations",
                    template_path.display()
                );
            }
            let html_content = read_required_file(&template_path, "FAST component template")?;
            let css_content = read_optional_file(
                resolve_fast_styles(&template_path).as_deref(),
                "FAST component styles",
            )?;
            components.push(DiscoveredComponent {
                tag_name: declaration.tag_name,
                html_content,
                css_content,
                is_client_owned: package.is_client_owned,
                source: package.name.to_string(),
            });
        }
        Ok(components)
    }
}

fn discover_local_templates(
    root: &Path,
    claims: impl Fn(&Path) -> bool,
) -> Result<Vec<DiscoveredComponent>> {
    let source = root.display().to_string();
    let mut components = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry.with_context(|| format!("Failed to scan {}", root.display()))?;
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|extension| extension == "html")
            || !claims(path)
        {
            continue;
        }
        let tag_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .with_context(|| format!("Invalid component filename: {}", path.display()))?;
        let html_content = read_required_file(path, "component template")?;
        let css_content =
            read_optional_file(resolve_local_styles(path).as_deref(), "component styles")?;
        components.push(DiscoveredComponent {
            tag_name: tag_name.to_string(),
            html_content,
            css_content,
            is_client_owned: has_sibling_script(path)?,
            source: source.clone(),
        });
    }
    Ok(components)
}

fn resolve_local_styles(template_path: &Path) -> Option<PathBuf> {
    let standard = template_path.with_extension("css");
    if standard.is_file() {
        return Some(standard);
    }
    resolve_fast_styles(template_path)
}

fn resolve_fast_styles(template_path: &Path) -> Option<PathBuf> {
    fast_style_candidates(template_path)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn fast_style_candidates(template_path: &Path) -> Vec<PathBuf> {
    let Some(file_name) = template_path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let Some(prefix) = file_name.strip_suffix(".template.html") else {
        return Vec::new();
    };
    let Some(parent) = template_path.parent() else {
        return Vec::new();
    };
    let mut candidates = Vec::with_capacity(2);
    for suffix in [".styles.css", ".css"] {
        candidates.push(parent.join(format!("{prefix}{suffix}")));
    }
    candidates
}

fn fast_template_candidates(
    root: &Path,
    module_path: &Path,
    declaration_name: &str,
) -> Vec<PathBuf> {
    let module = root.join(module_path);
    let parent = module.parent().unwrap_or(root);
    let module_stem = module.file_stem().and_then(|stem| stem.to_str());
    let declaration_stem = to_kebab_case(declaration_name);
    let mut candidates = Vec::with_capacity(2);
    for stem in module_stem
        .into_iter()
        .chain(std::iter::once(declaration_stem.as_str()))
    {
        candidates.push(parent.join(format!("{stem}.template.html")));
    }
    candidates
}

fn resolve_fast_template(
    root: &Path,
    module_path: &Path,
    declaration_name: &str,
) -> Result<PathBuf> {
    for candidate in fast_template_candidates(root, module_path, declaration_name) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("no sibling <component>.template.html file found")
}

fn to_kebab_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len() + 4);
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index != 0 {
                result.push('-');
            }
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}
