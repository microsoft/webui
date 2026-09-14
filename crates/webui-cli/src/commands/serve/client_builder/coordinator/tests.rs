// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::commands::common::AppArgs;
use clap::Parser;

#[derive(Parser)]
struct Options {
    #[command(flatten)]
    args: AppArgs,
}

fn shared() -> Arc<Mutex<SharedState>> {
    Arc::new(Mutex::new(SharedState {
        rendered_html: "previous".into(),
        css_files: Default::default(),
        component_assets: Default::default(),
        protocol: None,
        state_data: None,
        token_css: None,
        rebuild_error: None,
        rebuild_pending: true,
        entry: "index.html".into(),
    }))
}

fn queue() -> Queue {
    Queue {
        pending: Mutex::new(Pending {
            dirty: true,
            ..Pending::default()
        }),
        shared: shared(),
        changed: Notify::new(),
    }
}

fn coordinator(root: &Path) -> Coordinator {
    let app = root.join("app");
    let output = root.join("output");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(app.join("index.html"), "<h1>current</h1>").unwrap();
    let app = app.canonicalize().unwrap();
    let config = RenderConfig {
        app_args: Options::parse_from([std::ffi::OsStr::new("test"), app.as_os_str()]).args,
        app_dir: app,
        state_file: None,
        token_file: None,
        component_asset_roots: Vec::new(),
        metafile: None,
        base_path: None,
    };
    Coordinator::new(
        shared(),
        config,
        None,
        Some(LiveReload::new("/reload")),
        &output,
    )
    .unwrap()
}

#[test]
fn changes_coalesce_into_one_followup_and_gate_before_notification() {
    let queue = queue();
    assert_eq!(queue.take().unwrap(), Some(0));
    for _ in 0..10 {
        queue
            .invalidate(&[PathBuf::from("index.ts")], Some(Path::new("builder.mjs")))
            .unwrap();
    }
    assert!(queue.shared.lock().unwrap().rebuild_pending);
    assert_eq!(queue.take().unwrap(), Some(10));
    assert_eq!(queue.take().unwrap(), None);
}

#[test]
fn configuration_changes_require_restart_instead_of_using_cached_module() {
    let queue = queue();
    queue
        .invalidate(
            &[PathBuf::from("builder.mjs")],
            Some(Path::new("builder.mjs")),
        )
        .unwrap();
    assert!(queue.take().unwrap_err().to_string().contains("restart"));
}

#[test]
fn superseded_success_cannot_write_metafile_clear_pending_or_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(dir.path());
    let result = prepare_build(&coordinator.config, None).unwrap();
    let metafile = dir.path().join("metafile.json");
    std::fs::write(&metafile, "previous").unwrap();
    coordinator.config.metafile = Some(metafile.clone());
    let mut reloads = coordinator
        .livereload
        .as_ref()
        .unwrap()
        .sender()
        .subscribe();
    coordinator.queue.invalidate(&[], None).unwrap();
    coordinator
        .finish(
            0,
            Ok(Prepared::Fresh(Box::new(result), None)),
            Duration::ZERO,
            true,
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(metafile).unwrap(), "previous");
    let state = coordinator.queue.shared.lock().unwrap();
    assert!(state.rebuild_pending);
    assert_eq!(state.rendered_html, "previous");
    assert!(reloads.try_recv().is_err());
}

#[test]
fn superseded_failure_cannot_replace_current_failure_but_worker_death_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(dir.path());
    coordinator.queue.invalidate(&[], None).unwrap();
    coordinator
        .finish(
            1,
            Err(AttemptError::Ssr(anyhow::anyhow!("current failure"))),
            Duration::ZERO,
            true,
        )
        .unwrap();
    coordinator
        .finish(
            0,
            Err(AttemptError::Ssr(anyhow::anyhow!("stale failure"))),
            Duration::ZERO,
            true,
        )
        .unwrap();
    assert_eq!(
        coordinator
            .queue
            .shared
            .lock()
            .unwrap()
            .rebuild_error
            .as_deref(),
        Some("current failure")
    );
    let failure = AttemptError::Client(BuildError::Runtime(anyhow::anyhow!("worker exited")));
    assert!(coordinator
        .finish(0, Err(failure), Duration::ZERO, true)
        .is_err());
}

#[test]
fn publication_and_safe_reuse_each_emit_exactly_one_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(dir.path());
    let result = prepare_build(&coordinator.config, None).unwrap();
    let stamp = coordinator.inputs.capture().unwrap();
    let mut reloads = coordinator
        .livereload
        .as_ref()
        .unwrap()
        .sender()
        .subscribe();
    coordinator
        .finish(
            0,
            Ok(Prepared::Fresh(Box::new(result), stamp)),
            Duration::ZERO,
            true,
        )
        .unwrap();
    assert!(reloads.try_recv().is_ok());
    assert!(reloads.try_recv().is_err());
    let protocol = coordinator
        .queue
        .shared
        .lock()
        .unwrap()
        .protocol
        .clone()
        .unwrap();
    coordinator.queue.invalidate(&[], None).unwrap();
    coordinator
        .finish(1, Ok(Prepared::Reused), Duration::ZERO, true)
        .unwrap();
    assert!(reloads.try_recv().is_ok());
    assert!(reloads.try_recv().is_err());
    let state = coordinator.queue.shared.lock().unwrap();
    assert!(!state.rebuild_pending);
    assert!(state.rebuild_error.is_none());
    assert!(Arc::ptr_eq(&protocol, state.protocol.as_ref().unwrap()));
}

#[test]
fn failed_one_time_build_exits_and_stopped_coordinator_never_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(dir.path());
    let failure = AttemptError::Client(BuildError::Build("client failed".into()));
    assert!(coordinator
        .finish(0, Err(failure), Duration::ZERO, false)
        .is_err());
    coordinator.stop().unwrap();
    coordinator
        .finish(0, Ok(Prepared::Reused), Duration::ZERO, false)
        .unwrap();
    assert_eq!(
        coordinator
            .queue
            .shared
            .lock()
            .unwrap()
            .rebuild_error
            .as_deref(),
        Some(
            BuildError::Build("client failed".into())
                .to_string()
                .as_str()
        )
    );
}

#[test]
fn losing_or_gaining_a_fingerprint_during_preparation_requires_retry() {
    let dir = tempfile::tempdir().unwrap();
    let coordinator = coordinator(dir.path());
    let snapshot = coordinator.inputs.capture().unwrap();
    assert!(snapshot.is_some());
    assert!(input_change_detected(&snapshot, &None));
    assert!(input_change_detected(&None, &snapshot));
    assert!(!input_change_detected(&None, &None));
    assert!(!input_change_detected(&snapshot, &snapshot));
}
