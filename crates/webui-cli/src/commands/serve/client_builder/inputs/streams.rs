// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

struct Reader {
    remaining: usize,
    consumed: usize,
    chunk: usize,
    interrupt: bool,
    fail_after: Option<usize>,
}

impl Reader {
    fn new(size: usize, chunk: usize) -> Self {
        Self {
            remaining: size,
            consumed: 0,
            chunk,
            interrupt: false,
            fail_after: None,
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after.is_some_and(|limit| self.consumed >= limit) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let count = self.remaining.min(self.chunk).min(buffer.len());
        buffer[..count].fill(42);
        self.remaining -= count;
        self.consumed += count;
        Ok(count)
    }
}

#[test]
fn short_and_interrupted_reads_preserve_the_content_digest() -> Result<()> {
    let mut buffer = [0; BUFFER_SIZE];
    let mut ordinary = Reader::new(128 * 1024, BUFFER_SIZE);
    let expected = hash_contents(&mut ordinary, &mut buffer)?;
    let mut short = Reader::new(128 * 1024, 17);
    short.interrupt = true;
    assert_eq!(expected, hash_contents(&mut short, &mut buffer)?);
    Ok(())
}

#[test]
fn failed_content_reads_never_return_a_partial_digest() {
    let mut buffer = [0; BUFFER_SIZE];
    let mut reader = Reader::new(3 * BUFFER_SIZE, BUFFER_SIZE);
    reader.fail_after = Some(BUFFER_SIZE);
    assert!(hash_contents(&mut reader, &mut buffer).is_err());
}

#[test]
fn stream_growth_is_bounded_with_exactly_one_overflow_probe() -> Result<()> {
    let mut buffer = [0; BUFFER_SIZE];
    let mut at_limit = Reader::new(MAX_BYTES, BUFFER_SIZE);
    hash_contents(&mut at_limit, &mut buffer)?;
    assert_eq!(at_limit.consumed, MAX_BYTES);
    let mut growing = Reader::new(MAX_BYTES * 2, BUFFER_SIZE);
    assert!(hash_contents(&mut growing, &mut buffer).is_err());
    assert_eq!(growing.consumed, MAX_BYTES + 1);
    Ok(())
}

#[test]
fn pending_directory_entries_are_bounded() -> Result<()> {
    let fixture = Fixture::new()?;
    let capture = Capture {
        entries: BTreeMap::new(),
        directories: HashSet::new(),
        buffer: [0; BUFFER_SIZE],
    };
    let mut pending = vec![PathBuf::new(); MAX_ENTRIES];
    assert!(capture
        .children(&fixture.config.app_dir, &mut pending)
        .is_err());
    assert_eq!(pending.len(), MAX_ENTRIES);
    Ok(())
}
