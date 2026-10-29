// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Region scripts and the generated `lazy.webui.js`.
//!
//! Lazy-region packs carry templates and styles. The application's bundler
//! delivers code: for each region this resolves the module of every scripted
//! component the region's pack introduces and the page does not already load
//! at startup, then renders the loader module the application imports, so
//! those modules follow the bundler's splitting, public path, and caching.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use webui_handler::lazy_packs::{
    module_specifier, render_scripts_module, LazyPackFile, LazyScript, LAZY_SCRIPTS_DECLARATION,
    LAZY_SCRIPTS_DECLARATION_SOURCE, LAZY_SCRIPTS_MODULE,
};
use webui_parser::component_modules;
use webui_parser::{ComponentRegistry, Diagnostic};

use crate::WebUIError;

/// A generated ES module written to the generated-modules directory for the
/// application bundler, with TypeScript declarations written next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedModule {
    /// File name inside the generated-modules directory, such as `lazy.webui.js`.
    pub file_name: String,
    /// JavaScript module source.
    pub content: String,
    /// TypeScript declarations for the module, published beside it.
    pub declaration: GeneratedDeclaration,
}

/// TypeScript declarations for a [`GeneratedModule`], such as
/// `lazy.webui.d.ts`.
///
/// TypeScript resolves an import of the module to this file, so applications
/// type-check the import without `allowJs`. Declaration files are also exempt
/// from `rootDir`, so the generated-modules directory may lie outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDeclaration {
    /// File name inside the generated-modules directory, such as
    /// `lazy.webui.d.ts`.
    pub file_name: String,
    /// TypeScript declaration source.
    pub content: String,
}

impl GeneratedModule {
    /// [`LAZY_SCRIPTS_MODULE`] with `content`, and its declarations.
    pub(crate) fn lazy_scripts(content: String) -> Self {
        Self {
            file_name: LAZY_SCRIPTS_MODULE.to_owned(),
            content,
            declaration: GeneratedDeclaration {
                file_name: LAZY_SCRIPTS_DECLARATION.to_owned(),
                content: LAZY_SCRIPTS_DECLARATION_SOURCE.to_owned(),
            },
        }
    }
}

/// Modules of registered client-owned components, captured before the parser
/// is consumed. `None` marks client code without a known module.
pub(crate) type ComponentScripts = HashMap<String, Option<PathBuf>>;

/// Reject invalid module mappings before any build work.
pub(crate) fn validate_mappings(mappings: &BTreeMap<String, String>) -> Result<(), WebUIError> {
    for (tag, specifier) in mappings {
        component_modules::validate_component_module(tag, specifier)
            .map_err(WebUIError::ComponentModules)?;
    }
    Ok(())
}

/// Warn about mappings whose tags no registered component uses.
pub(crate) fn unknown_mapping_warnings(
    registry: &ComponentRegistry,
    mappings: &BTreeMap<String, String>,
    warnings: &mut Vec<Diagnostic>,
) {
    let mut known: Option<Vec<&str>> = None;
    for (tag, specifier) in mappings {
        if registry.contains(tag) {
            continue;
        }
        let names = known.get_or_insert_with(|| {
            let mut names: Vec<&str> = registry.names().collect();
            names.sort_unstable();
            names
        });
        warnings.push(component_modules::unknown_component_module(
            tag,
            specifier,
            names.iter().copied(),
        ));
    }
}

/// Capture the module of every client-owned component.
pub(crate) fn capture(registry: &ComponentRegistry) -> ComponentScripts {
    registry
        .get_all()
        .filter(|component| component.is_client_owned || component.script_path.is_some())
        .map(|component| (component.tag_name.clone(), component.script_path.clone()))
        .collect()
}

/// Inputs that decide each scripted component's module specifier.
pub(crate) struct ScriptSources<'a> {
    /// Modules of client-owned components from discovery.
    pub(crate) scripts: &'a ComponentScripts,
    /// Explicit tag-to-specifier mappings, which win over discovered modules.
    pub(crate) mappings: &'a BTreeMap<String, String>,
    /// Directory that receives `lazy.webui.js`; relative specifiers start here.
    pub(crate) modules_dir: &'a Path,
    /// Components the entry page loads at startup, from
    /// [`webui_handler::lazy_packs::startup_components`]. Regions never list
    /// them: their code is already loaded.
    pub(crate) startup: &'a HashSet<&'a str>,
}

