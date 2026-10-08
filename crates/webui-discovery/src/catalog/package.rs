// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::npm::{validate_package_asset_path, PackageContext};
use crate::ComponentFileSource;

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
            if super::has_templates_matching(root, &include, self.package.root)
                .with_context(|| read_context(self.package.name, root))?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn prepare(
        &self,
        include: impl Fn(&Path) -> bool,
    ) -> Result<Vec<ComponentFileSource>> {
        let mut components = Vec::new();
        let boundary: Arc<Path> = self.package.root.into();
        for root in &self.roots {
            super::prepare_components(root, &include, &boundary, &mut components)
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

    fn fixture(manifest: &str) -> Result<(tempfile::TempDir, PathBuf)> {
        let root = tempfile::tempdir()?;
        let package = root.path().join("node_modules/@fixture/catalog");
        fs::create_dir_all(&package)?;
        fs::write(package.join("package.json"), manifest)?;
        Ok((root, package))
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
        let (root, package) = fixture("{}")?;
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
    fn metadata_edits_invalidate_catalog_caches() -> TestResult {
        let (root, package) = fixture("{}")?;
        write(&package, "components/default-card.html", "<p>Default</p>")?;
        write(&package, "src/components/source-card.html", "<p>Source</p>")?;
        write(&package, "extra/extra-card.html", "<p>Extra</p>")?;
        let cases: &[(&str, &[&str])] = &[
            ("{}", &["default-card"]),
            (
                r#"{"webui":{"components":["./src/components","./extra"]}}"#,
                &["source-card", "extra-card"],
            ),
            (r#"{"webui":{"components":[]}}"#, &[]),
            (r#"{"webui":{"components":["./extra"]}}"#, &["extra-card"]),
            ("{}", &["default-card"]),
        ];
        for plugin in PLUGINS {
            for &(manifest, expected) in cases {
                write(&package, "package.json", manifest)?;
                for _ in 0..2 {
                    assert_eq!(tags(root.path(), plugin)?, expected);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn missing_empty_and_disabled_catalogs_have_distinct_scope_behavior() -> TestResult {
        let (root, package) = fixture("{}")?;
        write(&package, "flat-card.html", "<p>Outside</p>")?;
        write(&package, "src/components/source-card.html", "<p>Source</p>")?;
        fs::create_dir(package.join("empty"))?;
        for manifest in ["{}", r#"{"webui":{"components":["./empty"]}}"#] {
            write(&package, "package.json", manifest)?;
            for plugin in PLUGINS {
                assert!(tags(root.path(), plugin).is_err());
                assert!(
                    discover_source_with_plugin("@fixture", root.path(), plugin)?
                        .components
                        .is_empty()
                );
            }
        }
        write(&package, "package.json", "{}")?;
        write(&package, "components/default-card.html", "<p>Default</p>")?;
        for plugin in PLUGINS {
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
            assert_eq!(tags(root.path(), plugin)?, ["default-card"]);
            assert_eq!(
                discover_source_with_plugin("@fixture/*", root.path(), plugin)?
                    .components
                    .len(),
                1
            );
        }
        fs::remove_file(package.join("components/default-card.html"))?;
        fs::remove_dir(package.join("components"))?;
        for plugin in PLUGINS {
            assert!(tags(root.path(), plugin).is_err());
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
        write(&package, "package.json", "{}")?;
        write(&package, "components", "Not a directory")?;
        for plugin in PLUGINS {
            assert!(discover_source_with_plugin("@fixture", root.path(), plugin).is_err());
        }
        Ok(())
    }

    #[test]
    fn invalid_root_metadata_never_falls_back_or_uses_a_warm_cache() -> TestResult {
        let (root, package) = fixture("{}")?;
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
                r#"{"webui":{"components":null}}"#,
                r#"{"webui":{"components":"./src"}}"#,
                r#"{"webui":{"components":[1]}}"#,
                r#"{"webui":{"components":[""]}}"#,
                r#"{"webui":{"components":["../outside"]}}"#,
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
    fn dangling_template_links_fail_named_and_scoped_discovery() -> TestResult {
        use std::os::unix::fs::symlink;
        let (root, package) = fixture("{}")?;
        fs::create_dir(package.join("components"))?;
        symlink("missing.html", package.join("components/broken-card.html"))?;
        symlink("missing.html", package.join("components/index.html"))?;
        for with_valid_component in [false, true] {
            if with_valid_component {
                write(&package, "components/real-card.html", "<p>Real</p>")?;
            }
            for plugin in PLUGINS {
                for source in ["@fixture/catalog", "@fixture"] {
                    let error = discover_source_with_plugin(source, root.path(), plugin)
                        .expect_err("A qualifying template link must not disappear");
                    assert!(
                        format!("{error:#}").contains("broken-card.html"),
                        "{error:#}"
                    );
                }
            }
        }
        fs::remove_file(package.join("components/broken-card.html"))?;
        for plugin in PLUGINS {
            assert_eq!(tags(root.path(), plugin)?, ["real-card"]);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn declared_roots_and_catalog_assets_cannot_escape_through_symlinks() -> TestResult {
        use std::os::unix::fs::symlink;
        let (root, package) = fixture(r#"{"webui":{"components":["./catalog"]}}"#)?;
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
    fn internal_root_and_file_links_work_and_retargeting_invalidates_cache() -> TestResult {
        use std::os::unix::fs::symlink;
        let (root, package) = fixture(r#"{"webui":{"components":["./catalog"]}}"#)?;
        write(&package, "assets/styles.txt", "p { color: blue; }")?;
        write(&package, "assets/script.txt", "export {};")?;
        for (directory, html) in [("first", "<p>First</p>"), ("second", "<p>Second</p>")] {
            write(&package, &format!("assets/{directory}.txt"), html)?;
            fs::create_dir(package.join(directory))?;
            for (extension, asset) in [("html", directory), ("css", "styles"), ("js", "script")] {
                symlink(
                    package.join(format!("assets/{asset}.txt")),
                    package
                        .join(directory)
                        .join(format!("real-card.{extension}")),
                )?;
            }
            symlink(package.join(directory), package.join("catalog"))?;
            for plugin in PLUGINS {
                for _ in 0..2 {
                    let result =
                        discover_source_with_plugin("@fixture/catalog", root.path(), plugin)?;
                    assert_eq!(result.components.len(), 1);
                    assert_eq!(result.components[0].html_content, html);
                    assert_eq!(
                        result.components[0].css_content.as_deref(),
                        Some("p { color: blue; }")
                    );
                    assert!(result.components[0].is_client_owned);
                }
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
    fn link_validation_ignores_unowned_assets_but_checks_all_owned_scripts() -> TestResult {
        use std::os::unix::fs::symlink;
        let (root, package) = fixture(r#"{"webui":{"components":["./catalog"]}}"#)?;
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
        let canonical_package = package.canonicalize()?;
        let context = PackageContext {
            name: "@fixture/catalog",
            root: &canonical_package,
            manifest: &manifest,
        };
        let script = package.join("catalog/real-card.js");
        symlink(root.path().join("outside/file.txt"), &script)?;
        for plugin in PLUGINS {
            assert!(plugin.prepare_package(context).is_err());
            assert!(tags(root.path(), plugin).is_err());
        }
        fs::remove_file(script)?;
        symlink(
            package.join("missing.css"),
            package.join("catalog/real-card.css"),
        )?;
        for plugin in PLUGINS {
            assert!(plugin.prepare_package(context).is_err());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_declared_root_is_an_error() -> TestResult {
        use std::os::unix::fs::PermissionsExt;
        let (root, package) = fixture(r#"{"webui":{"components":["./catalog"]}}"#)?;
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
