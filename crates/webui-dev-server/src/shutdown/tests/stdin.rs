// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{fixture_command, fixture_output, wait_for, wait_until, TestResult};
use std::fs;
use std::io::Write;
use std::process::{Child, ExitStatus, Stdio};
use tempfile::TempDir;

#[test]
fn stdin_s_stops_gracefully_without_waiting_for_eof_or_stealing_the_child_gate() -> TestResult {
    let (output, mut child) = start("stdin-enabled")?;
    // Send before the private worker is ready. Both parent and re-executed
    // worker call prepare_with_stdin(true), but only the parent may read S.
    child
        .stdin
        .as_mut()
        .ok_or("missing controller pipe")?
        .write_all(b"S")?;
    assert!(finish(&mut child)?.success());
    assert!(child.stdin.is_some(), "controller pipe must remain open");
    assert_eq!(fs::read(output.path().join("complete"))?, b"done");
    Ok(())
}

#[test]
fn controller_eof_requests_graceful_stop() -> TestResult {
    let (output, mut child) = start("stdin-enabled")?;
    wait_for(&output.path().join("ready"))?;
    drop(child.stdin.take());
    assert!(finish(&mut child)?.success());
    assert_eq!(fs::read(output.path().join("complete"))?, b"done");
    Ok(())
}

#[test]
fn malformed_stdin_reports_error_after_graceful_cleanup() -> TestResult {
    let (output, mut child) = start("stdin-enabled")?;
    child
        .stdin
        .as_mut()
        .ok_or("missing controller pipe")?
        .write_all(b"?")?;
    assert_eq!(finish(&mut child)?.code(), Some(74));
    assert_eq!(fs::read(output.path().join("complete"))?, b"done");
    let error = fs::read_to_string(output.path().join("result"))?;
    assert!(error.contains("InvalidData"), "{error}");
    assert!(error.contains("send ASCII S or close stdin"), "{error}");
    Ok(())
}

#[test]
fn malformed_stdin_is_not_lost_when_cleanup_requires_force() -> TestResult {
    let (output, mut child) = start("stdin-hung")?;
    child
        .stdin
        .as_mut()
        .ok_or("missing controller pipe")?
        .write_all(b"?")?;
    assert_eq!(finish(&mut child)?.code(), Some(74));
    assert!(output.path().join("stopping").exists());
    let error = fs::read_to_string(output.path().join("result"))?;
    assert!(
        error.contains("invalid supervisor stdin control byte"),
        "{error}"
    );
    assert!(error.contains("deadline expired"), "{error}");
    Ok(())
}

#[test]
fn disabled_control_does_not_stop_on_null_stdin() -> TestResult {
    let output = fixture_output()?;
    let mut child = fixture_command("stdin-disabled", output.path())?
        .stdin(Stdio::null())
        .spawn()?;
    assert!(finish(&mut child)?.success());
    assert!(!output.path().join("complete").exists());
    Ok(())
}

#[test]
fn natural_completion_does_not_join_a_reader_waiting_on_an_open_controller() -> TestResult {
    let (output, mut child) = start("stdin-complete")?;
    assert_eq!(finish(&mut child)?.code(), Some(37));
    assert!(child.stdin.is_some(), "controller pipe must remain open");
    assert_eq!(fs::read_to_string(output.path().join("result"))?, "Ok(37)");
    Ok(())
}

fn start(mode: &str) -> TestResult<(TempDir, Child)> {
    let output = fixture_output()?;
    let child = fixture_command(mode, output.path())?
        .stdin(Stdio::piped())
        .spawn()?;
    Ok((output, child))
}

fn finish(child: &mut Child) -> TestResult<ExitStatus> {
    wait_until(|| Ok(child.try_wait()?.is_some()))?;
    // Child::wait closes stdin; retain it here to verify that normal completion
    // never needs controller EOF, even after the status has been collected.
    child
        .try_wait()?
        .ok_or_else(|| "fixture exited without a status".into())
}
