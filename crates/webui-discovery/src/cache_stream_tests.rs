// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::{Cursor, Seek, SeekFrom, Write};

use super::*;

struct ShortReads<'a> {
    content: Cursor<&'a [u8]>,
    interrupted: bool,
}

impl Read for ShortReads<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(buffer.len() <= 8 * 1024, "hash reads must stay bounded");
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
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    for size in [0, 1, 8191, 8192, 8193, 24595] {
        let mut content = vec![b'x'; size];
        if let Some(last) = content.last_mut() {
            *last = b'y';
        }
        let mut expected = DefaultHasher::new();
        expected.write(&content);
        let mut reader = ShortReads {
            content: Cursor::new(&content),
            interrupted: false,
        };
        assert_eq!(hash_contents(&mut reader, &mut buffer)?, expected.finish());
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
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound] {
        let result = hash_contents(&mut FailingRead(kind), &mut buffer);
        assert!(matches!(result, Err(error) if error.kind() == kind));
    }
}

#[test]
fn fingerprint_tracks_all_dependency_bytes_and_presence() -> Result<()> {
    let root = tempfile::tempdir()?;
    for extension in ["json", "html", "css", "ts", "js"] {
        let path = root.path().join(format!("input.{extension}"));
        let dependencies = [path.clone()];
        let fingerprint = || {
            if extension == "json" {
                DiscoveryCache::fingerprint(Some(&path), &[])
            } else {
                DiscoveryCache::fingerprint(None, &dependencies)
            }
        };
        let missing = fingerprint()?;
        let mut file = fs::File::create(&path)?;
        let empty = fingerprint()?;
        assert_ne!(missing, empty);
        file.write_all(&[b'x'; HASH_BUFFER_SIZE])?;
        file.write_all(b"tail")?;
        let original = fingerprint()?;
        assert_ne!(empty, original);
        file.seek(SeekFrom::End(-1))?;
        file.write_all(b"!")?;
        assert_ne!(original, fingerprint()?, "{extension} tail was not hashed");
        drop(file);
        fs::remove_file(&path)?;
        assert_eq!(missing, fingerprint()?);
    }
    Ok(())
}
