// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

const MAX_STATE_BYTES: u16 = 4096;
const MAX_TEMP_ATTEMPTS: usize = 16;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Persisted window geometry supplied to and consumed by native backends.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub maximized: bool,
}
/// Available display work area used to reject stale off-screen state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
/// Persistence error.
#[derive(Debug, Error)]
pub enum WindowStateError {
    #[error("failed while {context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse saved window state: {0}")]
    Json(serde_json::Error),
    #[error("saved window state exceeds 4096 bytes (observed at least {size}); help: remove the state file and relaunch")]
    TooLarge { size: u64 },
}
/// Bounded JSON state store in the app-data directory.
#[derive(Clone, Debug)]
pub struct WindowStateStore {
    path: PathBuf,
}
impl WindowStateStore {
    /// Create a state store at an explicit path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    /// Return the platform app-data state path for an app id.
    #[must_use]
    pub fn for_app_id(app_id: &str) -> Self {
        let base = std::env::var_os("WEBUI_APP_DATA_DIR")
            .map(PathBuf::from)
            .or_else(platform_app_data_root)
            .unwrap_or_else(std::env::temp_dir);
        Self::new(base.join(app_id).join("window-state.json"))
    }
    /// Configure native persistence only with an explicit, path-safe identity.
    #[cfg(any(feature = "native", test))]
    pub(crate) fn for_window(
        remember_state: bool,
        app_id: Option<&str>,
    ) -> Result<Option<Self>, WindowStateError> {
        if !remember_state {
            return Ok(None);
        }
        let Some(app_id) = app_id.filter(|id| {
            !id.is_empty()
                && id.len() <= 255
                && !id.ends_with('.')
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        }) else {
            return Err(WindowStateError::Io {
                context: "configuring window-state identity".to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "remember_state requires a path-safe app id; set DesktopFrame::with_app_id with at most 255 ASCII letters, digits, dots, underscores, or hyphens (no trailing dot)",
                ),
            });
        };
        Ok(Some(Self::for_app_id(app_id)))
    }

    /// Atomically replace state using a uniquely created sibling temporary file.
    ///
    /// Readers see either complete version. This guarantees atomic visibility,
    /// not durability across power loss.
    ///
    /// On Windows, the state directory must support POSIX-style atomic rename
    /// (Windows 10 version 1607 or later on a supported filesystem such as NTFS).
    /// Unsupported filesystems return an error rather than weaken this guarantee.
    pub fn save(&self, state: &WindowState) -> Result<(), WindowStateError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| io_error("creating", parent, source))?;
        }
        let data = serde_json::to_vec(state).map_err(WindowStateError::Json)?;
        let (mut file, mut temporary) = create_temporary(&self.path)
            .map_err(|source| io_error("creating temporary state for", &self.path, source))?;
        let result = file.write_all(&data);
        // Close before rename or failure cleanup, including on Windows.
        drop(file);
        result.map_err(|source| io_error("writing", &temporary.path, source))?;
        replace_state(&temporary.path, &self.path)?;
        temporary.committed = true;
        Ok(())
    }
    /// Load visible state, returning `None` for absent or stale geometry.
    ///
    /// Malformed or oversized input returns a typed error.
    pub fn load_valid(
        &self,
        displays: &[DisplayBounds],
    ) -> Result<Option<WindowState>, WindowStateError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error("opening", &self.path, source)),
        };
        let data = read_bounded(file, &self.path)?;
        let state = serde_json::from_slice(&data).map_err(WindowStateError::Json)?;
        Ok(is_visible(&state, displays).then_some(state))
    }
    /// Return the storage path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn replace_state(source: &Path, destination: &Path) -> Result<(), WindowStateError> {
    #[cfg(windows)]
    let result = windows::replace(source, destination);
    #[cfg(not(windows))]
    let result = fs::rename(source, destination);
    result.map_err(|source| io_error("replacing", destination, source))
}

struct TemporaryState {
    path: PathBuf,
    committed: bool,
}

impl Drop for TemporaryState {
    fn drop(&mut self) {
        if !self.committed {
            // Failed writes remove only the file reserved with create_new.
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_temporary(path: &Path) -> std::io::Result<(File, TemporaryState)> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if path.file_name().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "window state path must name a file",
        ));
    }
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".webui-window-state.{}.{sequence}.tmp", std::process::id());
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return Ok((
                    file,
                    TemporaryState {
                        path,
                        committed: false,
                    },
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve a unique temporary window state file",
    ))
}

#[cfg(target_os = "macos")]
fn platform_app_data_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    })
}

#[cfg(windows)]
fn platform_app_data_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_app_data_root() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
}

