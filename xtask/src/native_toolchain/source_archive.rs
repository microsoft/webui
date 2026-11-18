// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Interpret only digest-verified upstream archives, with a fixed member allowlist.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// Preflight regular-file membership, then extract only fixed, safe filenames.
pub(super) fn extract(
    package: &Path,
    output: &Path,
    required: &BTreeSet<String>,
) -> Result<(), String> {
    let listing = list(package)?;
    validate_listing(&listing, required)?;
    fs::create_dir(output).map_err(|e| e.to_string())?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(package)
        .arg("-C")
        .arg(output)
        .args([
            "--strip-components=1",
            "--no-same-owner",
            "--no-same-permissions",
        ])
        .args(
            required
                .iter()
                .map(|relative| format!("package/{relative}")),
        )
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("checker extraction requires tar: {e}"))?;
    if !status.success() {
        return Err(format!(
            "could not extract verified native checker package ({status})"
        ));
    }
    Ok(())
}

fn list(package: &Path) -> Result<Vec<u8>, String> {
    let mut child = Command::new("tar")
        .arg("-tvzf")
        .arg(package)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("checker inspection requires tar: {e}"))?;
    let mut bytes = Vec::with_capacity(16 * 1024);
    let result = child
        .stdout
        .take()
        .ok_or("missing tar listing output")?
        .take(65_537)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() > 65_536 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("checker archive listing failed or exceeded 64 KiB".to_owned());
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("could not list verified native checker tarball".to_owned());
    }
    Ok(bytes)
}

fn validate_listing(bytes: &[u8], required: &BTreeSet<String>) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    // Some upstream platforms carry a detached code signature. Its bytes are
    // covered by the frozen tarball hash; we neither interpret that signature
    // nor extract it into the standalone runtime inventory.
    let signature = if required.contains("lib/tsc.exe") {
        Some("lib/tsc.exe.sig")
    } else if required.contains("lib/tsc") {
        Some("lib/tsc.sig")
    } else {
        None
    };
    let maximum_entries = required.len() + 3 + usize::from(signature.is_some());
    let mut files = BTreeSet::new();
    let mut entries = BTreeSet::new();
    for line in text.lines() {
        let member = line
            .split_ascii_whitespace()
            .next_back()
            .ok_or("invalid tar listing")?;
        if !entries.insert(member) || entries.len() > maximum_entries {
            return Err("duplicate or excessive checker archive entries".to_owned());
        }
        match line.as_bytes().first() {
            Some(b'-') => {
                let relative = member
                    .strip_prefix("package/")
                    .ok_or("unsafe checker archive path")?;
                if !required.contains(relative)
                    && relative != "README.md"
                    && Some(relative) != signature
                {
                    return Err(format!("unknown checker archive member {member}"));
                }
                files.insert(relative.to_owned());
            }
            Some(b'd') if matches!(member, "package/" | "package/lib/") => {}
            _ => {
                return Err(format!(
                    "checker archive members must be regular files, never links: {member}"
                ))
            }
        }
    }
    if required.iter().any(|name| !files.contains(name)) {
        return Err("checker archive is missing pinned files".to_owned());
    }
    Ok(())
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
