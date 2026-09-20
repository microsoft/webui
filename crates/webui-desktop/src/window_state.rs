// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
    #[error("saved window state is too large: {size} bytes (max 4096); help: remove the state file and relaunch")]
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
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(std::env::temp_dir);
        Self::new(base.join(app_id).join("window-state.json"))
    }
    /// Persist one state atomically enough for a single-process desktop app.
    pub fn save(&self, state: &WindowState) -> Result<(), WindowStateError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| WindowStateError::Io {
                context: format!("creating {}", parent.display()),
                source,
            })?;
        }
        let data = serde_json::to_vec(state).map_err(WindowStateError::Json)?;
        fs::write(&self.path, data).map_err(|source| WindowStateError::Io {
            context: format!("writing {}", self.path.display()),
            source,
        })
    }
    /// Load valid visible state or return `None` for absent, malformed, or stale geometry.
    pub fn load_valid(
        &self,
        displays: &[DisplayBounds],
    ) -> Result<Option<WindowState>, WindowStateError> {
        let metadata = match fs::metadata(&self.path) {
            Ok(value) => value,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(WindowStateError::Io {
                    context: format!("reading {}", self.path.display()),
                    source,
                })
            }
        };
        if metadata.len() > 4096 {
            return Err(WindowStateError::TooLarge {
                size: metadata.len(),
            });
        }
        let mut text = String::with_capacity(4096);
        fs::File::open(&self.path)
            .map_err(|source| WindowStateError::Io {
                context: format!("opening {}", self.path.display()),
                source,
            })?
            .read_to_string(&mut text)
            .map_err(|source| WindowStateError::Io {
                context: format!("reading {}", self.path.display()),
                source,
            })?;
        let state = serde_json::from_str(&text).map_err(WindowStateError::Json)?;
        Ok(is_visible(&state, displays).then_some(state))
    }
    /// Return the storage path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
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
}
