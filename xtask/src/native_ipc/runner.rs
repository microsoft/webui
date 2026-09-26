// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    checks, plan, process, Inputs, Plan, Runner, BUILD_TIMEOUT, NO_IPC_MODES, PREP_TIMEOUT,
};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub(super) struct NativeCase<'a> {
    pub mode: &'a str,
    pub binary: &'a Path,
    pub app: &'a Path,
    pub digest: &'a str,
    pub metadata: &'a Value,
}

pub(super) fn prepare_inputs(
    fixture: &Path,
    artifacts: &Path,
    plan: Plan,
    source_runner: &Runner,
    runtime_runner: &Runner,
) -> Result<Inputs, String> {
    let source = artifacts.join("source");
    plan::copy_directory(&fixture.join("web"), &source)?;
    let mut bundle_script = Command::new("node");
    bundle_script.arg(fixture.join("build.mjs")).arg(&source);
    process::run(
        &mut bundle_script,
        "node native-ipc/build.mjs",
        PREP_TIMEOUT,
    )?;
    let bundle_input = artifacts.join("bundle-input");
    plan::copy_directory(&source, &bundle_input)?;
    let bundle = artifacts.join("bundle");
    let mut build = Command::new(&source_runner.binary);
    build.arg("build").arg(&bundle_input).arg(&bundle);
    process::run(&mut build, "native IPC bundle build", PREP_TIMEOUT)?;
    let mut package_command = Command::new(&source_runner.binary);
    package_command
        .arg("package")
        .arg(&bundle)
        .arg(artifacts.join("packages"))
        .arg(&runtime_runner.binary);
    let output = process::capture(&mut package_command, "native IPC package", PREP_TIMEOUT)?;
    if output.timed_out || !output.status.success() {
        return Err(format!("native IPC package failed: {}", output.status));
    }
    let package = checks::record(&output.stdout, "NATIVE_PACKAGE ")?;
    let packaged = checks::package(
        plan,
        &package,
        &runtime_runner.binary,
        &runtime_runner.digest,
        runtime_runner
            .binary
            .parent()
            .ok_or("runtime runner has no parent")?,
    )?;
    fs::remove_dir_all(&bundle_input)
        .map_err(|error| format!("remove {}: {error}", bundle_input.display()))?;
    if bundle_input.exists() {
        return Err("bundle input still exists after removal".into());
    }
    Ok(Inputs {
        source,
        bundle,
        bundle_input,
        source_runner: source_runner
            .binary
            .parent()
            .ok_or("source runner has no parent")?
            .into(),
        runtime_runner: runtime_runner
            .binary
            .parent()
            .ok_or("runtime runner has no parent")?
            .into(),
        package,
        packaged_binary: packaged.binary,
        packaged_resources: packaged.resources,
    })
}

pub(super) fn retire_inputs(inputs: &Inputs) -> Result<(), String> {
    for path in [
        &inputs.source,
        &inputs.bundle,
        &inputs.source_runner,
        &inputs.runtime_runner,
    ] {
        fs::remove_dir_all(path).map_err(|error| format!("remove {}: {error}", path.display()))?;
    }
    Ok(())
}

pub(super) fn native_run(
    case: &NativeCase<'_>,
    artifacts: &Path,
    timeout: Duration,
) -> Result<Value, String> {
    let mut command = Command::new(case.binary);
    command
        .arg(case.mode)
        .arg(case.app)
        .current_dir(artifacts)
        .env("NATIVE_IPC_BINARY_SHA256", case.digest);
    let output = process::capture(&mut command, &format!("native IPC {}", case.mode), timeout)?;
    fs::write(
        artifacts.join(format!("{}.stdout.log", case.mode)),
        &output.stdout,
    )
    .map_err(|error| format!("{} stdout log: {error}", case.mode))?;
    fs::write(
        artifacts.join(format!("{}.stderr.log", case.mode)),
        &output.stderr,
    )
    .map_err(|error| format!("{} stderr log: {error}", case.mode))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "{}: pid={} timeout={} exit={}",
            case.mode, output.pid, output.timed_out, output.status
        ));
    }
    checks::native_log(case.mode, &output.stderr)?;
    let mut report = checks::record(&output.stdout, "NATIVE_IPC_RESULT ")?;
    checks::report(&report, case.mode, case.digest, case.metadata)?;
    if plan::sha256(case.binary)? != case.digest {
        return Err(format!("{}: binary changed during run", case.mode));
    }
    report["exit_code"] = Value::from(
        output
            .status
            .code()
            .ok_or_else(|| format!("{}: process has no exit code", case.mode))?,
    );
    report["pid"] = Value::from(output.pid);
    Ok(report)
}

pub(super) fn no_ipc_run(
    root: &Path,
    artifacts: &Path,
    plan: Plan,
    timeout: Duration,
) -> Result<Value, String> {
    let mut build = process::command(
        "cargo",
        &[
            "build",
            "--locked",
            "--release",
            "-p",
            "microsoft-webui-desktop",
            "--example",
            "no-ipc-native",
            "--no-default-features",
            "--features",
            "native,source",
        ],
    );
    process::run(&mut build, "cargo build no-ipc-native", BUILD_TIMEOUT)?;
    let directory = artifacts.join("no-ipc");
    fs::create_dir(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let binary = directory.join(plan.executable());
    plan::copy_runner(
        &root
            .join("target/release/examples")
            .join(plan.example("no-ipc-native")),
        &binary,
        plan,
        &root.join("target/release"),
    )?;
    for mode in NO_IPC_MODES {
        let mut command = Command::new(&binary);
        command.arg(mode).current_dir(&directory);
        let output = process::capture(&mut command, &format!("no-IPC {mode}"), timeout)?;
        fs::write(directory.join(format!("{mode}.stdout.log")), &output.stdout)
            .map_err(|error| format!("{mode} stdout log: {error}"))?;
        fs::write(directory.join(format!("{mode}.stderr.log")), &output.stderr)
            .map_err(|error| format!("{mode} stderr log: {error}"))?;
        let expected = format!("NO_IPC_NATIVE_PASS mode={mode}");
        if output.timed_out
            || !output.status.success()
            || output
                .stdout
                .lines()
                .filter(|line| *line == expected)
                .count()
                != 1
            || output.stderr.contains("NO_IPC_NATIVE_FAILURE")
        {
            return Err(format!(
                "no-IPC {mode} failed: {}{}",
                output.stdout, output.stderr
            ));
        }
    }
    Ok(super::object([
        ("profile", Value::from("release")),
        (
            "modes",
            Value::Array(NO_IPC_MODES.iter().map(|mode| Value::from(*mode)).collect()),
        ),
        ("status", Value::from("pass")),
    ]))
}
