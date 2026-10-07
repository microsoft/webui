// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time header checks for explicitly mapped native binaries.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::{DesktopError, Result};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Format {
    Mach,
    Pe,
    Elf,
}

#[derive(Clone, Copy)]
pub(super) struct Target {
    pub format: Format,
    pub machine: u32,
}

pub(super) fn parse_target(triple: &str, format: Format) -> Result<Target> {
    let machine = match (format, triple) {
        (Format::Mach, "x86_64-apple-darwin") => 0x0100_0007,
        (Format::Mach, "aarch64-apple-darwin") => 0x0100_000c,
        (Format::Pe, "x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu") => 0x8664,
        (Format::Pe, "aarch64-pc-windows-msvc") => 0xaa64,
        (Format::Elf, "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl") => 0x3e,
        (Format::Elf, "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl") => 0xb7,
        _ => {
            return Err(invalid(format!(
                "unsupported target triple {triple:?} for this package layout"
            )))
        }
    };
    Ok(Target { format, machine })
}

pub(super) fn validate_binary(file: &mut File, path: &Path, target: Target) -> Result<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| DesktopError::Io {
            context: format!("seeking package executable {}", path.display()),
            source,
        })?;
    let mut header = [0u8; 4096];
    let count = file.read(&mut header).map_err(|source| DesktopError::Io {
        context: format!("reading package executable header {}", path.display()),
        source,
    })?;
    let bytes = &header[..count];
    let valid = match target.format {
        Format::Mach => mach_machine(file, bytes, target.machine),
        Format::Pe => pe_machine(file, bytes, path)? == Some(target.machine),
        Format::Elf => elf_machine(bytes) == Some(target.machine),
    };
    if !valid {
        return Err(invalid(format!(
            "binary {} does not match the requested OS and architecture",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn validate_data(file: &mut File, path: &Path, target: Target) -> Result<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| DesktopError::Io {
            context: format!("seeking package resource {}", path.display()),
            source,
        })?;
    let mut magic = [0u8; 4];
    let count = file.read(&mut magic).map_err(|source| DesktopError::Io {
        context: format!("reading package resource header {}", path.display()),
        source,
    })?;
    let native = count == 4
        && (magic == *b"\x7fELF"
            || magic == *b"PE\0\0"
            || magic[..2] == *b"MZ"
            || matches!(
                magic,
                [0xcf, 0xfa, 0xed, 0xfe]
                    | [0xfe, 0xed, 0xfa, 0xcf]
                    | [0xca, 0xfe, 0xba, 0xbe]
                    | [0xbe, 0xba, 0xfe, 0xca]
                    | [0xca, 0xfe, 0xba, 0xbf]
                    | [0xbf, 0xba, 0xfe, 0xca]
            ));
    let extension = path.extension().and_then(|value| value.to_str());
    if native || matches!(extension, Some("exe" | "dll" | "so" | "dylib")) {
        validate_binary(file, path, target)?;
    }
    Ok(())
}

fn mach_machine(file: &mut File, bytes: &[u8], machine: u32) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    // Thin 64-bit Mach-O. 32-bit hosts are not supported.
    if bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
        return u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == machine;
    }
    if bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf]) {
        return u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == machine;
    }
    // Universal binary: find the selected architecture in the bounded slice table.
    let (little, entry_size) = match &bytes[..4] {
        [0xca, 0xfe, 0xba, 0xbe] => (false, 20),
        [0xbe, 0xba, 0xfe, 0xca] => (true, 20),
        [0xca, 0xfe, 0xba, 0xbf] => (false, 32),
        [0xbf, 0xba, 0xfe, 0xca] => (true, 32),
        _ => return false,
    };
    let read = |chunk: &[u8]| -> u32 {
        let value = [chunk[0], chunk[1], chunk[2], chunk[3]];
        if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    };
    let count = read(&bytes[4..8]) as usize;
    if count == 0 || count > 64 || 8 + count * entry_size > bytes.len() {
        return false;
    }
    let file_len = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(_) => return false,
    };
    for index in 0..count {
        let entry = &bytes[8 + index * entry_size..][..entry_size];
        if read(&entry[..4]) != machine {
            continue;
        }
        let (offset, size) = if entry_size == 20 {
            (
                u64::from(read(&entry[8..12])),
                u64::from(read(&entry[12..16])),
            )
        } else {
            let read64 = |slice: &[u8]| {
                let value = [
                    slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                ];
                if little {
                    u64::from_le_bytes(value)
                } else {
                    u64::from_be_bytes(value)
                }
            };
            (read64(&entry[8..16]), read64(&entry[16..24]))
        };
        if size < 8 || offset.checked_add(size).is_none_or(|end| end > file_len) {
            continue;
        }
        let mut thin_header = [0u8; 8];
        if file.seek(SeekFrom::Start(offset)).is_err() || file.read_exact(&mut thin_header).is_err()
        {
            continue;
        }
        let inner_machine = match &thin_header[..4] {
            [0xcf, 0xfa, 0xed, 0xfe] => u32::from_le_bytes([
                thin_header[4],
                thin_header[5],
                thin_header[6],
                thin_header[7],
            ]),
            [0xfe, 0xed, 0xfa, 0xcf] => u32::from_be_bytes([
                thin_header[4],
                thin_header[5],
                thin_header[6],
                thin_header[7],
            ]),
            _ => continue,
        };
        if inner_machine == machine {
            return true;
        }
    }
    false
}

fn pe_machine(file: &mut File, bytes: &[u8], path: &Path) -> Result<Option<u32>> {
    if bytes.len() < 0x40 || !bytes.starts_with(b"MZ") {
        return Ok(None);
    }
    let offset = u64::from(u32::from_le_bytes([
        bytes[0x3c],
        bytes[0x3d],
        bytes[0x3e],
        bytes[0x3f],
    ]));
    if let Ok(start) = usize::try_from(offset) {
        if let Some(end) = start.checked_add(6) {
            if let Some(header) = bytes.get(start..end) {
                return Ok(pe_header_machine(header));
            }
        }
    }
    let file_len = file
        .metadata()
        .map_err(|source| DesktopError::Io {
            context: format!("checking PE executable size {}", path.display()),
            source,
        })?
        .len();
    if offset.checked_add(6).is_none_or(|end| end > file_len) {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| DesktopError::Io {
            context: format!("seeking PE executable header {}", path.display()),
            source,
        })?;
    let mut header = [0u8; 6];
    file.read_exact(&mut header)
        .map_err(|source| DesktopError::Io {
            context: format!("reading PE executable header {}", path.display()),
            source,
        })?;
    Ok(pe_header_machine(&header))
}

fn pe_header_machine(header: &[u8]) -> Option<u32> {
    if header.get(..4)? != b"PE\0\0" {
        return None;
    }
    let machine = header.get(4..6)?;
    Some(u32::from(u16::from_le_bytes([machine[0], machine[1]])))
}

fn elf_machine(bytes: &[u8]) -> Option<u32> {
    let header = bytes.get(..20)?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 {
        return None;
    }
    match header[5] {
        1 => Some(u32::from(u16::from_le_bytes([header[18], header[19]]))),
        2 => Some(u32::from(u16::from_be_bytes([header[18], header[19]]))),
        _ => None,
    }
}

pub(super) fn invalid(message: String) -> DesktopError {
    DesktopError::PackageValidation {
        message,
        help: "Use a precompiled native executable matching the explicit target triple; do not pass a script, a different architecture, or an unsupported target",
    }
}
