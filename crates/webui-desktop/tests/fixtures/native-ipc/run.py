# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Run the real native SDK, without an app server or desktop automation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

FIXTURE = Path(__file__).resolve().parent
ROOT = FIXTURE.parents[4]
NO_IPC_MODES = ["source", "bundle", "frameless-source", "frameless-bundle"]


def platform_plan(platform):
    plans = {
        "darwin": ("", "WKWebView", "macos-app", "Contents/MacOS", "Contents/Resources/webui"),
        "win32": (".exe", "WebView2", "windows-portable", "", "resources/webui"),
        "linux": ("", "WebKitGTK", "linux-portable", "", "resources/webui"),
    }
    if platform not in plans:
        raise RuntimeError(f"unsupported native platform: {platform}")
    suffix, backend, target, executable_dir, resources = plans[platform]
    return {
        "platform": platform, "executable": "webui-native-ipc-fixture" + suffix,
        "cli": "webui-desktop" + suffix,
        "native_backend": backend, "package_target": target,
        "executable_dir": executable_dir, "resources": resources,
    }


def capture(command, prefix, *, timeout=180):
    print("+", " ".join(map(str, command)), flush=True)
    result = subprocess.run(list(map(str, command)), cwd=ROOT, timeout=timeout,
                            capture_output=True, text=True)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}); see command diagnostics above")
    records = [json.loads(line.removeprefix(prefix)) for line in result.stdout.splitlines()
               if line.startswith(prefix)]
    if len(records) != 1:
        raise RuntimeError(f"expected one {prefix} record")
    return records[0]


def execute(command, *, timeout=180, env=None):
    print("+", " ".join(map(str, command)), flush=True)
    result = subprocess.run(list(map(str, command)), cwd=ROOT, env=env, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode})")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def assert_clean_native_log(mode, stderr):
    errors = [line for line in stderr.splitlines()
              if line.startswith(("NATIVE_IPC_FAILURE", "NATIVE_IPC_DIAGNOSTIC failure"))]
    if errors:
        raise RuntimeError(f"{mode}: {errors[0]}")


def assert_runtime_dependencies(packages):
    forbidden = {
        "microsoft-webui", "microsoft-webui-parser", "microsoft-webui-discovery",
        "microsoft-webui-desktop-build", "tokio", "rayon", "clap",
    }
    unexpected = set(packages) & forbidden
    if unexpected:
        raise RuntimeError("runtime-only consumer includes: " + ", ".join(sorted(unexpected)))


def check_runtime_dependencies():
    tree = subprocess.check_output([
        "cargo", "tree", "--offline", "--locked", "--manifest-path", str(FIXTURE / "Cargo.toml"),
        "--no-default-features", "--edges", "normal,build", "--prefix", "none", "--format", "{p}",
    ], cwd=ROOT, text=True)
    assert_runtime_dependencies(line.split()[0] for line in tree.splitlines() if line)
    return tree


def no_ipc_run(artifacts, plan, timeout):
    execute([
        "cargo", "build", "--locked", "--release", "-p", "microsoft-webui-desktop",
        "--example", "no-ipc-native", "--no-default-features", "--features", "native,source",
    ], timeout=900)
    directory = artifacts / "no-ipc"
    directory.mkdir()
    binary = directory / plan["executable"]
    suffix = ".exe" if sys.platform == "win32" else ""
    shutil.copy2(ROOT / "target/release/examples" / ("no-ipc-native" + suffix), binary)
    for mode in NO_IPC_MODES:
        result = subprocess.run([str(binary), mode], cwd=directory, timeout=timeout,
                                capture_output=True, text=True)
        (directory / f"{mode}.stdout.log").write_text(result.stdout)
        (directory / f"{mode}.stderr.log").write_text(result.stderr)
        if (result.returncode or result.stdout.count(f"NO_IPC_NATIVE_PASS mode={mode}") != 1
                or "NO_IPC_NATIVE_FAILURE" in result.stderr):
            raise RuntimeError(f"no-IPC {mode} failed: {result.stdout}\n{result.stderr}")
    return {"profile": "release", "modes": NO_IPC_MODES, "status": "pass"}


