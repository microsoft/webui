// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

pub(super) struct TemporaryComponentAssets(pub PathBuf);

impl TemporaryComponentAssets {
    pub(super) fn create() -> Result<Self> {
        loop {
            let id = DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "webui-component-assets-{}-{id}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to create {}", path.display()))
                }
            }
        }
    }
}

impl Drop for TemporaryComponentAssets {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            log::warn!(
                "Failed to clean component asset directory {}: {error}",
                self.0.display()
            );
        }
    }
}
