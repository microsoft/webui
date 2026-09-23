// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebView2 storage belongs to the frame application, never the runner filename.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct BrowserProfile {
    pub(crate) path: PathBuf,
    temporary: bool,
}

impl BrowserProfile {
    pub(crate) fn create(base: &Path, app_id: Option<&str>) -> io::Result<Self> {
        if let Some(id) = app_id {
            let path = identity_path(base, id)?;
            std::fs::create_dir_all(&path)?;
            return Ok(Self {
                path,
                temporary: false,
            });
        }
        // No identity means no authority to reopen any previous app's storage.
        // Atomic directory creation prevents collisions even after PID reuse.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = base.join("anonymous");
        std::fs::create_dir_all(&base)?;
        for _ in 0..128 {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{}-{serial}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        temporary: true,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot allocate an isolated WebView2 profile; set DesktopAppBuilder::app_id",
        ))
    }
}

impl Drop for BrowserProfile {
    fn drop(&mut self) {
        if self.temporary {
            // WebView2 helpers may still hold files after controller shutdown.
            // Such leftovers are never reopened, even by the same runner.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn identity_path(base: &Path, id: &str) -> io::Result<PathBuf> {
    if id.is_empty()
        || id.len() > 255
        || id.ends_with('.')
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "app_id must contain 1–255 ASCII letters, digits, dots, underscores or hyphens, with no trailing dot",
        ));
    }
    // Exact byte encoding avoids case-insensitive, reserved-device-name and
    // sanitization collisions. Chunking keeps each path component under 255.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut path = base.join("identified");
    for chunk in id.as_bytes().chunks(64) {
        let mut component = String::with_capacity(chunk.len() * 2);
        for byte in chunk {
            component.push(char::from(HEX[usize::from(byte >> 4)]));
            component.push(char::from(HEX[usize::from(byte & 15)]));
        }
        path.push(component);
    }
    Ok(path.join("WebView2"))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn stable_identity_is_exact_and_independent_of_runner_name() {
        let base = tempfile::tempdir().unwrap();
        let a = BrowserProfile::create(base.path(), Some("com.example.App")).unwrap();
        let same = BrowserProfile::create(base.path(), Some("com.example.App")).unwrap();
        let b = BrowserProfile::create(base.path(), Some("com.example.app")).unwrap();
        assert_eq!(a.path, same.path);
        assert_ne!(a.path, b.path);
        assert!(BrowserProfile::create(base.path(), Some(&"a".repeat(255))).is_ok());
        for id in [
            "", ".", "..", "../app", "app/", "app\\", "C:", "app.", "app?",
        ] {
            assert!(
                BrowserProfile::create(base.path(), Some(id)).is_err(),
                "{id}"
            );
        }
        assert!(BrowserProfile::create(base.path(), Some(&"a".repeat(256))).is_err());
    }

    #[test]
    fn unidentified_frames_never_share_storage_and_cleanup_is_owned() {
        let base = tempfile::tempdir().unwrap();
        let a = BrowserProfile::create(base.path(), None).unwrap();
        std::fs::write(a.path.join("storage"), "private").unwrap();
        let b = BrowserProfile::create(base.path(), None).unwrap();
        assert_ne!(a.path, b.path);
        assert!(!b.path.join("storage").exists());
        let path = a.path.clone();
        drop(a);
        assert!(!path.exists());
        assert!(b.path.exists());
    }
}