def native_run(binary, mode, app, artifacts, timeout, digest, metadata):
    env = dict(os.environ, NATIVE_IPC_BINARY_SHA256=digest)
    process = subprocess.Popen(
        [str(binary), mode, str(app)], cwd=artifacts, env=env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        # Only this harness's exact child PID; never process-name termination.
        process.terminate()
        try:
            stdout, stderr = process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate(timeout=5)
    (artifacts / f"{mode}.stdout.log").write_text(stdout)
    (artifacts / f"{mode}.stderr.log").write_text(stderr)
    print(stdout, end="")
    print(stderr, end="", file=sys.stderr)
    if timed_out or process.returncode != 0:
        raise RuntimeError(f"{mode}: pid={process.pid} timeout={timed_out} exit={process.returncode}")
    assert_clean_native_log(mode, stderr)
    reports = [json.loads(line.removeprefix("NATIVE_IPC_RESULT "))
               for line in stdout.splitlines() if line.startswith("NATIVE_IPC_RESULT ")]
    if len(reports) != 1:
        diagnostics = [line for line in stderr.splitlines()
                       if line.startswith("NATIVE_IPC_DIAGNOSTIC failure")]
        detail = diagnostics[-1] if diagnostics else "see native stderr log"
        raise RuntimeError(f"{mode}: expected exactly one final report, received {len(reports)}; {detail}")
    report = reports[0]
    assert report["scope"] == "native-ipc-four-flow" and report["status"] == "pass"
    assert report["mode"] == mode and report["binary_sha256"] == digest
    assert sha256(binary) == digest, "binary changed during run"
    assert report["native_window_closed"]
    assert report["native_backend"] == metadata["native_backend"]
    assert report["platform"] == metadata["platform"]
    assert report["saves"] == 5 and report["renderer_async_labels"] == 4
    assert report["changes"] == 4 and report["confirmations"] == 1
    assert report["startup_notifications"] == 2
    assert report["payload_sizes"] == [0, 16384, 262144]
    assert report["uint64"] == "18446744073709551615"
    assert report["visibility"] in ["visible", "hidden"]
    for condition in [
        "invalid_input", "handler_failure_recovery", "cancellation_recovery",
        "unsubscribe", "void_rpc_completed", "notification_acceptance_not_completion",
        "full_document_navigation", "connection_close_and_recovery", "retired_session_rejected",
        "same_document_generation", "history_fresh_admission", "native_disconnect_before_navigation",
        "history_renderer_callbacks_verified",
    ]:
        assert report[condition] is True, f"{mode}: {condition} was not asserted"
    assert report["persisted_restores"] in [0, 1, 2]
    report["exit_code"] = process.returncode
    report["pid"] = process.pid
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--fail-fast", action="store_true", help="stop immediately after the first failed native mode")
    parser.add_argument("--plan", choices=["darwin", "win32", "linux"],
                        help="print pure path planning only; does not claim a native run")
    args = parser.parse_args()
    if args.plan:
        print(json.dumps(platform_plan(args.plan), indent=2))
        return 0
    plan = platform_plan(sys.platform)
    if args.timeout <= 0:
        raise RuntimeError("--timeout must be positive")
    runs = FIXTURE / ".runs"
    runs.mkdir(exist_ok=True)
    artifacts = Path(tempfile.mkdtemp(prefix="native-", dir=runs))
    print(f"Artifacts: {artifacts}", flush=True)
    cli = ROOT / "target/debug" / plan["cli"]
    command = [
        cli, "ipc", "generate", FIXTURE / "application.proto",
        "--rust-out", FIXTURE / "generated/rust", "--ts-out", FIXTURE / "generated/ts",
        "--lock", FIXTURE / "ipc-schema.lock.json",
    ]
    execute([*command, "--check"])
    execute(["node", ROOT / "packages/webui-desktop/node_modules/typescript/bin/tsc",
             "-p", FIXTURE / "tsconfig.json"])
    (artifacts / "runtime-dependencies.log").write_text(check_runtime_dependencies())
    build = ["cargo", "build", "--offline", "--locked", "--release", "--no-default-features",
             "--manifest-path", FIXTURE / "Cargo.toml", "--target-dir", ROOT / "target"]
    execute([*build, "--features", "source"], timeout=900)
    source_runner = artifacts / "source-runner"
    source_runner.mkdir()
    binary = source_runner / plan["executable"]
    shutil.copy2(ROOT / "target/release" / plan["executable"], binary)
    digest = sha256(binary)
    metadata = capture([binary, "metadata"], "NATIVE_METADATA ")
    for key in ["platform", "native_backend", "package_target"]:
        assert metadata[key] == plan[key], f"SDK {key} differs from platform plan"
    assert all(metadata[key] for key in ["application_ipc", "events", "window_controls"])
    assert metadata["source"] is True
    execute(build, timeout=900)
    runtime_runner = artifacts / "runtime-runner"
    runtime_runner.mkdir()
    runtime_binary = runtime_runner / plan["executable"]
    shutil.copy2(ROOT / "target/release" / plan["executable"], runtime_binary)
    runtime_digest = sha256(runtime_binary)
    runtime_metadata = capture([runtime_binary, "metadata"], "NATIVE_METADATA ")
    assert runtime_metadata == dict(metadata, source=False)
    source = artifacts / "source"
    shutil.copytree(FIXTURE / "web", source)
    execute(["node", FIXTURE / "build.mjs", source])
    # A separate build input can be removed without mutating the source-mode run.
    bundle_input = artifacts / "bundle-input"
    shutil.copytree(source, bundle_input)
    bundle = artifacts / "bundle"
    execute([binary, "build", bundle_input, bundle])
    package = capture([binary, "package", bundle, artifacts / "packages", runtime_binary], "NATIVE_PACKAGE ")
    package_root = Path(package["root"])
    packaged_binary = Path(package["binary"])
    resources = Path(package["resources"])
    assert package["target"] == plan["package_target"]
    assert packaged_binary == package_root / plan["executable_dir"] / plan["executable"]
    assert resources == package_root / plan["resources"]
    assert sha256(packaged_binary) == runtime_digest
    assert (resources / "manifest.webui-desktop.json").is_file()
    shutil.rmtree(bundle_input)
    assert not bundle_input.exists()
    results = []
    failures = []
    no_ipc = {"status": "not-run"}
    for mode, executable, app in [("source", binary, source), ("packaged", packaged_binary, resources)]:
        try:
            if mode == "packaged":
                # Only the package survives: no original web input, staging
                # bundle, or unbundled runner can satisfy resource lookup.
                shutil.rmtree(source)
                shutil.rmtree(bundle)
                shutil.rmtree(source_runner)
                shutil.rmtree(runtime_runner)
            results.append(native_run(
                executable, mode, app, artifacts, args.timeout,
                digest if mode == "source" else runtime_digest,
                metadata if mode == "source" else runtime_metadata,
            ))
        except (RuntimeError, AssertionError) as error:
            failures.append(str(error))
            if args.fail_fast:
                break
    try:
        no_ipc = no_ipc_run(artifacts, plan, args.timeout)
    except (RuntimeError, subprocess.TimeoutExpired) as error:
        failures.append(str(error))
    summary = {
        "scope": "native-ipc-four-flow", "status": "fail" if failures else "pass",
        "profile": "release", "source_binary_sha256": digest,
        "packaged_binary_sha256": runtime_digest,
        "bundle_input_removed": not bundle_input.exists(),
        "source_and_staging_removed": not source.exists() and not bundle.exists(),
        "packaged_source_feature": runtime_metadata["source"], "no_ipc": no_ipc,
        "reports": results, "failures": failures, "artifacts": str(artifacts),
        "sdk_metadata": metadata, "package": package,
        "screenshots": "not applicable: internal protocol fixture, no changed product UI",
    }
    (artifacts / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
