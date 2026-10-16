// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![cfg(target_os = "linux")]

use std::fs;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use webui_dev_server::{spawn_watcher, WatchConfig};

#[test]
fn read_access_does_not_rebuild_but_writes_do() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("logo.svg");
    fs::write(&source, "before")?;
    let (sender, receiver) = mpsc::channel();
    let _watcher = spawn_watcher(
        WatchConfig {
            paths: vec![root.path().to_path_buf()],
            explicit_files: Vec::new(),
            ignore: Vec::new(),
            debounce: Duration::from_millis(25),
            retry_unchanged_when: None,
        },
        move |paths| {
            let _ = sender.send(paths);
        },
    )?;

    assert_eq!(fs::read_to_string(&source)?, "before");
    let event = receiver.recv_timeout(Duration::from_millis(500));
    assert!(
        matches!(event, Err(RecvTimeoutError::Timeout)),
        "reading a source must not queue a rebuild: {event:?}"
    );

    fs::write(&source, "after")?;
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5))?, vec![source]);
    Ok(())
}
