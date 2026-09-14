// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{hash_map::DefaultHasher, BTreeMap, HashSet};
use std::fs::{self, File, Metadata};
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use super::super::RenderConfig;

const BUFFER_SIZE: usize = 8 * 1024;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 16 * 1024;

// Immutable SSR dependencies for one server configuration, not client watch roots.
pub(super) struct Inputs {
    roots: Vec<PathBuf>,
    files: Vec<PathBuf>,
    theme: Option<String>,
    output: PathBuf,
    config: u64,
    cacheable: bool,
}

// A complete capture; never retain a capture from a failed preparation.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    config: u64,
    entries: BTreeMap<PathBuf, Fingerprint>,
}

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    target: Option<PathBuf>,
    directory: bool,
    content: Option<u64>,
}

impl Inputs {
    // Retain input spellings so each capture resolves workspace symlinks anew.
    pub(super) fn new(config: &RenderConfig, theme: Option<&str>, output: &Path) -> Result<Self> {
        let app = absolute(&config.app_dir)?;
        let mut roots = Vec::with_capacity(config.app_args.components.len() + 1);
        roots.push(app.clone());
        let mut cacheable = config.app_args.projection_manifests.is_empty()
            && matches!(config.app_args.plugin, None | Some(webui::Plugin::WebUI));
        for source in &config.app_args.components {
            if webui_discovery::is_local_source(source) {
                roots.push(absolute(&app.join(source))?);
            } else {
                // npm resolution and FAST/projection closures can include inputs
                // outside these roots. Do not substitute an incomplete fingerprint.
                cacheable = false;
            }
        }
        let mut files = Vec::with_capacity(config.app_args.projection_manifests.len() + 2);
        files.push(absolute(&app.join(&config.app_args.entry))?);
        if let Some(state) = &config.state_file {
            files.push(absolute(state)?);
        }
        for manifest in &config.app_args.projection_manifests {
            files.push(absolute(manifest)?);
        }
        let output = absolute(output)?;
        Ok(Self {
            roots,
            files,
            theme: theme.map(str::to_owned),
            output,
            config: config_hash(config, theme),
            cacheable,
        })
    }

    // None requires full preparation. Retain captures only with a successful,
    // stable publication; unknown/read-failed inputs must never permit reuse.
    pub(super) fn capture(&self) -> Result<Option<Snapshot>> {
        if !self.cacheable {
            return Ok(None);
        }
        match self.capture_complete() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(error) => {
                log::debug!("SSR input cache unavailable: {error:#}; running full preparation");
                Ok(None)
            }
        }
    }

    fn capture_complete(&self) -> Result<Snapshot> {
        let output = resolve(&self.output)?;
        ensure!(
            !resolve(&self.files[0])?.starts_with(&output),
            "SSR entry is inside --servedir; move the entry outside generated output"
        );
        let mut capture = Capture {
            entries: BTreeMap::new(),
            directories: HashSet::new(),
            buffer: [0; BUFFER_SIZE],
        };
        for file in &self.files {
            capture.file(file, true)?;
        }
        if let Some(theme) = &self.theme {
            // Resolution must be repeated: a generated file or nearer package
            // installation can change which theme the renderer will load.
            let path = webui::resolve_theme_path(theme, &self.roots[0])
                .with_context(|| format!("Cannot resolve SSR theme '{theme}'; check --theme"))?;
            capture.file(&path, true)?;
        }
        for root in &self.roots {
            let resolved = resolve(root)?;
            ensure!(
                !resolved.starts_with(&output),
                "SSR root {} is inside --servedir; move the input outside generated output",
                root.display()
            );
            ensure!(
                !discovery_can_reach(&resolved, &output),
                "--servedir is discoverable beneath SSR root {}; use a sibling output \
                 directory or a node_modules/dot-directory subtree",
                root.display()
            );
            capture.tree(&resolved, &output)?;
        }
        Ok(Snapshot {
            config: self.config,
            entries: capture.entries,
        })
    }
}

struct Capture {
    entries: BTreeMap<PathBuf, Fingerprint>,
    directories: HashSet<PathBuf>,
    buffer: [u8; BUFFER_SIZE],
}

