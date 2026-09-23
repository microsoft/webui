// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::artifacts::paths::Scope;

fn fixture() -> (tempfile::TempDir, Vec<Change>) {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let mut changes = Vec::new();
    for (name, before, after, order) in [
        ("a.rs", Some("original code"), Some("new code"), 0),
        ("obsolete.ts", Some("original obsolete"), None, 0),
        ("created/nested/new.ts", None, Some("new module"), 0),
        ("lock.json", Some("original lock"), Some("new lock"), 2),
        (
            "inventory.json",
            Some("original inventory"),
            Some("new inventory"),
            3,
        ),
    ] {
        let path = dir.path().join(name);
        if let Some(bytes) = before {
            fs::write(&path, bytes).unwrap();
        }
        changes.push(Change {
            destination: scope.destination(&path).unwrap(),
            before: before.map(|v| v.as_bytes().to_vec()),
            after: after.map(|v| v.as_bytes().to_vec()),
            order,
        });
    }
    fs::write(dir.path().join("unrelated"), b"untouched").unwrap();
    (dir, changes)
}

fn assert_originals(root: &Path) {
    for (file, content) in [
        ("a.rs", "original code"),
        ("obsolete.ts", "original obsolete"),
        ("lock.json", "original lock"),
        ("inventory.json", "original inventory"),
        ("unrelated", "untouched"),
    ] {
        assert_eq!(
            fs::read(root.join(file)).unwrap(),
            content.as_bytes(),
            "{file}"
        );
    }
    assert_eq!(
        fs::read_dir(root).unwrap().count(),
        5,
        "new directories or staging leaked"
    );
}

#[test]
fn rollback_restores_every_operation_including_completed_lock_and_inventory() {
    for failure_at in 1..=5 {
        let (dir, changes) = fixture();
        let mut count = 0;
        let result = publish_with(changes, |_| {
            count += 1;
            if count == failure_at {
                Err(std::io::Error::other("injected late publication failure"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err().code(), "ipc-io");
        assert_originals(dir.path());
    }
}

#[test]
fn rename_failure_after_backup_restores_old_inventory_and_all_previous_changes() {
    let (dir, changes) = fixture();
    let mut staged = Vec::new();
    let mut directories = Vec::new();
    prepare(changes, &mut staged, &mut directories).unwrap();
    let last = staged.len() - 1;
    for entry in &mut staged[..last] {
        install(entry).unwrap();
    }
    fs::remove_file(staged[last].directory.join("new")).unwrap();
    let error = install(&mut staged[last]).unwrap_err();
    assert_eq!(
        finish_failure(error, &mut staged, &directories)
            .unwrap_err()
            .code(),
        "ipc-io"
    );
    assert_originals(dir.path());
}

#[test]
fn changed_removal_is_preserved_and_earlier_install_is_rolled_back() {
    let (dir, changes) = fixture();
    let obsolete = dir.path().join("obsolete.ts");
    let mut first = true;
    let result = publish_with(changes, |_| {
        if first {
            first = false;
            fs::write(&obsolete, b"concurrent user changes")?;
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err().code(), "ipc-output-changed");
    assert_eq!(fs::read(obsolete).unwrap(), b"concurrent user changes");
    assert_eq!(fs::read(dir.path().join("a.rs")).unwrap(), b"original code");
    assert_eq!(
        fs::read(dir.path().join("lock.json")).unwrap(),
        b"original lock"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 5);
}

#[test]
fn successful_publication_advances_metadata_after_payloads_and_removals() {
    let (dir, changes) = fixture();
    publish_with(changes, |path| {
        if path.file_name().is_some_and(|name| name == "lock.json") {
            assert_eq!(fs::read(dir.path().join("a.rs"))?, b"new code");
            assert!(!dir.path().join("obsolete.ts").exists());
            assert_eq!(
                fs::read(dir.path().join("created/nested/new.ts"))?,
                b"new module"
            );
            assert_eq!(
                fs::read(dir.path().join("inventory.json"))?,
                b"original inventory"
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        fs::read(dir.path().join("inventory.json")).unwrap(),
        b"new inventory"
    );
    assert_eq!(
        fs::read(dir.path().join("unrelated")).unwrap(),
        b"untouched"
    );
}
