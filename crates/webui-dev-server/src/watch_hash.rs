// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::Hasher;
use std::io::{self, Read};
use std::path::Path;

pub(super) const HASH_BUFFER_SIZE: usize = 8 * 1024;

// Largest file hashed for no-op detection; larger inputs always trigger a rebuild.
pub(super) const MAX_HASH_BYTES: usize = 8 * 1024 * 1024;

// Hash readable regular files with the standard hasher; failures count as changes.
pub(super) fn hash_file(path: &Path, buffer: &mut [u8; HASH_BUFFER_SIZE]) -> Option<u64> {
    // Reject non-files before opening: opening a FIFO can block the watcher.
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_HASH_BYTES as u64 {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_HASH_BYTES as u64 {
        return None;
    }
    hash_contents(&mut file, buffer).ok().flatten()
}

pub(super) fn hash_contents(
    reader: &mut impl Read,
    buffer: &mut [u8; HASH_BUFFER_SIZE],
) -> io::Result<Option<u64>> {
    let mut hasher = DefaultHasher::new();
    let mut remaining = MAX_HASH_BYTES;
    loop {
        // Probe one byte past the cap to catch growth after the metadata check.
        let limit = buffer.len().min(remaining + 1);
        match reader.read(&mut buffer[..limit]) {
            Ok(0) => return Ok(Some(hasher.finish())),
            Ok(count) if count > remaining => return Ok(None),
            Ok(count) => {
                remaining -= count;
                hasher.write(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}
