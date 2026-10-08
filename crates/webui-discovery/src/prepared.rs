// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::npm::{package_asset_metadata, MAX_MANIFEST_SIZE};
use crate::DiscoveredComponent;

const HASH_BUFFER_SIZE: usize = 8 * 1024;

/// A component whose source selection and ownership have already been resolved.
///
/// Preparation must validate the owning package boundary and select optional
/// styles and script ownership. Loading does not repeat those decisions.
#[derive(Debug)]
pub struct ComponentFileSource {
    /// Custom element name used for registration.
    pub tag_name: String,
    /// Selected HTML template.
    pub html: ComponentFile,
    /// Selected stylesheet, or `None` when no stylesheet was found.
    pub css: Option<ComponentFile>,
    /// Whether authored browser code owns the custom element.
    pub is_client_owned: bool,
}

/// A selected file and its owning package boundary.
#[derive(Debug, Hash)]
pub struct ComponentFile {
    pub(crate) package_root: Arc<Path>,
    pub(crate) path: PathBuf,
}

impl ComponentFile {
    /// Resolve a selected file within its canonical owning package root.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot be resolved or leaves the package.
    #[must_use = "the resolved file belongs in a prepared component source"]
    pub fn new(package_root: Arc<Path>, path: &Path) -> Result<Self> {
        let resolved = path
            .canonicalize()
            .with_context(|| format!("Cannot resolve component source {}", path.display()))?;
        if !resolved.starts_with(package_root.as_ref()) {
            bail!(
                "Component source resolves outside package {}: {}",
                package_root.display(),
                path.display()
            );
        }
        Ok(Self {
            package_root,
            path: resolved,
        })
    }
}

/// A package's resolved discovery inputs.
///
/// File-backed plans can be cached from their decisions and actual source bytes.
/// Arbitrary computed components are returned without caching because their
/// inputs cannot be verified by the shared file loader.
#[derive(Debug)]
pub enum PreparedPackage {
    /// A fixed, ordered inventory of component sources.
    Files(Vec<ComponentFileSource>),
    /// Already computed components with plugin-owned input semantics.
    Uncached(Vec<DiscoveredComponent>),
}

pub(crate) struct LoadedPackage {
    components: Vec<DiscoveredComponent>,
    fingerprint: u64,
}

impl LoadedPackage {
    pub(crate) fn components(&self) -> &[DiscoveredComponent] {
        &self.components
    }

    pub(crate) fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub(crate) fn into_components(self) -> Vec<DiscoveredComponent> {
        self.components
    }
}

fn start_fingerprint(package_json: &str, count: usize) -> DefaultHasher {
    let mut hasher = DefaultHasher::new();
    package_json.hash(&mut hasher);
    count.hash(&mut hasher);
    hasher
}

fn hash_source(input: &ComponentFileSource, hasher: &mut DefaultHasher) {
    input.tag_name.hash(hasher);
    input.html.hash(hasher);
    input.css.hash(hasher);
    input.is_client_owned.hash(hasher);
}

pub(crate) fn fingerprint(package_json: &str, inputs: &[ComponentFileSource]) -> Result<u64> {
    let mut hasher = start_fingerprint(package_json, inputs.len());
    let mut buffer = [0; HASH_BUFFER_SIZE];
    for input in inputs {
        hash_source(input, &mut hasher);
        hash_file(&input.html.package_root, &input.html.path, &mut buffer)?.hash(&mut hasher);
        if let Some(file) = &input.css {
            hash_file(&file.package_root, &file.path, &mut buffer)?.hash(&mut hasher);
        }
    }
    Ok(hasher.finish())
}

pub(crate) fn load(
    source: &str,
    package_json: &str,
    inputs: Vec<ComponentFileSource>,
) -> Result<LoadedPackage> {
    let mut hasher = start_fingerprint(package_json, inputs.len());
    let mut components = Vec::with_capacity(inputs.len());
    for input in inputs {
        hash_source(&input, &mut hasher);
        let html_content = read_source(&input.html.package_root, &input.html.path)?;
        hash_bytes(html_content.as_bytes()).hash(&mut hasher);
        let css_content = if let Some(file) = &input.css {
            let content = read_source(&file.package_root, &file.path)?;
            hash_bytes(content.as_bytes()).hash(&mut hasher);
            Some(content)
        } else {
            None
        };
        components.push(DiscoveredComponent {
            tag_name: input.tag_name,
            html_content,
            css_content,
            is_client_owned: input.is_client_owned,
            source: source.to_string(),
        });
    }
    Ok(LoadedPackage {
        components,
        fingerprint: hasher.finish(),
    })
}

