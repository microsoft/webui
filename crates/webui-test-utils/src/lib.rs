//! Test utilities for WebUI framework.
//!
//! This crate provides testing helpers and should only be used in test code.

use std::fs;
use std::{collections::HashMap, path::PathBuf};
use tempfile::TempDir;

/// A macro that wraps `serde_json::json!` but allows bypassing clippy::disallowed_methods.
///
/// This macro should only be used in test code.
#[macro_export]
macro_rules! test_json {
    ($($json:tt)+) => {{
        #[allow(clippy::disallowed_methods)]
        let value = serde_json::json!($($json)+);
        value
    }};
}

/// A test file system that manages temporary files and directories
pub struct TestFileSystem {
    files: HashMap<String, PathBuf>,

    // Keep directories alive for the lifetime of this struct
    _temp_dirs: Vec<TempDir>,
}

impl Default for TestFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TestFileSystem {
    /// Create a new empty test file system
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            _temp_dirs: Vec::new(),
        }
    }

    /// Add a file to the test file system at the specified path
    pub fn add_file(&mut self, path: &str, content: &str) -> PathBuf {
        // Create a new temporary directory for this file
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");

        // Parse the path to separate directories and filename
        let path_parts: Vec<&str> = path.split('/').collect();
        let filename = path_parts.last().expect("Path must contain a filename");

        // Create the file within the temporary directory
        let file_path = temp_dir.path().join(filename);
        fs::write(&file_path, content).expect("Failed to write content to file");

        // Store the path and keep the directory alive
        self.files.insert(path.to_string(), file_path.clone());
        self._temp_dirs.push(temp_dir);

        // Return a reference to the stored path
        self.files
            .get(path)
            .expect("File path not found in the test file system");

        // Return the path by value (clone it)
        file_path
    }
}
