// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod fixtures;
pub(super) mod support;

use super::*;
use std::fs;
use std::io;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use support::{command, output, wait_for, Running, TestResult};

#[test]
fn no_policy_is_direct_and_timeout_validation_precedes_spawn() -> TestResult {
    assert!(matches!(prepare(None)?, Mode::Direct));
    assert!(matches!(
        prepare(NonZeroU64::new(u64::MAX)),
        Err(Error::InvalidPolicy)
    ));
    let (stop, requests) = supervisor::stop_channel();
    let result = supervisor::supervise(
        Command::new("webui-no-such-shutdown-fixture"),
        Duration::ZERO,
        requests,
    );
    assert!(matches!(result, Err(Error::InvalidPolicy)));
    assert!(!stop.request());
    assert!(validate_policy(Duration::from_secs(1)).is_ok());
    Ok(())
}

#[test]
fn spawn_failure_does_not_fall_back_to_direct_execution() {
    let (_stop, requests) = supervisor::stop_channel();
    assert!(matches!(
        supervisor::supervise(
            Command::new("webui-no-such-shutdown-fixture"),
            Duration::from_secs(1),
            requests,
        ),
        Err(Error::Startup(_))
    ));
}

#[test]
fn graceful_stop_waits_for_completed_writes() -> TestResult {
    let mut running = Running::start("graceful", Duration::from_secs(3))?;
    assert!(running.stop());
    wait_for(&running.path().join("stopping"))?;
    assert!(!running.path().join("complete").exists());
    fs::write(running.path().join("release"), b"finish active work")?;
    assert_eq!(running.finish()?, 0);
    assert_eq!(fs::read(running.path().join("complete"))?, b"completed");
    Ok(())
}

#[test]
fn deadline_kills_a_hung_rust_worker() -> TestResult {
    let mut running = Running::start("hung", Duration::from_millis(150))?;
    let started = Instant::now();
    assert!(running.stop());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[test]
fn second_request_bypasses_a_long_grace_period() -> TestResult {
    let mut running = Running::start("hung", Duration::from_secs(30))?;
    let started = Instant::now();
    assert!(running.stop());
    assert!(running.stop());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::RepeatedRequest))
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[test]
fn disconnected_request_sender_initiates_graceful_shutdown() -> TestResult {
    let mut running = Running::start("graceful", Duration::from_secs(3))?;
    running.disconnect_requests();
    wait_for(&running.path().join("stopping"))?;
    fs::write(running.path().join("release"), b"finish")?;
    assert_eq!(running.finish()?, 0);
    Ok(())
}

#[test]
fn deadline_terminates_rust_descendant_and_grandchild_writers() -> TestResult {
    let mut running = Running::start("tree", Duration::from_millis(150))?;
    wait_for(&running.path().join("writes"))?;
    assert!(running.stop());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    running.assert_writes_stopped()?;
    Ok(())
}

#[test]
fn root_exit_zero_is_not_a_join_attestation() -> TestResult {
    let mut running = Running::start("exit-zero", Duration::from_secs(2))?;
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::ChildExited))
    ));
    Ok(())
}

#[test]
fn root_exit_does_not_leave_grandchild_writers() -> TestResult {
    let mut running = Running::start("root-exits", Duration::from_secs(2))?;
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::ChildExited))
    ));
    running.assert_writes_stopped()?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn surviving_windows_job_members_override_a_join_attestation() -> TestResult {
    let mut running = Running::start("false-attestation", Duration::from_secs(2))?;
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::ChildExited))
    ));
    running.assert_writes_stopped()?;
    Ok(())
}

