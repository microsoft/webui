// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::resolve::Compiler;
use crate::{
    diagnostic, ComponentSurface, ProjectionDiagnostic, MAX_MODULES, MAX_MODULE_BYTES,
    MAX_SOURCE_BYTES,
};

/// Analyze each compiler-selected component module independently.
///
/// The source contract recognizes the canonical WebUI framework import and
/// unambiguous, explicitly extended relative imports. No application code is
/// executed or transformed. A failed proof yields a diagnostic for that tag,
/// without invalidating the exact surfaces of other tags.
pub fn analyze_sources<'a>(
    app_dir: &Path,
    components: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> BTreeMap<&'a str, Result<ComponentSurface, ProjectionDiagnostic>> {
    let root = app_dir.canonicalize();
    let mut compiler = Compiler::source(SourceEvidence::default());
    components
        .into_iter()
        .map(|(tag, path)| {
            let surface = root
                .as_deref()
                .map_err(|error| {
                    diagnostic(
                        &app_dir.to_string_lossy(),
                        "PROJ-C006",
                        format!("Use a readable application source directory: {error}"),
                    )
                })
                .and_then(|root| selected_source(root, path))
                .and_then(|(scope, module)| compiler.source_component(scope, &module, tag));
            (tag, surface)
        })
        .collect()
}

#[derive(Default)]
pub(super) struct SourceEvidence {
    pub(super) scope: PathBuf,
    bytes_read: usize,
}

impl SourceEvidence {
    pub(super) fn relative_import(&self, module: &str, specifier: &str) -> Option<String> {
        if !(specifier.starts_with("./") || specifier.starts_with("../"))
            || specifier.contains(['\\', ':', '?', '#'])
        {
            return None;
        }
        let path = local_path(Path::new(module).parent()?, specifier, &self.scope)?;
        let ext = path.extension()?.to_str()?;
        let file_name = path.file_name()?.to_str()?;
        if file_name.ends_with(".d.ts") || file_name.ends_with(".d.js") {
            return None;
        }
        let resolved = match ext {
            "js" => match (
                physical_file(&path)?,
                physical_file(&path.with_extension("ts"))?,
            ) {
                (Some(path), None) | (None, Some(path)) => path,
                _ => return None,
            },
            "ts" => physical_file(&path)??,
            _ => return None,
        };
        resolved.into_os_string().into_string().ok()
    }

    pub(super) fn read(
        &mut self,
        module: &str,
        attempts: usize,
    ) -> Result<String, ProjectionDiagnostic> {
        let limit = MAX_MODULE_BYTES.min(MAX_SOURCE_BYTES - self.bytes_read);
        if attempts > MAX_MODULES || limit == 0 {
            return Err(source_limit(module));
        }
        let mut source = String::new();
        let file = File::open(module).map_err(|error| unreadable_source(module, &error))?;
        if file
            .metadata()
            .map_err(|error| unreadable_source(module, &error))?
            .len()
            > limit as u64
        {
            return Err(source_limit(module));
        }
        file.take(limit as u64 + 1)
            .read_to_string(&mut source)
            .map_err(|error| unreadable_source(module, &error))?;
        if source.len() > limit {
            return Err(source_limit(module));
        }
        self.bytes_read += source.len();
        Ok(source)
    }
}

fn selected_source(
    root: &Path,
    selected: &Path,
) -> Result<(PathBuf, String), ProjectionDiagnostic> {
    let module = || {
        let parent = selected.parent()?.canonicalize().ok()?;
        let path = parent.join(selected.file_name()?);
        let alternate = match path.extension()?.to_str()? {
            "ts" => "js",
            "js" => "ts",
            _ => return None,
        };
        if physical_file(&path.with_extension(alternate))?.is_some() {
            return None;
        }
        let physical = physical_file(&path)??;
        let scope = if physical.starts_with(root) {
            root.to_path_buf()
        } else {
            parent
        };
        Some((scope, physical.into_os_string().into_string().ok()?))
    };
    module().ok_or_else(|| {
        diagnostic(
            &selected.to_string_lossy(),
            "PROJ-C006",
            "Use a readable same-named .ts or .js module, not both, without a file symlink.",
        )
    })
}

fn local_path(parent: &Path, specifier: &str, scope: &Path) -> Option<PathBuf> {
    let mut path = parent.to_path_buf();
    let mut descending = false;
    for part in Path::new(specifier).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if !descending && path != scope => {
                path.pop();
            }
            Component::Normal(name) => {
                descending = true;
                path.push(name);
            }
            _ => return None,
        }
    }
    path.starts_with(scope).then_some(path)
}

// Distinguish absent files from existing but unprovable paths; the latter may
// be the one the external bundler chose, so never substitute another source.
fn physical_file(path: &Path) -> Option<Option<PathBuf>> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => {
            let physical = path.canonicalize().ok()?;
            (physical == path).then_some(Some(physical))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        _ => None,
    }
}

#[cold]
#[inline(never)]
fn unreadable_source(module: &str, error: &std::io::Error) -> ProjectionDiagnostic {
    diagnostic(
        module,
        "PROJ-C006",
        format!("Use a readable UTF-8 source module: {error}"),
    )
}

#[cold]
#[inline(never)]
fn source_limit(module: &str) -> ProjectionDiagnostic {
    diagnostic(
        module,
        "PROJ-C013",
        "Keep analyzed source below 65,535 modules, 16 MiB per file and 64 MiB total.",
    )
}