impl Capture {
    fn tree(&mut self, root: &Path, output: &Path) -> Result<()> {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            // Check lexical and resolved directory paths, never ancestor names:
            // explicitly supplied roots under target/ remain real inputs.
            if path.starts_with(output) {
                continue;
            }
            let metadata = inspect(&path)?;
            if metadata.is_dir() {
                if path != root && ignored_directory(&path) {
                    continue;
                }
                let target = resolve(&path)?;
                ensure!(
                    !target.starts_with(output),
                    "SSR directory {} links into --servedir; move inputs outside output",
                    path.display()
                );
                self.check_capacity()?;
                let alias = (target != path).then(|| target.clone());
                let visited = !self.directories.insert(target);
                self.entries.insert(
                    path.clone(),
                    Fingerprint {
                        target: alias,
                        directory: true,
                        content: None,
                    },
                );
                if !visited {
                    self.children(&path, &mut pending)?;
                }
            } else {
                self.file(&path, !is_script(&path))?;
            }
        }
        Ok(())
    }

    fn children(&self, directory: &Path, pending: &mut Vec<PathBuf>) -> Result<()> {
        let start = pending.len();
        let entries = fs::read_dir(directory).with_context(|| {
            format!(
                "Cannot enumerate SSR directory {}; check read permissions",
                directory.display()
            )
        })?;
        for entry in entries {
            ensure!(
                self.entries.len() + pending.len() < MAX_ENTRIES,
                "SSR input tree exceeds {MAX_ENTRIES} entries; narrow the app/component roots"
            );
            pending.push(
                entry
                    .with_context(|| format!("Cannot enumerate SSR input {}", directory.display()))?
                    .path(),
            );
        }
        // Explicit DFS stack with deterministic sibling order and bounded growth.
        pending[start..].sort_unstable_by(|left, right| right.cmp(left));
        Ok(())
    }

    fn file(&mut self, path: &Path, include_content: bool) -> Result<()> {
        if self.entries.contains_key(path) {
            return Ok(());
        }
        self.check_capacity()?;
        let metadata = inspect(path)?;
        ensure!(
            metadata.is_file(),
            "SSR input {} must be a regular file",
            path.display()
        );
        let target = resolve(path)?;
        let content = if include_content {
            Some(self.read_content(path, &metadata)?)
        } else {
            // Standard discovery only checks sibling-script presence. Its bytes,
            // size, and read permissions cannot change compiled SSR.
            None
        };
        let (key, target) = if target == path {
            (target, None)
        } else {
            (path.to_path_buf(), Some(target))
        };
        self.entries.insert(
            key,
            Fingerprint {
                target,
                directory: false,
                content,
            },
        );
        Ok(())
    }

    fn read_content(&mut self, path: &Path, metadata: &Metadata) -> Result<u64> {
        check_file(metadata, path)?;
        let mut file = File::open(path).with_context(|| {
            format!(
                "Cannot open SSR input {}; check read permissions",
                path.display()
            )
        })?;
        check_file(&file.metadata()?, path)?;
        hash_contents(&mut file, &mut self.buffer)
            .with_context(|| format!("Cannot read SSR input {}; check this file", path.display()))
    }

    fn check_capacity(&self) -> Result<()> {
        ensure!(
            self.entries.len() < MAX_ENTRIES,
            "SSR input tree exceeds {MAX_ENTRIES} entries; narrow the app/component roots"
        );
        Ok(())
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| {
        format!(
            "Cannot locate SSR input {}; provide an absolute path",
            path.display()
        )
    })
}

fn resolve(path: &Path) -> Result<PathBuf> {
    path.canonicalize().with_context(|| {
        format!(
            "Cannot resolve SSR input {}; check that it exists",
            path.display()
        )
    })
}

fn inspect(path: &Path) -> Result<Metadata> {
    fs::metadata(path).with_context(|| {
        format!(
            "Cannot inspect SSR input {}; check that it is accessible",
            path.display()
        )
    })
}

fn check_file(metadata: &Metadata, path: &Path) -> Result<()> {
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES as u64,
        "SSR input {} must be a readable regular file no larger than 8 MiB",
        path.display()
    );
    Ok(())
}

fn ignored_directory(path: &Path) -> bool {
    path.file_name().is_some_and(ignored_name)
}

fn ignored_name(name: &std::ffi::OsStr) -> bool {
    name == "node_modules" || name.to_string_lossy().starts_with('.')
}

fn discovery_can_reach(root: &Path, output: &Path) -> bool {
    // The SDK excludes node_modules and dot names, but not dist/target or an
    // arbitrary configured output. Only names below this explicit root count.
    output.strip_prefix(root).is_ok_and(|relative| {
        relative
            .components()
            .all(|component| !ignored_name(component.as_os_str()))
    })
}

fn is_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "ts" | "mjs" | "cjs" | "mts" | "cts" | "jsx" | "tsx")
    )
}

fn hash_contents(reader: &mut impl Read, buffer: &mut [u8; BUFFER_SIZE]) -> Result<u64> {
    let mut hasher = DefaultHasher::new();
    let mut remaining = MAX_BYTES;
    loop {
        let limit = buffer.len().min(remaining + 1);
        match reader.read(&mut buffer[..limit]) {
            Ok(0) => return Ok(hasher.finish()),
            Ok(count) if count > remaining => {
                bail!("SSR input grew beyond the 8 MiB fingerprint limit")
            }
            Ok(count) => {
                remaining -= count;
                hasher.write(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn config_hash(config: &RenderConfig, theme: Option<&str>) -> u64 {
    let mut hash = DefaultHasher::new();
    let args = &config.app_args;
    config.app_dir.hash(&mut hash);
    args.entry.hash(&mut hash);
    std::mem::discriminant(&args.css).hash(&mut hash);
    std::mem::discriminant(&args.dom).hash(&mut hash);
    args.css_bundle.hash(&mut hash);
    args.plugin
        .map(|plugin| std::mem::discriminant(&plugin))
        .hash(&mut hash);
    args.components.hash(&mut hash);
    args.projection_manifests.hash(&mut hash);
    args.asset_file_name_template.hash(&mut hash);
    args.css_public_base.hash(&mut hash);
    std::mem::discriminant(&args.legal_comments).hash(&mut hash);
    config.component_asset_roots.hash(&mut hash);
    config.state_file.hash(&mut hash);
    config.metafile.hash(&mut hash);
    config.base_path.hash(&mut hash);
    theme.hash(&mut hash);
    config.token_file.is_some().hash(&mut hash);
    if let Some(tokens) = &config.token_file {
        let mut themes: Vec<_> = tokens.themes.iter().collect();
        themes.sort_unstable_by_key(|(name, _)| *name);
        for (name, values) in themes {
            name.hash(&mut hash);
            let mut values: Vec<_> = values.iter().collect();
            values.sort_unstable_by_key(|(name, _)| *name);
            values.hash(&mut hash);
        }
    }
    hash.finish()
}

#[cfg(test)]
#[path = "inputs/tests.rs"]
mod tests;
