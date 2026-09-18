// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bounded, script-safe JSON serialization into the response.

use std::io;

use serde::Serialize;

use crate::{write_script_safe_json_str, HandlerError, ResponseWriter, Result};

const SCRATCH_LIMIT: usize = 4096;

pub(crate) fn write_script_safe_json<T>(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    value: &T,
) -> Result<()>
where
    T: Serialize + ?Sized,
{
    scratch.clear();
    if scratch.capacity() > SCRATCH_LIMIT {
        scratch.shrink_to(SCRATCH_LIMIT);
    }
    struct CompactUtf8;

    impl Utf8Policy for CompactUtf8 {
        #[inline]
        #[allow(unsafe_code)]
        fn decode<'b>(&self, bytes: &'b [u8]) -> Result<&'b str> {
            // SAFETY: this policy's only writer starts with empty scratch below
            // and is lent solely to serde_json::to_writer, whose CompactFormatter
            // guarantees complete UTF-8 writes. JsonWriter consumes whole writes
            // or errors, never byte-splits them, and only trims/carries ASCII '<'
            // at drains. Concatenation and these cuts preserve valid UTF-8.
            // Neither this policy nor its writer escapes this function.
            Ok(unsafe { std::str::from_utf8_unchecked(bytes) })
        }
    }

    let mut output = JsonWriter {
        writer,
        scratch,
        error: None,
        utf8: CompactUtf8,
    };
    let result = serde_json::to_writer(&mut output, value);
    if let Some(error) = output.error.take() {
        return Err(error);
    }
    result.map_err(serialization_error)?;
    output.drain(true)
}

// The compact serializer writes complete UTF-8 fragments. Only the ASCII
// closing-tag prefix needs to survive a drain into the response.
struct JsonWriter<'a, Policy = Checked> {
    writer: &'a mut dyn ResponseWriter,
    scratch: &'a mut Vec<u8>,
    error: Option<HandlerError>,
    utf8: Policy,
}

trait Utf8Policy {
    fn decode<'b>(&self, bytes: &'b [u8]) -> Result<&'b str>;
}

struct Checked;

impl Utf8Policy for Checked {
    #[inline]
    fn decode<'b>(&self, bytes: &'b [u8]) -> Result<&'b str> {
        std::str::from_utf8(bytes).map_err(utf8_error)
    }
}

impl<Policy: Utf8Policy> JsonWriter<'_, Policy> {
    #[inline(never)]
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.error.is_some() {
            return Err(io::ErrorKind::Other.into());
        }
        if bytes.len() > SCRATCH_LIMIT - self.scratch.len() {
            self.drain(false).map_err(|error| self.fail(error))?;
            if bytes.len() > SCRATCH_LIMIT - self.scratch.len() {
                return self.write_large(bytes).map_err(|error| self.fail(error));
            }
        }
        // Vec's usual doubling must not overshoot the cap for an odd capacity.
        if bytes.len() > self.scratch.capacity() - self.scratch.len()
            && self.scratch.capacity() > SCRATCH_LIMIT / 2
        {
            self.scratch
                .reserve_exact(SCRATCH_LIMIT - self.scratch.len());
        }
        self.scratch.extend_from_slice(bytes);
        Ok(())
    }

    fn drain(&mut self, complete: bool) -> Result<()> {
        // A closing tag may straddle serializer writes or buffer drains.
        let keep_open = !complete && self.scratch.last() == Some(&b'<');
        let end = self.scratch.len() - usize::from(keep_open);
        let json = self.utf8.decode(&self.scratch[..end])?;
        write_script_safe_json_str(self.writer, json)?;
        self.scratch.clear();
        if keep_open {
            self.scratch.push(b'<');
        }
        Ok(())
    }

    #[inline(never)]
    fn write_large(&mut self, bytes: &[u8]) -> Result<()> {
        let mut json = self.utf8.decode(bytes)?;
        if !self.scratch.is_empty() {
            if let Some(rest) = json.strip_prefix('/') {
                self.writer.write("<\\/")?;
                json = rest;
            } else {
                self.writer.write("<")?;
            }
            self.scratch.clear();
        }
        if let Some(rest) = json.strip_suffix('<') {
            write_script_safe_json_str(self.writer, rest)?;
            self.scratch.push(b'<');
            Ok(())
        } else {
            write_script_safe_json_str(self.writer, json)
        }
    }

    #[cold]
    #[inline(never)]
    fn fail(&mut self, error: HandlerError) -> io::Error {
        self.error = Some(error);
        // Poison implies zero capacity: later nonempty writes cannot bypass
        // append's error guard through the in-capacity fast path.
        *self.scratch = Vec::new();
        io::ErrorKind::Other.into()
    }
}

impl<Policy: Utf8Policy> io::Write for JsonWriter<'_, Policy> {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_all(bytes)?;
        Ok(bytes.len())
    }

    #[inline(always)]
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if !bytes.is_empty() && bytes.len() <= self.scratch.capacity() - self.scratch.len() {
            self.scratch.extend_from_slice(bytes);
            return Ok(());
        }
        self.append(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.error.is_some() {
            return Err(io::ErrorKind::Other.into());
        }
        self.drain(false).map_err(|error| self.fail(error))
    }
}

#[cold]
#[inline(never)]
fn serialization_error(error: serde_json::Error) -> HandlerError {
    HandlerError::Rendering(format!("failed to serialize JSON: {error}"))
}

#[cold]
#[inline(never)]
fn utf8_error(error: std::str::Utf8Error) -> HandlerError {
    HandlerError::Rendering(format!("invalid JSON UTF-8: {error}"))
}

#[cfg(test)]
mod tests;