fn read_bounded(reader: impl Read, path: &Path) -> Result<Vec<u8>, WindowStateError> {
    let limit = u64::from(MAX_STATE_BYTES) + 1;
    let mut data = Vec::with_capacity(usize::from(MAX_STATE_BYTES) + 1);
    reader
        .take(limit)
        .read_to_end(&mut data)
        .map_err(|source| io_error("reading", path, source))?;
    if data.len() > usize::from(MAX_STATE_BYTES) {
        return Err(WindowStateError::TooLarge { size: limit });
    }
    Ok(data)
}

#[cold]
#[inline(never)]
fn io_error(context: &str, path: &Path, source: std::io::Error) -> WindowStateError {
    WindowStateError::Io {
        context: format!("{context} {}", path.display()),
        source,
    }
}

fn is_visible(state: &WindowState, displays: &[DisplayBounds]) -> bool {
    if state.width < 64 || state.height < 64 || state.width > 16_384 || state.height > 16_384 {
        return false;
    }
    displays.iter().any(|display| {
        let right = i64::from(state.x) + i64::from(state.width);
        let bottom = i64::from(state.y) + i64::from(state.height);
        right > i64::from(display.x)
            && bottom > i64::from(display.y)
            && i64::from(state.x) < i64::from(display.x) + i64::from(display.width)
            && i64::from(state.y) < i64::from(display.y) + i64::from(display.height)
    })
}
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_reject_stale_geometry() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = WindowStateStore::new(dir.path().join("state.json"));
        let state = WindowState {
            x: 10,
            y: 10,
            width: 800,
            height: 600,
            maximized: true,
        };
        store.save(&state).unwrap();
        assert_eq!(
            store
                .load_valid(&[DisplayBounds {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080
                }])
                .unwrap(),
            Some(state)
        );
        assert_eq!(
            store
                .load_valid(&[DisplayBounds {
                    x: 2000,
                    y: 0,
                    width: 100,
                    height: 100
                }])
                .unwrap(),
            None
        );
    }

    #[test]
    fn read_limit_applies_to_the_stream_not_a_metadata_snapshot() {
        let mut reader = std::io::Cursor::new(vec![b' '; usize::from(MAX_STATE_BYTES) * 2]);
        assert!(matches!(
            read_bounded(&mut reader, Path::new("state.json")),
            Err(WindowStateError::TooLarge { .. })
        ));
        assert_eq!(reader.position(), u64::from(MAX_STATE_BYTES) + 1);
    }

    #[test]
    fn native_persistence_requires_and_isolates_explicit_identities() {
        assert!(WindowStateStore::for_window(false, None).unwrap().is_none());
        for id in [
            None,
            Some(""),
            Some("../other"),
            Some("/absolute"),
            Some("app."),
        ] {
            assert!(WindowStateStore::for_window(true, id).is_err());
        }
        let first = WindowStateStore::for_window(true, Some("com.example.first"))
            .unwrap()
            .unwrap();
        let second = WindowStateStore::for_window(true, Some("com.example.second"))
            .unwrap()
            .unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first
            .path()
            .ends_with("com.example.first/window-state.json"));
    }

    #[test]
    fn concurrent_replacement_never_exposes_partial_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = WindowStateStore::new(dir.path().join("state.json"));
        let state = WindowState {
            x: 10,
            y: 10,
            width: 800,
            height: 600,
            maximized: false,
        };
        store.save(&state).unwrap();
        std::thread::scope(|scope| {
            for index in 0..4 {
                let store = &store;
                let state = &state;
                scope.spawn(move || {
                    for _ in 0..32 {
                        store
                            .save(&WindowState {
                                x: index,
                                ..state.clone()
                            })
                            .unwrap();
                    }
                });
            }
            for _ in 0..128 {
                let saved: WindowState =
                    serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
                assert_eq!(saved.width, state.width);
                assert_eq!(saved.height, state.height);
            }
        });
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn replacement_keeps_existing_readers_and_new_opens_valid() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = WindowStateStore::new(dir.path().join("state.json"));
        let previous = WindowState {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            maximized: false,
        };
        store.save(&previous).unwrap();
        let reader = File::open(store.path()).unwrap();
        let replacement = WindowState {
            width: 1200,
            maximized: true,
            ..previous
        };
        store.save(&replacement).unwrap();
        let visible: WindowState =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(visible, replacement);
        let original: WindowState = serde_json::from_reader(reader).unwrap();
        assert_eq!(original, previous);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_keeps_existing_target_and_cleans_temporary_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("occupied");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"existing").unwrap();
        let store = WindowStateStore::new(target.clone());
        let state = WindowState {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            maximized: false,
        };
        assert!(store.save(&state).is_err());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"existing");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn committed_temporary_guard_does_not_delete_a_reused_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("state.json");
        let (file, mut temporary) = create_temporary(&target).unwrap();
        drop(file);
        fs::rename(&temporary.path, &target).unwrap();
        temporary.committed = true;
        let reused = temporary.path.clone();
        fs::write(&reused, b"another writer").unwrap();
        drop(temporary);
        assert_eq!(fs::read(reused).unwrap(), b"another writer");
    }
}