/// Fill each pack's scripts and render `lazy.webui.js` with its declarations.
///
/// Returns `None` when the build has no lazy regions.
///
/// # Errors
///
/// Returns [`WebUIError::ComponentModules`] when a discovered module has no
/// path relative to the generated-modules directory, and [`WebUIError::Io`]
/// when a path cannot be made absolute.
pub(crate) fn resolve(
    packs: &mut [LazyPackFile],
    sources: &ScriptSources<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Option<GeneratedModule>, WebUIError> {
    if packs.is_empty() {
        return Ok(None);
    }
    let modules_dir = absolute(sources.modules_dir)?;
    let resolved = region_scripts(packs, sources, &modules_dir, warnings)?;
    for (pack, scripts) in packs.iter_mut().zip(resolved) {
        pack.scripts = scripts;
    }
    Ok(render_scripts_module(packs).map(GeneratedModule::lazy_scripts))
}

/// Resolve the scripts of every pack, in pack order.
fn region_scripts(
    packs: &[LazyPackFile],
    sources: &ScriptSources<'_>,
    modules_dir: &Path,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Vec<Vec<LazyScript>>, WebUIError> {
    // Regions often share components, so each tag resolves once.
    let mut specifiers: HashMap<&str, Option<String>> = HashMap::new();
    let mut missing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut resolved = Vec::with_capacity(packs.len());
    for pack in packs {
        let mut scripts = Vec::new();
        for tag in &pack.components {
            if sources.startup.contains(tag.as_str()) {
                continue;
            }
            let specifier = match specifiers.get(tag.as_str()) {
                Some(known) => known.clone(),
                None => {
                    let found = specifier(tag, sources, modules_dir)?;
                    specifiers.insert(tag.as_str(), found.clone());
                    found
                }
            };
            match specifier {
                Some(specifier) => scripts.push(LazyScript {
                    tag: tag.clone(),
                    specifier,
                }),
                None if sources.scripts.contains_key(tag.as_str()) => {
                    missing
                        .entry(tag.as_str())
                        .or_default()
                        .push(pack.region.as_str());
                }
                None => {}
            }
        }
        resolved.push(scripts);
    }
    for (tag, mut regions) in missing {
        regions.sort_unstable();
        warnings.push(component_modules::lazy_component_without_module(
            tag, &regions,
        ));
    }
    Ok(resolved)
}

/// A component's module specifier: its mapping, else its discovered module
/// relative to the generated-modules directory. `None` without either.
fn specifier(
    tag: &str,
    sources: &ScriptSources<'_>,
    modules_dir: &Path,
) -> Result<Option<String>, WebUIError> {
    if let Some(specifier) = sources.mappings.get(tag) {
        return Ok(Some(specifier.clone()));
    }
    let Some(Some(path)) = sources.scripts.get(tag) else {
        return Ok(None);
    };
    let path = absolute(path)?;
    module_specifier(modules_dir, &path)
        .map(Some)
        .ok_or_else(|| unreachable_module(tag, &path, modules_dir))
}

fn absolute(path: &Path) -> Result<PathBuf, WebUIError> {
    std::path::absolute(path).map_err(|source| WebUIError::Io {
        context: format!("Failed to resolve {}", path.display()),
        source,
    })
}

#[cold]
#[inline(never)]
fn unreachable_module(tag: &str, module: &Path, modules_dir: &Path) -> WebUIError {
    WebUIError::ComponentModules(Box::new(component_modules::unreachable_component_module(
        tag,
        module,
        modules_dir,
    )))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_handler::lazy_packs::LazyTrigger;
    use webui_parser::codes;

    fn pack(region: &str, components: &[&str]) -> LazyPackFile {
        LazyPackFile {
            file_name: format!("{}.0123456789abcdef.json", region.replace(':', "_")),
            content: "{}".to_owned(),
            region: region.to_owned(),
            owner: "app-shell".to_owned(),
            trigger: LazyTrigger::Intent,
            components: components.iter().map(|tag| (*tag).to_owned()).collect(),
            scripts: Vec::new(),
            brotli_bytes: None,
        }
    }

    fn scripts(root: &Path) -> ComponentScripts {
        HashMap::from([
            (
                "settings-panel".to_owned(),
                Some(
                    root.join("src")
                        .join("settings-panel")
                        .join("settings-panel.ts"),
                ),
            ),
            ("vendor-picker".to_owned(), None),
            (
                "theme-picker".to_owned(),
                Some(root.join("src").join("theme-picker.js")),
            ),
        ])
    }

    #[test]
    fn mappings_override_discovered_modules_and_unknown_modules_warn_once() {
        let root = std::env::temp_dir().join("webui-lazy-scripts");
        let scripts = scripts(&root);
        let mappings = BTreeMap::from([(
            "theme-picker".to_owned(),
            "@contoso/themes/picker.js".to_owned(),
        )]);
        let mut packs = [
            pack(
                "app-shell::lazy:@1",
                &["plain-card", "theme-picker", "vendor-picker"],
            ),
            pack("app-shell::lazy:@0", &["settings-panel", "vendor-picker"]),
        ];
        let mut warnings = Vec::new();
        let module = resolve(
            &mut packs,
            &ScriptSources {
                scripts: &scripts,
                mappings: &mappings,
                modules_dir: &root.join("dist"),
                startup: &HashSet::new(),
            },
            &mut warnings,
        )
        .unwrap()
        .unwrap();

        assert_eq!(module.file_name, "lazy.webui.js");
        assert_eq!(module.declaration.file_name, "lazy.webui.d.ts");
        assert_eq!(
            module.declaration.content,
            "// Generated by WebUI. Do not edit.\nimport '@microsoft/webui-framework/lazy.js';\n"
        );
        assert_eq!(
            packs[1].scripts,
            [LazyScript {
                tag: "settings-panel".to_owned(),
                specifier: "../src/settings-panel/settings-panel.ts".to_owned(),
            }]
        );
        assert_eq!(
            packs[0].scripts,
            [LazyScript {
                tag: "theme-picker".to_owned(),
                specifier: "@contoso/themes/picker.js".to_owned(),
            }]
        );
        assert!(module.content.contains(
            r#""settings-panel": () => import("../src/settings-panel/settings-panel.ts"),"#
        ));
        assert!(module
            .content
            .contains(r#""theme-picker": () => import("@contoso/themes/picker.js"),"#));
        assert!(!module.content.contains("plain-card"));
        assert!(!module.content.contains("vendor-picker"));

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let warning = &warnings[0];
        assert_eq!(
            warning.error_code(),
            Some(codes::LAZY_COMPONENT_WITHOUT_MODULE)
        );
        assert_eq!(warning.severity(), webui_parser::Severity::Warning);
        assert!(
            warning
                .title()
                .contains("app-shell::lazy:@0, app-shell::lazy:@1"),
            "{}",
            warning.title()
        );
        assert!(warning
            .help_text()
            .unwrap()
            .contains("--component-module vendor-picker=<specifier>"));
    }

    #[test]
    fn components_the_page_loads_at_startup_are_never_region_scripts() {
        let root = std::env::temp_dir().join("webui-lazy-scripts");
        let scripts = scripts(&root);
        let mappings = BTreeMap::new();
        // Neither a known module nor a missing one matters at startup.
        let startup = HashSet::from(["settings-panel", "vendor-picker"]);
        let mut packs = [pack(
            "app-shell::lazy:@0",
            &["settings-panel", "theme-picker", "vendor-picker"],
        )];
        let mut warnings = Vec::new();
        let module = resolve(
            &mut packs,
            &ScriptSources {
                scripts: &scripts,
                mappings: &mappings,
                modules_dir: &root.join("dist"),
                startup: &startup,
            },
            &mut warnings,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            packs[0].scripts,
            [LazyScript {
                tag: "theme-picker".to_owned(),
                specifier: "../src/theme-picker.js".to_owned(),
            }]
        );
        assert!(
            !module.content.contains("settings-panel"),
            "{}",
            module.content
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            packs[0].components,
            ["settings-panel", "theme-picker", "vendor-picker"],
            "packs still carry startup templates"
        );
    }

    #[test]
    fn builds_without_lazy_regions_have_no_module() {
        let scripts = ComponentScripts::new();
        let mappings = BTreeMap::new();
        let mut warnings = Vec::new();
        let module = resolve(
            &mut [],
            &ScriptSources {
                scripts: &scripts,
                mappings: &mappings,
                modules_dir: Path::new("."),
                startup: &HashSet::new(),
            },
            &mut warnings,
        )
        .unwrap();
        assert_eq!(module, None);
        assert!(warnings.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn modules_on_another_drive_need_a_mapping_or_another_output_directory() {
        let scripts = HashMap::from([(
            "settings-panel".to_owned(),
            Some(PathBuf::from(r"Q:\packages\settings-panel.js")),
        )]);
        let mappings = BTreeMap::new();
        let mut packs = [pack("app-shell::lazy:@0", &["settings-panel"])];
        let error = resolve(
            &mut packs,
            &ScriptSources {
                scripts: &scripts,
                mappings: &mappings,
                modules_dir: Path::new(r"R:\app\dist"),
                startup: &HashSet::new(),
            },
            &mut Vec::new(),
        )
        .unwrap_err();
        let WebUIError::ComponentModules(diagnostic) = error else {
            panic!("expected a component module error, got {error}");
        };
        assert_eq!(
            diagnostic.error_code(),
            Some(codes::UNREACHABLE_COMPONENT_MODULE)
        );
        assert!(diagnostic.help_text().unwrap().contains("--modules-out"));
    }
}
