// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! FAST manifest naming and generated template/style layouts.

use super::DiscoveryPlugin;
use crate::npm::{
    package_component_declarations, package_export_path, read_optional_file, read_required_file,
    ComponentDeclaration, PackageContext,
};
use crate::{has_sibling_script, DiscoveredComponent};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

// Initial estimate for module, template, and style dependencies.
const FAST_CACHE_FILES_PER_COMPONENT: usize = 16;

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
        discover_local_templates(root, fast_local_tag)
    }

    fn supports_package(&self, package: PackageContext<'_>) -> Result<bool> {
        if package.manifest.get("customElements").is_some() {
            return Ok(true);
        }
        crate::catalog::has_templates_matching(&crate::catalog::root(package)?, ordinary_html)
    }

    fn package_cache_files(&self, package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
        let declarations = declarations(package)?;
        let names: HashSet<_> = declarations
            .iter()
            .map(|item| item.tag_name.as_str())
            .collect();
        let mut files =
            crate::catalog::cache_files_matching(&crate::catalog::root(package)?, |path| {
                fallback_html(path, &names)
            })?;
        if package.manifest.get("customElements").is_some() {
            files.push(crate::npm::custom_elements_manifest_path(package)?);
        }
        if declarations.is_empty() {
            return Ok(files);
        }
        let assets = exported_assets(package, declarations.len())?;
        let capacity = if assets.template.is_some() {
            4
        } else {
            1 + declarations.len() * FAST_CACHE_FILES_PER_COMPONENT
        };
        files.reserve(capacity);
        if let Some(template) = assets.template {
            if let Some(styles) = assets.styles {
                files.push(styles);
            } else {
                files.extend(fast_style_candidates(&template));
            }
            files.push(template);
            return Ok(files);
        }
        for declaration in declarations {
            let module_path = declaration.module_path.as_deref().with_context(|| {
                format!(
                    "FAST component <{}> in package '{}' has no CEM module path",
                    declaration.tag_name, package.name
                )
            })?;
            files.push(package_path(package.root, module_path).with_context(|| {
                format!(
                    "FAST component <{}> in package '{}' has an invalid CEM module path",
                    declaration.tag_name, package.name
                )
            })?);
            let declaration_name = declaration.name.as_deref().unwrap_or(&declaration.tag_name);
            for candidate in fast_template_candidates(package.root, module_path, declaration_name) {
                files.push(candidate.clone());
                if assets.styles.is_none() {
                    files.extend(fast_style_candidates(&candidate));
                }
            }
        }
        files.extend(assets.styles);
        Ok(files)
    }

    fn discover_package(&self, package: PackageContext<'_>) -> Result<Vec<DiscoveredComponent>> {
        let declarations = declarations(package)?;
        let mut components = {
            let names: HashSet<_> = declarations
                .iter()
                .map(|item| item.tag_name.as_str())
                .collect();
            crate::catalog::discover_matching(
                package.name,
                &crate::catalog::root(package)?,
                |path| fallback_html(path, &names),
            )?
        };
        if declarations.is_empty() {
            if components.is_empty() {
                bail!(
                    "No components found in package '{}'. Declare FAST components through \
                     customElements or provide <component-name>.html files.",
                    package.name
                );
            }
            return Ok(components);
        }

        let assets = exported_assets(package, declarations.len())?;
        components.reserve(declarations.len());
        let mut seen_templates = HashSet::with_capacity(declarations.len());
        for declaration in declarations {
            let template_path = match &assets.template {
                Some(path) => path.clone(),
                None => inferred_template(package, &declaration)?,
            };
            if !seen_templates.insert(template_path.clone()) {
                bail!(
                    "FAST template {} maps to multiple component declarations",
                    template_path.display()
                );
            }
            let html_content = read_required_file(&template_path, "FAST component template")?;
            let css_content = match &assets.styles {
                Some(path) => Some(read_required_file(path, "FAST component styles")?),
                None => read_optional_file(
                    resolve_fast_styles(&template_path).as_deref(),
                    "FAST component styles",
                )?,
            };
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

fn declarations(package: PackageContext<'_>) -> Result<Vec<ComponentDeclaration>> {
    if package.manifest.get("customElements").is_some() {
        package_component_declarations(package)
    } else {
        Ok(Vec::new())
    }
}

fn ordinary_html(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            !name.ends_with(".template.html") && !name.ends_with(".template-webui.html")
        })
}