#[test]
fn terminating_one_scope_leaves_an_independent_scope_untouched() -> TestResult {
    let mut first = Running::start("tree", Duration::from_millis(100))?;
    let mut second = Running::start("tree", Duration::from_millis(100))?;
    assert!(first.stop());
    assert!(matches!(
        first.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    let before = fs::metadata(second.path().join("writes"))?.len();
    thread::sleep(Duration::from_millis(100));
    assert!(fs::metadata(second.path().join("writes"))?.len() > before);
    assert!(second.stop());
    assert!(matches!(
        second.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    Ok(())
}

#[test]
fn ordinary_child_error_status_is_preserved() -> TestResult {
    let mut running = Running::start("error", Duration::from_secs(2))?;
    assert_eq!(running.finish()?, 23);
    Ok(())
}

#[test]
fn child_gate_blocks_application_output_until_parent_release() -> TestResult {
    let output = output()?;
    let mut command = command("graceful", output.path())?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut tree = tree::Tree::spawn(command)?;
    thread::sleep(Duration::from_millis(100));
    assert!(!output.path().join("root.ready").exists());
    tree.release()?;
    wait_for(&output.path().join("root.ready"))?;
    tree.finish()?;
    Ok(())
}

#[test]
fn tree_drop_terminates_all_writers() -> TestResult {
    let output = output()?;
    let mut command = command("tree", output.path())?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut tree = tree::Tree::spawn(command)?;
    tree.release()?;
    wait_for(&output.path().join("root.ready"))?;
    drop(tree);
    let before = fs::read(output.path().join("writes"))?;
    thread::sleep(Duration::from_millis(100));
    assert_eq!(fs::read(output.path().join("writes"))?, before);
    Ok(())
}

#[test]
fn invalid_private_protocol_fails_before_application_output() -> TestResult {
    let output = output()?;
    let mut command = command("graceful", output.path())?;
    command.env(CHILD_ENV, "unsupported-version");
    let mut tree = tree::Tree::spawn(command)?;
    support::wait_until(|| tree.root_exited())?;
    assert!(!output.path().join("root.ready").exists());
    assert_eq!(tree.finish()?.status.code(), Some(91));
    Ok(())
}

#[test]
fn release_error_still_gets_checked_cleanup() -> TestResult {
    let output = output()?;
    let mut command = command("graceful", output.path())?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut tree = tree::Tree::spawn(command)?;
    tree.control.take();
    assert!(tree.release().is_err());
    tree.finish()?;
    assert!(!output.path().join("root.ready").exists());
    Ok(())
}

#[test]
fn real_http_bridge_waits_for_worker_join_and_discards_queued_rebuilds() -> TestResult {
    let mut running = Running::start("http", Duration::from_secs(5))?;
    running.assert_http_ready()?;
    assert!(running.stop());
    wait_for(&running.path().join("joining"))?;
    assert!(!running.path().join("complete").exists());
    fs::write(running.path().join("release"), b"finish")?;
    assert_eq!(running.finish()?, 0);
    assert_eq!(fs::read(running.path().join("complete"))?, b"completed");
    assert_eq!(fs::read_to_string(running.path().join("build-count"))?, "1");
    Ok(())
}

#[test]
fn hung_http_rebuild_join_cannot_trap_supervisor() -> TestResult {
    let mut running = Running::start("http-hung", Duration::from_millis(150))?;
    running.assert_http_ready()?;
    assert!(running.stop());
    assert!(matches!(
        running.finish(),
        Err(Error::Forced(ForcedReason::Deadline))
    ));
    assert!(running.path().join("joining").exists());
    assert!(!running.path().join("complete").exists());
    Ok(())
}

#[test]
fn control_disconnect_stops_http_and_still_joins_rebuild() -> TestResult {
    let output = output()?;
    let mut command = command("http", output.path())?;
    command.env(CHILD_ENV, CHILD_VERSION);
    let mut tree = tree::Tree::spawn(command)?;
    tree.release()?;
    wait_for(&output.path().join("root.ready"))?;
    tree.control.take();
    wait_for(&output.path().join("joining"))?;
    fs::write(output.path().join("release"), b"finish")?;
    support::wait_until(|| tree.root_exited())?;
    assert_eq!(tree.finish()?.status.code(), Some(24));
    assert!(output.path().join("complete").exists());
    assert_eq!(fs::read_to_string(output.path().join("build-count"))?, "1");
    Ok(())
}

#[test]
fn private_marker_does_not_leak_into_build_subprocesses() -> TestResult {
    let mut running = Running::start("marker", Duration::from_secs(2))?;
    assert_eq!(running.finish()?, 0);
    assert_eq!(fs::read(running.path().join("marker-check"))?, b"absent");
    Ok(())
}

#[test]
fn prepare_supervises_current_executable_with_unchanged_arguments() -> TestResult {
    let output = output()?;
    let command = command("prepare", output.path())?;
    // Contain the test parent as an additional safety guard. The actual child
    // enters prepare's nested owned scope and uses its own private stdin pipe.
    let mut tree = tree::Tree::spawn(command)?;
    support::wait_until(|| tree.root_exited())?;
    assert_eq!(tree.finish()?.status.code(), Some(0));
    assert_eq!(
        fs::read(output.path().join("parent.complete"))?,
        b"confirmed"
    );
    assert!(output.path().join("root.ready").exists());
    Ok(())
}

#[test]
fn direct_mode_does_not_install_a_signal_handler() -> TestResult {
    let output = output()?;
    let command = command("direct", output.path())?;
    let mut tree = tree::Tree::spawn(command)?;
    support::wait_until(|| tree.root_exited())?;
    assert_eq!(tree.finish()?.status.code(), Some(0));
    assert!(output.path().join("direct.complete").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn unix_quit_forwards_repeated_requests_and_restores_the_previous_handler() -> TestResult {
    let mut running = Running::start("quit", Duration::from_secs(5))?;
    assert_eq!(running.finish()?, 0);
    assert_eq!(
        fs::read(running.path().join("quit.complete"))?,
        b"forwarded twice and restored"
    );
    Ok(())
}

#[test]
fn diagnostics_are_stable_actionable_plain_and_do_not_duplicate_sources() {
    use std::error::Error as _;
    let errors = [
        (Error::InvalidPolicy, "shutdown-invalid-policy"),
        (
            Error::Startup(io::Error::other("source-detail")),
            "shutdown-startup",
        ),
        (
            Error::Supervision(io::Error::other("source-detail")),
            "shutdown-supervision",
        ),
        (
            Error::UnconfirmedTermination,
            "shutdown-unconfirmed-termination",
        ),
        (
            Error::Signal(ctrlc::Error::MultipleHandlers),
            "shutdown-signal",
        ),
        (
            Error::Control(io::Error::other("source-detail")),
            "shutdown-control",
        ),
        (
            Error::Http(io::Error::other("source-detail")),
            "shutdown-http",
        ),
        (Error::Forced(ForcedReason::Deadline), "shutdown-forced"),
    ];
    for (error, code) in errors {
        assert_eq!(error.code(), code);
        assert!(!error.help().is_empty());
        assert!(!error.help().contains('\u{1b}'));
        assert!(!error.to_string().contains('\u{1b}'));
        assert!(!error.to_string().contains("source-detail"));
        if let Some(source) = error.source() {
            assert!(!source.to_string().is_empty());
        }
    }
}