fn open_source(root: &Path, path: &Path) -> Result<(File, u64)> {
    let metadata = package_asset_metadata(root, path)?.with_context(|| {
        format!(
            "Component source disappeared: {}. Retry after source generation finishes.",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        bail!("Component source must be a file: {}", path.display());
    }
    if metadata.len() > MAX_MANIFEST_SIZE {
        return Err(oversized_source(path));
    }
    let file = File::open(path)
        .with_context(|| format!("Failed to open component source {}", path.display()))?;
    Ok((file, metadata.len()))
}

fn read_source(root: &Path, path: &Path) -> Result<String> {
    let (file, size) = open_source(root, path)?;
    let capacity =
        usize::try_from(size).context("Component source is too large for this platform")?;
    let mut content = String::with_capacity(capacity);
    let mut reader = file.take(MAX_MANIFEST_SIZE + 1);
    reader
        .read_to_string(&mut content)
        .with_context(|| format!("Failed to read component source {}", path.display()))?;
    if reader.limit() == 0 {
        return Err(oversized_source(path));
    }
    Ok(content)
}

fn hash_file(root: &Path, path: &Path, buffer: &mut [u8; HASH_BUFFER_SIZE]) -> Result<u64> {
    let (file, _) = open_source(root, path)?;
    let mut reader = file.take(MAX_MANIFEST_SIZE + 1);
    let hash = hash_contents(&mut reader, buffer)
        .with_context(|| format!("Failed to hash component source {}", path.display()))?;
    if reader.limit() == 0 {
        return Err(oversized_source(path));
    }
    Ok(hash)
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn hash_contents(reader: &mut impl Read, buffer: &mut [u8; HASH_BUFFER_SIZE]) -> io::Result<u64> {
    let mut hasher = DefaultHasher::new();
    loop {
        match reader.read(buffer) {
            Ok(0) => return Ok(hasher.finish()),
            Ok(count) => hasher.write(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cold]
#[inline(never)]
fn oversized_source(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "Component source exceeds {MAX_MANIFEST_SIZE} bytes: {}. \
         Reduce the source size before building.",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Cursor, Seek, SeekFrom, Write};

    #[test]
    fn selected_file_must_resolve_inside_its_package() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("package"))?;
        fs::write(root.path().join("package/template.html"), "<p>Inside</p>")?;
        fs::write(root.path().join("outside.html"), "<p>Outside</p>")?;
        let boundary: Arc<Path> = root.path().join("package").canonicalize()?.into();
        let input = ComponentFile::new(Arc::clone(&boundary), &boundary.join("template.html"))?;
        assert_eq!(input.path, boundary.join("template.html"));
        assert!(ComponentFile::new(Arc::clone(&boundary), &boundary.join("missing.html")).is_err());
        assert!(ComponentFile::new(boundary, &root.path().join("outside.html")).is_err());
        Ok(())
    }

    struct ShortReads<'a> {
        content: Cursor<&'a [u8]>,
        interrupted: bool,
    }

    impl Read for ShortReads<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            assert!(buffer.len() <= HASH_BUFFER_SIZE);
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let count = buffer.len().min(37);
            self.content.read(&mut buffer[..count])
        }
    }

    #[test]
    fn streaming_hash_is_bounded_and_independent_of_read_boundaries() -> Result<()> {
        let mut buffer = [0; HASH_BUFFER_SIZE];
        for size in [0, 1, 8191, 8192, 8193, 24595] {
            let mut content = vec![b'x'; size];
            if let Some(last) = content.last_mut() {
                *last = b'y';
            }
            let mut reader = ShortReads {
                content: Cursor::new(&content),
                interrupted: false,
            };
            assert_eq!(
                hash_contents(&mut reader, &mut buffer)?,
                hash_bytes(&content)
            );
        }
        Ok(())
    }

    struct FailingRead(io::ErrorKind);

    impl Read for FailingRead {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
    }

    #[test]
    fn streaming_hash_propagates_read_failures() {
        let mut buffer = [0; HASH_BUFFER_SIZE];
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound] {
            let result = hash_contents(&mut FailingRead(kind), &mut buffer);
            assert!(matches!(result, Err(error) if error.kind() == kind));
        }
    }

    #[test]
    fn fingerprint_tracks_all_selected_bytes_and_rejects_missing_sources() -> Result<()> {
        let root = tempfile::tempdir()?;
        let boundary = root.path().canonicalize()?;
        for extension in ["json", "html", "css", "ts", "js"] {
            let path = boundary.join(format!("input.{extension}"));
            let inputs = vec![ComponentFileSource {
                tag_name: "test-input".to_string(),
                html: ComponentFile {
                    package_root: boundary.as_path().into(),
                    path: path.clone(),
                },
                css: None,
                is_client_owned: false,
            }];
            assert!(fingerprint("{}", &inputs).is_err());
            let mut file = fs::File::create(&path)?;
            let empty = fingerprint("{}", &inputs)?;
            file.write_all(&[b'x'; HASH_BUFFER_SIZE])?;
            file.write_all(b"tail")?;
            let original = fingerprint("{}", &inputs)?;
            assert_ne!(empty, original);
            file.seek(SeekFrom::End(-1))?;
            file.write_all(b"!")?;
            let changed = fingerprint("{}", &inputs)?;
            assert_ne!(original, changed, "{extension} tail was not hashed");
            drop(file);
            assert_eq!(load("test", "{}", inputs)?.fingerprint(), changed);
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