fn fallback_html(path: &Path, names: &HashSet<&str>) -> bool {
    ordinary_html(path)
        && crate::catalog::template_tag(path).is_some_and(|name| !names.contains(name))
}

struct ExportedAssets {
    template: Option<PathBuf>,
    styles: Option<PathBuf>,
}

fn exported_assets(package: PackageContext<'_>, declarations: usize) -> Result<ExportedAssets> {
    let template = package_export_path(package, "./template.html")?;
    if template.is_some() && declarations != 1 {
        bail!(
            "Package '{}' exports one './template.html' but declares {declarations} components. \
             Use one component declaration or per-module .template.html files.",
            package.name
        );
    }
    let styles = if declarations == 1 {
        package_export_path(package, "./styles.css")?
    } else {
        None
    };
    Ok(ExportedAssets { template, styles })
}

fn inferred_template(
    package: PackageContext<'_>,
    declaration: &ComponentDeclaration,
) -> Result<PathBuf> {
    let module_path = declaration.module_path.as_deref().with_context(|| {
        format!(
            "FAST component <{}> in package '{}' has no CEM module path",
            declaration.tag_name, package.name
        )
    })?;
    let name = declaration.name.as_deref().unwrap_or(&declaration.tag_name);
    resolve_fast_template(package.root, module_path, name).with_context(|| {
        format!(
            "Failed to locate FAST template for <{}> in package '{}'",
            declaration.tag_name, package.name
        )
    })
}

fn discover_local_templates(
    root: &Path,
    tag_name_for_path: fn(&Path) -> Option<&str>,
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
        {
            continue;
        }
        let Some(tag_name) = tag_name_for_path(path) else {
            continue;
        };
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

fn fast_local_tag(path: &Path) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    if file_name.ends_with(".template-webui.html") {
        return None;
    }
    file_name
        .strip_suffix(".template.html")
        .or_else(|| crate::catalog::template_tag(path))
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
    let Some(module) = package_path(root, module_path) else {
        return Vec::new();
    };
    let parent = module.parent().unwrap_or(root);
    let module_stem = module.file_stem().and_then(|stem| stem.to_str());
    let declaration_stem = to_kebab_case(declaration_name);
    let mut candidates = Vec::with_capacity(5);
    for stem in module_stem
        .into_iter()
        .chain(std::iter::once(declaration_stem.as_str()))
    {
        push_template_candidate(&mut candidates, parent, stem);
    }

    // Some generated manifests expose a virtual public module path while the
    // package stores the implementation under a compact or base class noun.
    if !module.is_file() {
        let component_root = if parent == root {
            root
        } else {
            parent
                .parent()
                .filter(|candidate| candidate.starts_with(root))
                .unwrap_or(root)
        };
        push_nested_template_candidate(&mut candidates, component_root, declaration_stem.as_str());
        let compact_stem = declaration_stem.replace('-', "");
        push_nested_template_candidate(&mut candidates, component_root, &compact_stem);
        if let Some((_, suffix)) = declaration_stem.rsplit_once('-') {
            push_nested_template_candidate(&mut candidates, component_root, suffix);
        }
    }
    if !candidates.iter().any(|candidate| candidate.is_file()) {
        for directory in parent
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(root))
        {
            for stem in module_stem
                .into_iter()
                .chain(std::iter::once(declaration_stem.as_str()))
            {
                push_template_candidate(&mut candidates, directory, stem);
            }
        }
    }
    candidates
}

fn package_path(root: &Path, relative: &Path) -> Option<PathBuf> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return None;
    }
    Some(root.join(relative))
}

fn push_template_candidate(candidates: &mut Vec<PathBuf>, parent: &Path, stem: &str) {
    let candidate = parent.join(format!("{stem}.template.html"));
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn push_nested_template_candidate(candidates: &mut Vec<PathBuf>, root: &Path, stem: &str) {
    push_template_candidate(candidates, &root.join(stem), stem);
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

#[cfg(test)]
mod tests;
