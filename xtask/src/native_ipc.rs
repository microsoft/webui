// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod checks;
mod plan;
mod process;
mod runner;

use plan::Plan;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

const FIXTURE: &str = "crates/webui-desktop/tests/fixtures/native-ipc";
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);
const PREP_TIMEOUT: Duration = Duration::from_secs(180);
const NO_IPC_MODES: &[&str] = &["source", "bundle", "frameless-source", "frameless-bundle"];

struct Options {
    timeout: Duration,
    fail_fast: bool,
    plan: Option<Plan>,
}

struct Runner {
    binary: PathBuf,
    digest: String,
    metadata: Value,
}

struct Inputs {
    source: PathBuf,
    bundle: PathBuf,
    bundle_input: PathBuf,
    source_runner: PathBuf,
    runtime_runner: PathBuf,
    package: Value,
    packaged_binary: PathBuf,
    packaged_resources: PathBuf,
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    let mut object = serde_json::Map::new();
    for (name, value) in fields {
        object.insert(name.to_owned(), value);
    }
    Value::Object(object)
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match execute(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("native IPC acceptance failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut timeout = Duration::from_secs(60);
    let mut fail_fast = false;
    let mut plan = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--timeout" => {
                let seconds = args
                    .next()
                    .ok_or("--timeout requires a positive number of seconds")?
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --timeout: {error}"))?;
                if seconds == 0 {
                    return Err("--timeout must be positive".into());
                }
                timeout = Duration::from_secs(seconds);
                if Instant::now().checked_add(timeout).is_none() {
                    return Err("--timeout exceeds the supported duration".into());
                }
            }
            "--fail-fast" => fail_fast = true,
            "--plan" => {
                plan = Some(Plan::for_name(
                    args.next()
                        .ok_or("--plan requires darwin, win32, or linux")?,
                )?);
            }
            other => return Err(format!("unknown native-ipc option: {other}")),
        }
    }
    Ok(Options {
        timeout,
        fail_fast,
        plan,
    })
}

fn execute(args: &[String]) -> Result<(), String> {
    let options = parse_args(args)?;
    if let Some(plan) = options.plan {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan.json())
                .map_err(|error| format!("platform plan JSON: {error}"))?
        );
        return Ok(());
    }

    let plan = Plan::for_host()?;
    let root = crate::util::workspace_root()?;
    let fixture = root.join(FIXTURE);
    let runs = fixture.join(".runs");
    fs::create_dir_all(&runs).map_err(|error| format!("{}: {error}", runs.display()))?;
    let artifacts = tempfile::Builder::new()
        .prefix("native-")
        .tempdir_in(runs)
        .map_err(|error| format!("native IPC artifacts: {error}"))?
        .keep();
    println!("Artifacts: {}", artifacts.display());

    preflight(&root, &fixture, &artifacts, plan)?;
    let source = build_runner(&root, &artifacts, plan, true)?;
    let runtime = build_runner(&root, &artifacts, plan, false)?;
    checks::metadata(plan, &source.metadata, &runtime.metadata)?;
    let inputs = runner::prepare_inputs(&fixture, &artifacts, plan, &source, &runtime)?;
    let mut reports = Vec::with_capacity(2);
    let mut failures = Vec::new();
    for case in [
        runner::NativeCase {
            mode: "source",
            binary: &source.binary,
            app: &inputs.source,
            digest: &source.digest,
            metadata: &source.metadata,
        },
        runner::NativeCase {
            mode: "packaged",
            binary: &inputs.packaged_binary,
            app: &inputs.packaged_resources,
            digest: &runtime.digest,
            metadata: &runtime.metadata,
        },
    ] {
        let result = if case.mode == "packaged" {
            runner::retire_inputs(&inputs)
                .and_then(|()| runner::native_run(&case, &artifacts, options.timeout))
        } else {
            runner::native_run(&case, &artifacts, options.timeout)
        };
        match result {
            Ok(report) => reports.push(report),
            Err(error) => {
                failures.push(error);
                if options.fail_fast {
                    break;
                }
            }
        }
    }

    let no_ipc = match runner::no_ipc_run(&root, &artifacts, plan, options.timeout) {
        Ok(result) => result,
        Err(error) => {
            failures.push(error);
            object([("status", Value::from("fail"))])
        }
    };
    let passed = failures.is_empty();
    let summary = object([
        ("scope", Value::from("native-ipc-four-flow")),
        ("status", Value::from(if passed { "pass" } else { "fail" })),
        ("profile", Value::from("release")),
        ("source_binary_sha256", Value::from(source.digest)),
        ("packaged_binary_sha256", Value::from(runtime.digest)),
        (
            "bundle_input_removed",
            Value::from(!inputs.bundle_input.exists()),
        ),
        (
            "source_and_staging_removed",
            Value::from(!inputs.source.exists() && !inputs.bundle.exists()),
        ),
        (
            "packaged_source_feature",
            runtime.metadata["source"].clone(),
        ),
        ("no_ipc", no_ipc),
        ("reports", Value::Array(reports)),
        (
            "failures",
            Value::Array(failures.into_iter().map(Value::from).collect()),
        ),
        ("artifacts", Value::from(artifacts.display().to_string())),
        ("sdk_metadata", source.metadata),
        ("package", inputs.package),
        (
            "screenshots",
            Value::from("not applicable: internal protocol fixture, no changed product UI"),
        ),
    ]);
    checks::write_result(&artifacts, &summary)?;
    if summary["status"] != "pass" {
        return Err(format!("see {}", artifacts.join("result.json").display()));
    }
    Ok(())
}

