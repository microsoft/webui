// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::Hasher;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use super::hash::*;

struct ShortReads<'a> {
    contents: Cursor<&'a [u8]>,
    chunks: &'a [usize],
    next: usize,
    interrupt: bool,
}

impl Read for ShortReads<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(!buffer.is_empty());
        assert!(buffer.len() <= HASH_BUFFER_SIZE);
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = buffer.len().min(self.chunks[self.next % self.chunks.len()]);
        self.next += 1;
        self.contents.read(&mut buffer[..count])
    }
}

fn expected_hash(contents: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(contents);
    hasher.finish()
}

#[test]
fn digest_matches_whole_file_with_varied_read_boundaries_and_reused_buffer() -> io::Result<()> {
    let mut buffer = [0xff; HASH_BUFFER_SIZE];
    for size in [0, 1, 7, 8, 9, 8191, 8192, 8193, 3 * 8192 + 37, 0, 3] {
        let contents: Vec<u8> = (0_u8..=255).cycle().take(size).collect();
        for chunks in [
            &[1][..],
            &[7][..],
            &[8][..],
            &[9][..],
            &[8192][..],
            &[3, 8191, 2, 37, 8][..],
        ] {
            let mut reader = ShortReads {
                contents: Cursor::new(contents.as_slice()),
                chunks,
                next: 0,
                interrupt: false,
            };
            assert_eq!(
                hash_contents(&mut reader, &mut buffer)?,
                Some(expected_hash(&contents)),
                "size={size}, chunks={chunks:?}"
            );
        }
    }
    Ok(())
}

struct FailingRead<'a> {
    contents: &'a [u8],
    error: io::ErrorKind,
}

impl Read for FailingRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.contents.is_empty() {
            return Err(self.error.into());
        }
        self.contents.read(buffer)
    }
}

#[test]
fn read_errors_do_not_return_partial_hashes_or_poison_reuse() -> io::Result<()> {
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
        io::ErrorKind::UnexpectedEof,
        io::ErrorKind::WouldBlock,
        io::ErrorKind::Other,
    ] {
        for contents in [&b""[..], &b"partial contents"[..]] {
            let mut reader = FailingRead {
                contents,
                error: kind,
            };
            assert!(matches!(
                hash_contents(&mut reader, &mut buffer),
                Err(error) if error.kind() == kind
            ));
            assert_eq!(
                hash_contents(&mut &b"next file"[..], &mut buffer)?,
                Some(expected_hash(b"next file"))
            );
        }
    }
    Ok(())
}

#[test]
fn cap_is_inclusive_and_reads_at_most_one_extra_byte() -> io::Result<()> {
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    let contents = vec![b'x'; MAX_HASH_BYTES + HASH_BUFFER_SIZE];
    for size in [
        MAX_HASH_BYTES - 1,
        MAX_HASH_BYTES,
        MAX_HASH_BYTES + 1,
        contents.len(),
    ] {
        let mut reader = Cursor::new(&contents[..size]);
        let result = hash_contents(&mut reader, &mut buffer)?;
        if size <= MAX_HASH_BYTES {
            assert_eq!(result, Some(expected_hash(&contents[..size])));
        } else {
            assert_eq!(result, None);
        }
        assert_eq!(reader.position(), size.min(MAX_HASH_BYTES + 1) as u64);
    }
    Ok(())
}

struct GrowOnRead {
    reader: File,
    append_to: File,
    appended: bool,
}

impl Read for GrowOnRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.appended {
            self.append_to.write_all(b"!")?;
            self.appended = true;
        }
        self.reader.read(buffer)
    }
}

#[test]
fn growth_after_metadata_is_rejected_and_does_not_poison_reuse() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("growing.css");
    let file = File::create(&path)?;
    file.set_len(MAX_HASH_BYTES as u64)?;
    drop(file);

    let reader = File::open(&path)?;
    assert_eq!(reader.metadata()?.len(), MAX_HASH_BYTES as u64);
    let mut reader = GrowOnRead {
        reader,
        append_to: std::fs::OpenOptions::new().append(true).open(&path)?,
        appended: false,
    };
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    assert_eq!(hash_contents(&mut reader, &mut buffer)?, None);
    assert_eq!(reader.reader.stream_position()?, MAX_HASH_BYTES as u64 + 1);
    assert_eq!(
        hash_contents(&mut &b"after growth"[..], &mut buffer)?,
        Some(expected_hash(b"after growth"))
    );
    Ok(())
}

#[test]
fn file_hash_covers_empty_multibuffer_and_cap_sized_files() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("input.css");
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    for size in [
        0,
        3 * HASH_BUFFER_SIZE + 7,
        MAX_HASH_BYTES - 1,
        MAX_HASH_BYTES,
    ] {
        let mut contents = vec![b'x'; size];
        if let Some(last) = contents.last_mut() {
            *last = b'y';
        }
        std::fs::write(&path, &contents)?;
        assert_eq!(
            hash_file(&path, &mut buffer),
            Some(expected_hash(&contents))
        );
        if size > 0 {
            let mut file = std::fs::OpenOptions::new().write(true).open(&path)?;
            file.seek(SeekFrom::End(-1))?;
            file.write_all(b"!")?;
            assert_ne!(
                hash_file(&path, &mut buffer),
                Some(expected_hash(&contents))
            );
        }
    }
    Ok(())
}

#[test]
fn missing_non_file_and_oversized_inputs_have_no_digest() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("input.css");
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    assert_eq!(hash_file(&path, &mut buffer), None);
    assert_eq!(hash_file(root.path(), &mut buffer), None);
    File::create(&path)?.set_len(MAX_HASH_BYTES as u64 + 1)?;
    assert_eq!(hash_file(&path, &mut buffer), None);

    std::fs::write(&path, b"readable again")?;
    assert_eq!(
        hash_file(&path, &mut buffer),
        Some(expected_hash(b"readable again"))
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn non_regular_socket_is_rejected() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&path)?;
    assert_eq!(hash_file(&path, &mut [0_u8; HASH_BUFFER_SIZE]), None);
    Ok(())
}
