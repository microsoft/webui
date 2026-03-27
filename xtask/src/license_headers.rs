// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! License header enforcement for source files.
//!
//! Uses `git ls-files` to enumerate tracked source files, automatically
//! respecting `.gitignore` rules. Only files with checked extensions that
//! are not in the skip list are inspected for the required copyright header.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The two-line copyright header that must appear at the top of every source
/// file.
const HEADER_LINE_1: &str = "// Copyright (c) Microsoft Corporation.";
const HEADER_LINE_2: &str = "// Licensed under the MIT license.";

/// Extensions that require the `//`-style license header.
const CHECKED_EXTENSIONS: &[&str] = &["rs", "ts", "js", "cs", "h", "proto"];

/// Individual tracked files to skip (relative to workspace root).
/// Generated files that are checked in but not hand-authored belong here.
const SKIP_FILES: &[&str] = &["crates/webui-ffi/include/webui_ffi.h"];

// ── Public API ──────────────────────────────────────────────────────────

/// Check all source files for the license header.
///
/// Returns `Ok(())` if every file passes, or `Err` with a summary of
/// missing-header files.
pub fn check() -> Result<(), String> {
    let missing = collect_missing()?;

    if missing.is_empty() {
        return Ok(());
    }

    let mut msg = format!("{} file(s) missing the license header:\n", missing.len());
    for path in &missing {
        msg.push_str(&format!("  {}\n", path.display()));
    }
    msg.push_str("\nRun `cargo xtask license-headers --fix` to add the header automatically.");
    Err(msg)
}

/// Add the license header to every source file that is missing it.
pub fn fix() -> Result<(), String> {
    let missing = collect_missing()?;

    if missing.is_empty() {
        eprintln!("  All source files already have the license header.");
        return Ok(());
    }

    for path in &missing {
        prepend_header(path)?;
    }

    eprintln!("  Added license header to {} file(s).", missing.len());
    Ok(())
}

// ── Internals ───────────────────────────────────────────────────────────

/// Collect every source file that is missing the required header.
fn collect_missing() -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    for path in git_tracked_files()? {
        if !is_checked_file(&path) {
            continue;
        }
        if is_skipped_file(&path) {
            continue;
        }
        if !has_header(&path)? {
            missing.push(path);
        }
    }
    missing.sort();
    Ok(missing)
}

/// List all tracked files via `git ls-files`, which inherently respects
/// `.gitignore` and excludes untracked / ignored paths.
fn git_tracked_files() -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--exclude-standard"])
        .output()
        .map_err(|e| format!("failed to run `git ls-files`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`git ls-files` failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();
    Ok(files)
}

/// Whether a path's extension is in the checked set.
fn is_checked_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| CHECKED_EXTENSIONS.contains(&ext))
}

/// Whether a path matches one of the individually skipped files.
/// Paths from `git ls-files` use forward slashes, matching `SKIP_FILES`.
fn is_skipped_file(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    SKIP_FILES.iter().any(|skip| normalized.ends_with(skip))
}

/// Read the first two non-empty lines and check whether they match the header.
fn has_header(path: &Path) -> Result<bool, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(true);
        }
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };

    let mut lines = content.lines();
    let first = match lines.next() {
        Some(line) => line,
        None => return Ok(false),
    };
    let second = match lines.next() {
        Some(line) => line,
        None => return Ok(false),
    };

    Ok(first == HEADER_LINE_1 && second == HEADER_LINE_2)
}

/// Prepend the two-line header to a file, preserving existing content.
fn prepend_header(path: &Path) -> Result<(), String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let mut new_content =
        String::with_capacity(HEADER_LINE_1.len() + HEADER_LINE_2.len() + 3 + content.len());
    new_content.push_str(HEADER_LINE_1);
    new_content.push('\n');
    new_content.push_str(HEADER_LINE_2);
    new_content.push('\n');

    // Add a blank separator line unless the file already starts with one.
    if !content.is_empty() && !content.starts_with('\n') {
        new_content.push('\n');
    }

    new_content.push_str(&content);

    fs::write(path, new_content).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("webui_license_header_tests_{id}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn detects_missing_header() {
        let dir = temp_dir();
        let file = dir.join("missing.rs");
        fs::write(&file, "fn main() {}\n").expect("write");

        assert!(!has_header(&file).expect("has_header"));
    }

    #[test]
    fn detects_present_header() {
        let dir = temp_dir();
        let file = dir.join("present.rs");
        let content = format!("{HEADER_LINE_1}\n{HEADER_LINE_2}\n\nfn main() {{}}\n");
        fs::write(&file, content).expect("write");

        assert!(has_header(&file).expect("has_header"));
    }

    #[test]
    fn prepend_adds_header_and_separator() {
        let dir = temp_dir();
        let file = dir.join("fix_me.rs");
        fs::write(&file, "fn main() {}\n").expect("write");

        prepend_header(&file).expect("prepend");

        let result = fs::read_to_string(&file).expect("read");
        assert!(result.starts_with(HEADER_LINE_1));
        assert!(result.contains(HEADER_LINE_2));
        assert!(result.contains("\n\nfn main()"));
    }

    #[test]
    fn empty_file_gets_header_without_double_blank() {
        let dir = temp_dir();
        let file = dir.join("empty.rs");
        fs::write(&file, "").expect("write");

        prepend_header(&file).expect("prepend");

        let result = fs::read_to_string(&file).expect("read");
        assert_eq!(result, format!("{HEADER_LINE_1}\n{HEADER_LINE_2}\n"));
    }

    #[test]
    fn missing_file_is_treated_as_already_satisfied() {
        let dir = temp_dir();
        let file = dir.join("deleted.rs");

        assert!(has_header(&file).expect("missing file should be ignored"));
    }

    #[test]
    fn extension_filter_works() {
        assert!(is_checked_file(Path::new("foo.rs")));
        assert!(is_checked_file(Path::new("bar.ts")));
        assert!(is_checked_file(Path::new("baz.cs")));
        assert!(is_checked_file(Path::new("qux.h")));
        assert!(is_checked_file(Path::new("quux.js")));
        assert!(is_checked_file(Path::new("schema.proto")));

        assert!(!is_checked_file(Path::new("page.html")));
        assert!(!is_checked_file(Path::new("style.css")));
        assert!(!is_checked_file(Path::new("data.json")));
        assert!(!is_checked_file(Path::new("config.yml")));
        assert!(!is_checked_file(Path::new("doc.xml")));
        assert!(!is_checked_file(Path::new("README.md")));
    }

    #[test]
    fn skip_file_detection() {
        assert!(is_skipped_file(Path::new(
            "crates/webui-ffi/include/webui_ffi.h"
        )));
        assert!(!is_skipped_file(Path::new("crates/webui/src/lib.rs")));
    }

    #[test]
    fn git_tracked_files_returns_files() {
        let files = git_tracked_files().expect("git ls-files should work in repo");
        assert!(!files.is_empty(), "should find tracked files");
        // Cargo.toml is always tracked at the workspace root.
        assert!(
            files.iter().any(|f| f == Path::new("Cargo.toml")),
            "Cargo.toml should be in tracked files"
        );
    }

    #[test]
    fn all_source_files_have_header() {
        // This test runs against the real workspace and will fail if any
        // tracked source file is missing the header — acting as a
        // regression guard.
        let result = check();
        assert!(
            result.is_ok(),
            "License header check failed:\n{}",
            result.unwrap_err()
        );
    }
}
