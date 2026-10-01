// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use percent_encoding::percent_decode_str;

pub(crate) const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        bail!(
            "{} must be a regular file no larger than 32 MiB",
            path.display()
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!(
            "{} grew beyond the 32 MiB limit while reading",
            path.display()
        );
    }
    Ok(bytes)
}

pub(crate) fn asset_path(root: &Path, request_path: &str) -> Result<Option<PathBuf>> {
    let Some(relative) = request_path.strip_prefix('/') else {
        bail!("asset paths must start with '/'");
    };
    let mut path = root.to_path_buf();
    for raw in relative.split('/') {
        let decoded = percent_decode_str(raw).decode_utf8()?;
        if decoded.is_empty()
            || decoded == "."
            || decoded == ".."
            || decoded.contains(['/', '\\', '\0', ':'])
        {
            bail!("invalid asset path segment");
        }
        path.push(decoded.as_ref());
    }
    // Only browser assets are public, never protocol/state/build metadata.
    if !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some(
            "js" | "mjs"
                | "css"
                | "svg"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
        )
    ) {
        return Ok(None);
    }
    match path.canonicalize() {
        Ok(canonical) if canonical.starts_with(root) => Ok(Some(canonical)),
        Ok(_) => bail!("asset resolves outside the app directory"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("cannot resolve asset path"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_nested_assets_but_not_private_inputs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        std::fs::create_dir(root.join("assets"))?;
        let script = root.join("assets/hello world.js");
        std::fs::write(&script, "export const ready = true;")?;
        assert_eq!(asset_path(&root, "/assets/hello%20world.js")?, Some(script));
        for path in ["/protocol.bin", "/state.json", "/source.map", "/missing.js"] {
            assert!(asset_path(&root, path)?.is_none(), "{path}");
        }
        Ok(())
    }

    #[test]
    fn rejects_traversal_separators_and_windows_streams() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for path in [
            "../secret.js",
            "/../secret.js",
            "/%2e%2e/secret.js",
            "/assets//index.js",
            "/assets/./index.js",
            "/a%2fb.js",
            "/a%5cb.js",
            "/a%00.js",
            "/C%3a/a.js",
            "/a.js%3asecret.js",
            "/%ff.js",
        ] {
            assert!(asset_path(dir.path(), path).is_err(), "{path}");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_outside_the_bundle() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let secret = outside.path().join("secret.js");
        std::fs::write(&secret, "private")?;
        std::os::unix::fs::symlink(secret, dir.path().join("escape.js"))?;
        assert!(asset_path(&dir.path().canonicalize()?, "/escape.js").is_err());
        Ok(())
    }

    #[test]
    fn bounds_files_before_allocating() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("large.bin");
        File::create(&path)?.set_len(MAX_FILE_BYTES + 1)?;
        assert!(read_bounded(&path).is_err());
        assert!(read_bounded(dir.path()).is_err());
        Ok(())
    }
}
