// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use crate::DesktopResponseBody;

/// Owned protocol content. Files retain an open handle and a fixed delivery
/// length; native adapters must not reopen their paths or eagerly buffer them.
#[derive(Debug)]
pub enum DesktopResponseContent {
    /// Bytes, including any IPC response-lifetime reservation.
    Bytes(DesktopResponseBody),
    /// An already-opened, length-bounded file.
    File(DesktopResponseFile),
}

/// A file response bounded to the length observed when it was opened.
/// Growth is ignored; premature EOF is an explicit I/O error.
#[derive(Debug)]
pub struct DesktopResponseFile {
    file: File,
    length: u64,
    position: u64,
}

impl DesktopResponseFile {
    pub(crate) fn new(file: File, length: u64) -> Self {
        Self {
            file,
            length,
            position: 0,
        }
    }

    /// Return the fixed response length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.length
    }

    /// Whether the response has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

impl Read for DesktopResponseFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.length.saturating_sub(self.position);
        let count = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if count == 0 {
            return Ok(0);
        }
        let read = self.file.read(&mut buffer[..count])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "desktop asset was truncated during delivery",
            ));
        }
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for DesktopResponseFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(value) => Some(value),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid desktop asset seek"))?;
        self.position = self.file.seek(SeekFrom::Start(target))?;
        Ok(self.position)
    }
}

impl DesktopResponseContent {
    /// Borrow buffered bytes. File responses return `None` without performing I/O.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&DesktopResponseBody> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::File(_) => None,
        }
    }

    /// Materialize content explicitly for a non-native consumer.
    ///
    /// # Errors
    /// Returns an I/O error if reading a file fails or its contents were truncated.
    pub fn into_bytes(self) -> io::Result<DesktopResponseBody> {
        match self {
            Self::Bytes(bytes) => Ok(bytes),
            Self::File(mut file) => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes.into())
            }
        }
    }
}

impl From<Vec<u8>> for DesktopResponseContent {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes.into())
    }
}

impl From<DesktopResponseBody> for DesktopResponseContent {
    fn from(bytes: DesktopResponseBody) -> Self {
        Self::Bytes(bytes)
    }
}

impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for DesktopResponseContent {
    fn eq(&self, other: &T) -> bool {
        self.as_bytes()
            .is_some_and(|bytes| bytes.as_slice() == other.as_ref())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_is_lazy_length_bounded_and_owns_its_handle() {
        let mut source = tempfile::tempfile().unwrap();
        source.write_all(b"abcdef").unwrap();
        source.rewind().unwrap();
        let body = DesktopResponseContent::File(DesktopResponseFile::new(source, 3));
        assert!(body.as_bytes().is_none());
        assert_eq!(body.into_bytes().unwrap(), b"abc");
    }

    #[test]
    fn file_truncation_is_an_error_not_successful_short_delivery() {
        let mut file = DesktopResponseFile::new(tempfile::tempfile().unwrap(), 3);
        assert_eq!(
            file.read(&mut [0; 3]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn file_seek_and_empty_contract() {
        let mut file = DesktopResponseFile::new(tempfile::tempfile().unwrap(), 0);
        assert!(file.is_empty());
        assert_eq!(file.len(), 0);
        assert!(file.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(file.seek(SeekFrom::End(0)).unwrap(), 0);
        assert_eq!(file.read(&mut [0; 1]).unwrap(), 0);
    }
}
