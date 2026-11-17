// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Standalone artifact contract shared with the isolated xtask producer.
//! No CLI policy, process launch, package discovery, or SDK dependencies here.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Upstream checker distribution identity, independent of the WebUI release.
pub(crate) const TYPESCRIPT_VERSION: &str = "7.0.2";
/// Maximum toolchain/package metadata bytes interpreted by the consumer.
pub(crate) const MAX_METADATA: u64 = 65_536;
const LIBRARIES: &str = include_str!("typescript-libs.txt");

/// Target-specific names and upstream native distribution identity.
pub(crate) struct Target {
    pub(crate) cli: &'static str,
    pub(crate) compiler: &'static str,
    pub(crate) os: &'static str,
    pub(crate) cpu: &'static str,
}

/// Supported release target, selected explicitly rather than from runner CPU.
pub(crate) fn target(triple: &str) -> Option<Target> {
    let (os, cpu) = match triple {
        "aarch64-apple-darwin" => ("darwin", "arm64"),
        "x86_64-apple-darwin" => ("darwin", "x64"),
        "aarch64-unknown-linux-gnu" => ("linux", "arm64"),
        "x86_64-unknown-linux-gnu" => ("linux", "x64"),
        "aarch64-pc-windows-msvc" => ("win32", "arm64"),
        "x86_64-pc-windows-msvc" => ("win32", "x64"),
        _ => return None,
    };
    Some(Target {
        cli: if os == "win32" { "webui.exe" } else { "webui" },
        compiler: if os == "win32" { "tsc.exe" } else { "tsc" },
        os,
        cpu,
    })
}

/// Exact upstream standard-library filenames for the pinned checker release.
pub(crate) fn libraries() -> impl Iterator<Item = &'static str> {
    LIBRARIES.lines()
}

/// Exact standalone archive payload, excluding the self-describing manifest.
pub(crate) fn inventory(target: &Target) -> impl Iterator<Item = String> + '_ {
    [
        format!("bin/{}", target.cli),
        format!("compiler/{}", target.compiler),
        "notices/WebUI-LICENSE".to_owned(),
        "notices/TypeScript-LICENSE".to_owned(),
        "notices/TypeScript-NOTICE.txt".to_owned(),
    ]
    .into_iter()
    .chain(libraries().map(|name| format!("compiler/{name}")))
}

/// Check every component below the canonical root; directory symlinks are
/// rejected even when they point back inside the root.
pub(crate) fn regular_file(root: &Path, relative: &str) -> io::Result<PathBuf> {
    if relative.is_empty()
        || relative.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':'])
        })
    {
        return Err(invalid_data("unsafe artifact inventory path"));
    }
    let mut path = root.to_owned();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        let kind = fs::symlink_metadata(&path)?.file_type();
        if (parts.peek().is_some() && !kind.is_dir()) || (parts.peek().is_none() && !kind.is_file())
        {
            return Err(invalid_data(
                "artifact paths must be regular files/directories, not symlinks",
            ));
        }
    }
    if !fs::canonicalize(&path)?.starts_with(root) {
        return Err(invalid_data("artifact escapes its root"));
    }
    Ok(path)
}

/// Read a small artifact, rejecting rather than truncating oversized input.
pub(crate) fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(4096);
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid_data("artifact exceeds its size limit"));
    }
    Ok(bytes)
}

/// Hash final staged bytes with a single bounded buffer, not a file-sized read.
pub(crate) fn sha256(path: &Path, limit: u64) -> io::Result<String> {
    let mut file = File::open(path)?.take(limit + 1);
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut total = 0_u64;
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > limit {
            return Err(invalid_data("artifact exceeds its size limit"));
        }
        hash.update(&buffer[..length]);
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in hash.finalize() {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(hex)
}

/// Per-entry byte budget for streaming integrity checks.
pub(crate) fn file_limit(relative: &str) -> u64 {
    if relative.ends_with(".d.ts") || relative.starts_with("notices/") {
        8 * 1024 * 1024
    } else {
        256 * 1024 * 1024
    }
}

/// Reject script launchers and wrong-architecture native executables before
/// execution. Release authenticity is provided by release provenance, not magic.
pub(crate) fn native_executable(path: &Path, target: &Target) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 64];
    if let Err(error) = file.read_exact(&mut header) {
        return if error.kind() == io::ErrorKind::UnexpectedEof {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let arm = target.cpu == "arm64";
    Ok(match target.os {
        "darwin" => {
            let cpu = if arm {
                0x0100_000c_u32
            } else {
                0x0100_0007_u32
            };
            header[..4] == [0xcf, 0xfa, 0xed, 0xfe] && header[4..8] == cpu.to_le_bytes()
        }
        "linux" => {
            let machine = if arm { 183_u16 } else { 62_u16 };
            header[..6] == [0x7f, b'E', b'L', b'F', 2, 1] && header[18..20] == machine.to_le_bytes()
        }
        "win32" if &header[..2] == b"MZ" => {
            let offset = u32::from_le_bytes([header[60], header[61], header[62], header[63]]);
            if offset > 1024 * 1024 {
                return Ok(false);
            }
            file.seek(SeekFrom::Start(u64::from(offset)))?;
            let mut pe = [0_u8; 6];
            file.read_exact(&mut pe)?;
            let machine = if arm { 0xaa64_u16 } else { 0x8664_u16 };
            &pe[..4] == b"PE\0\0" && pe[4..6] == machine.to_le_bytes()
        }
        _ => false,
    })
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
