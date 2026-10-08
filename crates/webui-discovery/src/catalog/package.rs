// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::npm::{validate_package_asset_path, PackageContext};
use crate::DiscoveredComponent;

pub(crate) struct PackageCatalog<'a> {
    package: PackageContext<'a>,
    roots: Vec<PathBuf>,
    explicitly_disabled: bool,
}

impl<'a> PackageCatalog<'a> {
    pub(crate) fn new(package: PackageContext<'a>) -> Result<Self> {
        let declared = declared_roots(package)?;
        let explicitly_disabled = declared.as_ref().is_some_and(Vec::is_empty);
        let roots = match declared {
            Some(roots) => roots,
            None => default_roots(package)?,
        };
        Ok(Self {
            package,
            roots,
            explicitly_disabled,
        })
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.explicitly_disabled
    }

    pub(crate) fn has_templates(&self, include: impl Fn(&Path) -> bool) -> Result<bool> {
        for root in &self.roots {
            if super::has_templates_matching(root, &include)
                .with_context(|| read_context(self.package.name, root))?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn cache_files(self, include: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
        let mut files = Vec::with_capacity(self.roots.len());
        for root in self.roots {
            super::append_cache_files(&root, &include, Some(self.package.root), &mut files)
                .with_context(|| read_context(self.package.name, &root))?;
            files.push(root);
        }
        Ok(files)
    }

    pub(crate) fn discover(
        &self,
        include: impl Fn(&Path) -> bool,
    ) -> Result<Vec<DiscoveredComponent>> {
        let mut components = Vec::new();
        for root in &self.roots {
            super::append_components(
                self.package.name,
                root,
                &include,
                Some(self.package.root),
                &mut components,
            )
            .with_context(|| read_context(self.package.name, root))?;
        }
        Ok(components)
    }
}

#[cold]
#[inline(never)]
fn read_context(package: &str, root: &Path) -> String {
    format!(
        "Cannot read component catalog '{}' in package '{}'. \
         help: Check catalog files and permissions; keep symlink targets inside the package.",
        root.display(),
        package
    )
}

fn declared_roots(package: PackageContext<'_>) -> Result<Option<Vec<PathBuf>>> {
    let manifest = package
        .manifest
        .as_object()
        .ok_or_else(|| invalid_metadata(package.name, "package.json must be an object"))?;
    let Some(webui) = manifest.get("webui") else {
        return Ok(None);
    };
    let webui = webui
        .as_object()
        .ok_or_else(|| invalid_metadata(package.name, "'webui' must be an object"))?;
    let Some(components) = webui.get("components") else {
        return Ok(None);
    };
    let components = components
        .as_array()
        .ok_or_else(|| invalid_metadata(package.name, "'components' must be an array"))?;
    let mut roots = Vec::with_capacity(components.len());
    for component in components {
        let relative = component
            .as_str()
            .ok_or_else(|| invalid_metadata(package.name, "each root must be a string"))?;
        roots.push(resolve_root(package, relative)?);
    }
    Ok(Some(roots))
}

fn default_roots(package: PackageContext<'_>) -> Result<Vec<PathBuf>> {
    let root = package.root.join("components");
    match fs::symlink_metadata(&root) {
        Ok(_) => Ok(vec![resolve_root(package, "components")?]),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "Cannot inspect default component catalog '{}'. \
                 help: Check directory permissions or set webui.components in package.json.",
                root.display()
            )
        }),
    }
}

fn resolve_root(package: PackageContext<'_>, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if relative.trim().is_empty()
        || relative.contains(['\\', ':'])
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid_metadata(package.name, relative));
    }
    let root = package.root.join(path);
    let metadata = fs::metadata(&root).with_context(|| {
        format!(
            "Cannot read webui.components root '{}' in package '{}'. \
             help: Create the declared directory and make it readable, or correct package.json.",
            root.display(),
            package.name
        )
    })?;
    if !metadata.is_dir() {
        return Err(invalid_metadata(
            package.name,
            &format!("'{relative}' must be a directory"),
        ));
    }
    validate_package_asset_path(package.root, &root, "webui.components").with_context(|| {
        format!(
            "Invalid webui.components root '{}' in package '{}'. \
             help: Keep every catalog root inside the package, including symlink targets.",
            relative, package.name
        )
    })?;
    Ok(root)
}