fn preflight(root: &Path, fixture: &Path, artifacts: &Path, plan: Plan) -> Result<(), String> {
    let cli = root.join("target/debug").join(plan.cli());
    let mut generator = Command::new(&cli);
    generator
        .args(["ipc", "generate"])
        .arg(fixture.join("application.proto"))
        .arg("--rust-out")
        .arg(fixture.join("generated/rust"))
        .arg("--ts-out")
        .arg(fixture.join("generated/ts"))
        .arg("--lock")
        .arg(fixture.join("ipc-schema.lock.json"))
        .arg("--check");
    process::run(
        &mut generator,
        "webui-desktop ipc generate --check",
        PREP_TIMEOUT,
    )?;

    let compiler = root.join("packages/webui-desktop/node_modules/typescript/bin/tsc");
    let mut typescript = Command::new("node");
    typescript
        .arg(&compiler)
        .arg("-p")
        .arg(fixture.join("tsconfig.json"));
    process::run(
        &mut typescript,
        "node tsc -p native-ipc/tsconfig.json",
        PREP_TIMEOUT,
    )?;

    let mut tree = process::command(
        "cargo",
        &[
            "tree",
            "-p",
            "microsoft-webui-desktop",
            "--offline",
            "--locked",
            "--no-default-features",
            "--features",
            "native,application-ipc",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
    );
    let output = process::capture(&mut tree, "cargo tree (runtime-only)", PREP_TIMEOUT)?;
    if output.timed_out || !output.status.success() {
        return Err("runtime-only cargo tree failed".into());
    }
    checks::runtime_dependencies(&output.stdout)?;
    fs::write(artifacts.join("runtime-dependencies.log"), output.stdout)
        .map_err(|error| format!("runtime dependency log: {error}"))
}

fn build_runner(root: &Path, artifacts: &Path, plan: Plan, source: bool) -> Result<Runner, String> {
    let features = if source {
        "native,application-ipc,source"
    } else {
        "native,application-ipc"
    };
    let mut build = process::command(
        "cargo",
        &[
            "build",
            "--offline",
            "--locked",
            "--release",
            "-p",
            "microsoft-webui-desktop",
            "--example",
            "webui-native-ipc-fixture",
            "--no-default-features",
            "--features",
            features,
        ],
    );
    process::run(
        &mut build,
        &format!("cargo build native-ipc ({features})"),
        BUILD_TIMEOUT,
    )?;
    let directory = artifacts.join(if source {
        "source-runner"
    } else {
        "runtime-runner"
    });
    fs::create_dir(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let binary = directory.join(plan.executable());
    plan::copy_runner(
        &root.join("target/release/examples").join(plan.executable()),
        &binary,
        plan,
        &root.join("target/release"),
    )?;
    let digest = plan::sha256(&binary)?;
    let mut command = Command::new(&binary);
    command.arg("metadata");
    let output = process::capture(&mut command, "native IPC metadata", PREP_TIMEOUT)?;
    if output.timed_out || !output.status.success() {
        return Err(format!("native IPC metadata failed: {}", output.status));
    }
    let metadata = checks::record(&output.stdout, "NATIVE_METADATA ")?;
    Ok(Runner {
        binary,
        digest,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_args, NO_IPC_MODES};

    #[test]
    fn no_ipc_covers_native_and_frameless_source_and_bundle() {
        assert_eq!(
            NO_IPC_MODES,
            ["source", "bundle", "frameless-source", "frameless-bundle"]
        );
    }

    #[test]
    fn rejects_invalid_native_timeouts_and_plans() {
        for arguments in [
            vec!["--timeout".into(), "0".into()],
            vec!["--timeout".into()],
            vec!["--timeout".into(), u64::MAX.to_string()],
            vec!["--plan".into(), "unsupported".into()],
            vec!["--unknown".into()],
        ] {
            assert!(parse_args(&arguments).is_err());
        }
        let plan = parse_args(&["--plan".into(), "win32".into()]).unwrap();
        assert_eq!(
            plan.plan.unwrap().executable(),
            "webui-native-ipc-fixture.exe"
        );
    }
}