#[cold]
#[inline(never)]
fn invalid_metadata(package: &str, detail: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Invalid webui.components in package '{package}': {detail}. \
         help: Set package.json webui.components to an array of package-relative directory \
         paths without '..', for example [\"./src/components\"], or [] to disable ordinary HTML discovery."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        discover_source_with_plugin, DiscoveryPlugin, FastDiscoveryPlugin, WebUIDiscoveryPlugin,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    const PLUGINS: [&dyn DiscoveryPlugin; 2] = [&WebUIDiscoveryPlugin, &FastDiscoveryPlugin];

    fn write(root: &Path, name: &str, content: &str) -> TestResult {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn tags(root: &Path, plugin: &dyn DiscoveryPlugin) -> Result<Vec<String>> {
        Ok(
            discover_source_with_plugin("@fixture/catalog", root, plugin)?
                .components
                .into_iter()
                .map(|component| component.tag_name)
                .collect(),
        )
    }

    #[test]
    fn absent_metadata_scans_only_components_with_explicit_root_opt_in() -> TestResult {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(&package, "components/first-card.html", "<p>Components</p>")?;
        write(&package, "dist/published-card.html", "<p>Published</p>")?;
        write(&package, "flat-card.html", "<p>Flat</p>")?;
        write(&package, "src/components/source-card.html", "<p>Source</p>")?;
        write(&package, "src/index.ts", "export {};")?;
        for manifest in ["{}", r#"{"webui":{}}"#] {
            write(&package, "package.json", manifest)?;
            for plugin in PLUGINS {
                assert_eq!(tags(root.path(), plugin)?, ["first-card"]);
            }
        }
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./"]}}"#,
        )?;
        for plugin in PLUGINS {
            assert_eq!(
                tags(root.path(), plugin)?,
                ["first-card", "published-card", "flat-card", "source-card"]
            );
        }
        Ok(())
    }

    #[test]
    fn metadata_and_source_edits_invalidate_catalog_caches() -> TestResult {
        for plugin in PLUGINS {
            let root = tempfile::tempdir()?;
            let package = root.path().join("node_modules/@fixture/catalog");
            write(&package, "package.json", "{}")?;
            write(&package, "components/default-card.html", "<p>Default</p>")?;
            write(&package, "flat-card.html", "<p>Flat</p>")?;
            write(
                &package,
                "src/components/source-card.v2.html",
                "<p>Source</p>",
            )?;
            write(&package, "extra/extra-card.html", "<p>Extra</p>")?;
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
            write(
                &package,
                "package.json",
                r#"{"webui":{"components":["./src/components","./extra"]}}"#,
            )?;
            assert_eq!(tags(root.path(), plugin)?, ["source-card.v2", "extra-card"]);
            assert_eq!(tags(root.path(), plugin)?, ["source-card.v2", "extra-card"]);
            let source = package.join("src/components");
            write(&source, "source-card.v2.html", "<p>Edited</p>")?;
            write(&source, "source-card.v2.css", "p { color: blue; }")?;
            write(&source, "source-card.v2.ts", "export {};")?;
            let result = discover_source_with_plugin("@fixture/catalog", root.path(), plugin)?;
            assert_eq!(result.components[0].html_content, "<p>Edited</p>");
            assert_eq!(
                result.components[0].css_content.as_deref(),
                Some("p { color: blue; }")
            );
            assert!(result.components[0].is_client_owned);
            write(&source, "added-card.html", "<p>Added</p>")?;
            assert_eq!(
                tags(root.path(), plugin)?,
                ["added-card", "source-card.v2", "extra-card"]
            );
            fs::remove_file(source.join("added-card.html"))?;
            fs::remove_file(source.join("source-card.v2.ts"))?;
            assert!(
                !discover_source_with_plugin("@fixture/catalog", root.path(), plugin)?.components
                    [0]
                .is_client_owned
            );
            write(&package, "package.json", r#"{"webui":{"components":[]}}"#)?;
            assert!(tags(root.path(), plugin)?.is_empty());
            write(
                &package,
                "package.json",
                r#"{"webui":{"components":["./extra"]}}"#,
            )?;
            assert_eq!(tags(root.path(), plugin)?, ["extra-card"]);
            fs::remove_file(package.join("extra/extra-card.html"))?;
            assert!(tags(root.path(), plugin).is_err());
            fs::remove_dir(package.join("extra"))?;
            assert!(tags(root.path(), plugin).is_err());
            write(&package, "package.json", "{}")?;
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
        }
        Ok(())
    }

    #[test]
    fn missing_default_catalog_never_falls_back_and_tracks_creation_and_removal() -> TestResult {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(&package, "package.json", "{}")?;
        write(&package, "flat-card.html", "<p>Outside</p>")?;
        write(&package, "src/components/source-card.html", "<p>Source</p>")?;
        for plugin in PLUGINS {
            assert!(tags(root.path(), plugin).is_err());
            assert!(
                discover_source_with_plugin("@fixture", root.path(), plugin)?
                    .components
                    .is_empty()
            );
        }
        write(&package, "components/default-card.html", "<p>Default</p>")?;
        for plugin in PLUGINS {
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
        }
        fs::remove_file(package.join("components/default-card.html"))?;
        fs::remove_dir(package.join("components"))?;
        for plugin in PLUGINS {
            assert!(tags(root.path(), plugin).is_err());
        }
        write(&package, "components", "Not a directory")?;
        for plugin in PLUGINS {
            assert!(discover_source_with_plugin("@fixture", root.path(), plugin).is_err());
        }
        Ok(())
    }

    #[test]
    fn scope_support_checks_respect_disabled_and_empty_catalogs() -> TestResult {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./src/components"]}}"#,
        )?;
        fs::create_dir_all(package.join("src/components"))?;
        write(
            &package,
            "test_results/coverage/lcov-report/initial-state.ts.html",
            "<p>Report</p>",
        )?;
        for plugin in PLUGINS {
            assert!(
                discover_source_with_plugin("@fixture", root.path(), plugin)?
                    .components
                    .is_empty()
            );
            assert!(tags(root.path(), plugin).is_err());
        }
        write(&package, "src/components/real-card.html", "<p>Real</p>")?;
        for plugin in PLUGINS {
            let result = discover_source_with_plugin("@fixture/*", root.path(), plugin)?;
            assert_eq!(result.components.len(), 1);
            assert_eq!(result.components[0].tag_name, "real-card");
        }
        write(&package, "package.json", r#"{"webui":{"components":[]}}"#)?;
        for plugin in PLUGINS {
            assert!(
                discover_source_with_plugin("@fixture", root.path(), plugin)?
                    .components
                    .is_empty()
            );
            assert!(tags(root.path(), plugin)?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn invalid_root_metadata_never_falls_back_or_uses_a_warm_cache() -> TestResult {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(&package, "flat-card.html", "<p>Flat</p>")?;
        for plugin in PLUGINS {
            write(
                &package,
                "package.json",
                r#"{"webui":{"components":["./"]}}"#,
            )?;
            assert_eq!(tags(root.path(), plugin)?, ["flat-card"]);
            for manifest in [
                r#"{"webui":null}"#,
                r#"{"webui":[]}"#,
                r#"{"webui":{"components":null}}"#,
                r#"{"webui":{"components":"./src"}}"#,
                r#"{"webui":{"components":{}}}"#,
                r#"{"webui":{"components":[1]}}"#,
                r#"{"webui":{"components":[""]}}"#,
                r#"{"webui":{"components":[" "]}}"#,
                r#"{"webui":{"components":["../outside"]}}"#,
                r#"{"webui":{"components":["./src/../../outside"]}}"#,
                r#"{"webui":{"components":["/absolute"]}}"#,
                r#"{"webui":{"components":["C:/absolute"]}}"#,
                r#"{"webui":{"components":["C:\\absolute"]}}"#,
                r#"{"webui":{"components":["..\\outside"]}}"#,
                r#"{"webui":{"components":["./missing"]}}"#,
                r#"{"webui":{"components":["./flat-card.html"]}}"#,
                r#"{"webui":{"components":["./", "./missing"]}}"#,
            ] {
                write(&package, "package.json", manifest)?;
                let error = tags(root.path(), plugin).expect_err(manifest);
                let message = format!("{error:#}");
                assert!(
                    message.contains("webui.components"),
                    "{manifest}: {message}"
                );
                assert!(message.contains("help:"), "{manifest}: {message}");
                assert!(discover_source_with_plugin("@fixture", root.path(), plugin).is_err());
            }
        }
        Ok(())
    }

    #[test]
    fn explicit_local_roots_do_not_read_package_metadata() -> TestResult {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "package.json",
            r#"{"webui":{"components":["../invalid"]}}"#,
        )?;
        write(root.path(), "flat-card.html", "<p>Flat</p>")?;
        write(
            root.path(),
            "src/components/source-card.html",
            "<p>Source</p>",
        )?;
        for plugin in PLUGINS {
            let result = plugin.discover_local(root.path())?;
            let tags: Vec<_> = result
                .iter()
                .map(|component| component.tag_name.as_str())
                .collect();
            assert_eq!(tags, ["flat-card", "source-card"]);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn declared_roots_and_catalog_assets_cannot_escape_through_symlinks() -> TestResult {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./catalog"]}}"#,
        )?;
        write(root.path(), "outside/real-card.html", "<p>Outside</p>")?;
        symlink(root.path().join("outside"), package.join("catalog"))?;
        for plugin in PLUGINS {
            assert!(tags(root.path(), plugin).is_err());
        }
        fs::remove_file(package.join("catalog"))?;
        fs::create_dir(package.join("catalog"))?;
        for extension in ["html", "css", "ts", "js"] {
            write(root.path(), "outside/asset", "<p>Outside</p>")?;
            write(&package, "catalog/real-card.html", "<p>Real</p>")?;
            let asset = package.join(format!("catalog/real-card.{extension}"));
            if asset.exists() {
                fs::remove_file(&asset)?;
            }
            symlink(root.path().join("outside/asset"), &asset)?;
            for plugin in PLUGINS {
                assert!(tags(root.path(), plugin).is_err(), "{extension}");
            }
            fs::remove_file(asset)?;
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn internal_root_symlinks_work_and_retargeting_invalidates_the_cache() -> TestResult {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./catalog"]}}"#,
        )?;
        write(&package, "first/real-card.html", "<p>First</p>")?;
        write(&package, "second/real-card.html", "<p>Second</p>")?;
        for directory in ["first", "second"] {
            symlink(package.join(directory), package.join("catalog"))?;
            for plugin in PLUGINS {
                let result = discover_source_with_plugin("@fixture/catalog", root.path(), plugin)?;
                assert_eq!(result.components.len(), 1);
                assert_eq!(
                    result.components[0].html_content,
                    if directory == "first" {
                        "<p>First</p>"
                    } else {
                        "<p>Second</p>"
                    }
                );
            }
            fs::remove_file(package.join("catalog"))?;
        }
        for plugin in PLUGINS {
            assert!(tags(root.path(), plugin).is_err());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn file_links_within_the_package_preserve_templates_styles_and_ownership() -> TestResult {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./catalog"]}}"#,
        )?;
        write(&package, "assets/template.txt", "<p>Linked</p>")?;
        write(&package, "assets/styles.txt", "p { color: blue; }")?;
        write(&package, "assets/script.txt", "export {};")?;
        fs::create_dir(package.join("catalog"))?;
        for (extension, asset) in [("html", "template"), ("css", "styles"), ("js", "script")] {
            symlink(
                package.join(format!("assets/{asset}.txt")),
                package.join(format!("catalog/real-card.{extension}")),
            )?;
        }
        for plugin in PLUGINS {
            for _ in 0..2 {
                let result = discover_source_with_plugin("@fixture/catalog", root.path(), plugin)?;
                assert_eq!(result.components.len(), 1);
                assert_eq!(result.components[0].html_content, "<p>Linked</p>");
                assert_eq!(
                    result.components[0].css_content.as_deref(),
                    Some("p { color: blue; }")
                );
                assert!(result.components[0].is_client_owned);
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn link_validation_ignores_unowned_assets_but_checks_all_owned_scripts() -> TestResult {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./catalog"]}}"#,
        )?;
        write(&package, "catalog/real-card.html", "<p>Real</p>")?;
        write(&package, "catalog/real-card.ts", "export {};")?;
        write(
            root.path(),
            "outside/file.txt",
            "Must not read outside the package",
        )?;
        for file in ["orphan-card.css", "orphan-card.js", "real-card.spec.ts"] {
            symlink(
                root.path().join("outside/file.txt"),
                package.join("catalog").join(file),
            )?;
        }
        for plugin in PLUGINS {
            assert_eq!(tags(root.path(), plugin)?, ["real-card"]);
        }
        let manifest = serde_json::json!({"webui":{"components":["./catalog"]}});
        let context = PackageContext {
            name: "@fixture/catalog",
            root: &package,
            manifest: &manifest,
        };
        let script = package.join("catalog/real-card.js");
        symlink(root.path().join("outside/file.txt"), &script)?;
        for plugin in PLUGINS {
            assert!(plugin.package_cache_files(context).is_err());
            assert!(plugin.discover_package(context).is_err());
            assert!(tags(root.path(), plugin).is_err());
        }
        fs::remove_file(script)?;
        symlink(
            package.join("missing.css"),
            package.join("catalog/real-card.css"),
        )?;
        for plugin in PLUGINS {
            assert!(plugin.package_cache_files(context).is_err());
            assert!(plugin.discover_package(context).is_err());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_declared_root_is_an_error() -> TestResult {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        write(
            &package,
            "package.json",
            r#"{"webui":{"components":["./catalog"]}}"#,
        )?;
        write(&package, "catalog/real-card.html", "<p>Real</p>")?;
        let catalog = package.join("catalog");
        let permissions = fs::metadata(&catalog)?.permissions();
        fs::set_permissions(&catalog, fs::Permissions::from_mode(0))?;
        let can_read = fs::read_dir(&catalog).is_ok();
        let results: Vec<_> = PLUGINS
            .iter()
            .map(|plugin| tags(root.path(), *plugin))
            .collect();
        fs::set_permissions(catalog, permissions)?;
        if !can_read {
            for result in results {
                let error = result.expect_err("Unreadable catalog must fail");
                assert!(error.to_string().contains("help:"));
            }
        }
        Ok(())
    }
}
