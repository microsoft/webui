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


def platform_plan(platform):
    plans = {
        "darwin": ("", "", "WKWebView", "macos-app", "Contents/MacOS", "Contents/Resources/webui"),
        "win32": (".exe", ".cmd", "WebView2", "windows-portable", "", "resources/webui"),
        "linux": ("", "", "WebKitGTK", "linux-portable", "", "resources/webui"),
    }
    if platform not in plans:
        raise RuntimeError(f"unsupported native platform: {platform}")
    suffix, shim, backend, target, executable_dir, resources = plans[platform]
    return {
        "platform": platform, "executable": "webui-native-ipc-fixture" + suffix,
        "cli": "webui-desktop" + suffix, "plugin": "protoc-gen-ts_proto" + shim,
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
    parser.add_argument("--skip-build", action="store_true", help="use an already rebuilt native fixture binary")
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
    plugin = ROOT / "packages/webui-desktop/node_modules/.bin" / plan["plugin"]
    command = [
        cli, "ipc", "generate", FIXTURE / "application.proto",
        "--rust-out", FIXTURE / "generated/rust", "--ts-out", FIXTURE / "generated/ts",
        "--lock", FIXTURE / "ipc-schema.lock.json", "--ts-proto-plugin", plugin,
    ]
    execute(command)
    execute([*command, "--check"])
    execute(["node", ROOT / "packages/webui-desktop/node_modules/typescript/bin/tsc",
             "-p", FIXTURE / "tsconfig.json"])
    if not args.skip_build:
        execute(["cargo", "build", "--offline", "--locked", "--manifest-path", FIXTURE / "Cargo.toml",
                 "--target-dir", ROOT / "target"], timeout=300)
    binary = artifacts / plan["executable"]
    shutil.copy2(ROOT / "target/debug" / plan["executable"], binary)
    digest = sha256(binary)
    metadata = capture([binary, "metadata"], "NATIVE_METADATA ")
    for key in ["platform", "native_backend", "package_target"]:
        assert metadata[key] == plan[key], f"SDK {key} differs from platform plan"
    assert all(metadata[key] for key in ["application_ipc", "events", "window_controls"])
    source = artifacts / "source"
    shutil.copytree(FIXTURE / "web", source)
    execute(["node", FIXTURE / "build.mjs", source])
    # A separate build input can be removed without mutating the source-mode run.
    bundle_input = artifacts / "bundle-input"
    shutil.copytree(source, bundle_input)
    bundle = artifacts / "bundle"
    execute([binary, "build", bundle_input, bundle])
    package = capture([binary, "package", bundle, artifacts / "packages"], "NATIVE_PACKAGE ")
    package_root = Path(package["root"])
    packaged_binary = Path(package["binary"])
    resources = Path(package["resources"])
    assert package["target"] == plan["package_target"]
    assert packaged_binary == package_root / plan["executable_dir"] / plan["executable"]
    assert resources == package_root / plan["resources"]
    assert sha256(packaged_binary) == digest
    assert (resources / "manifest.webui-desktop.json").is_file()
    shutil.rmtree(bundle_input)
    assert not bundle_input.exists()
    results = []
    failures = []
    for mode, executable, app in [("source", binary, source), ("packaged", packaged_binary, resources)]:
        try:
            results.append(native_run(executable, mode, app, artifacts, args.timeout, digest, metadata))
        except (RuntimeError, AssertionError) as error:
            failures.append(str(error))
            if args.fail_fast:
                break
    summary = {
        "scope": "native-ipc-four-flow", "status": "fail" if failures else "pass",
        "binary_sha256": digest, "bundle_input_removed": not bundle_input.exists(),
        "reports": results, "failures": failures, "artifacts": str(artifacts),
        "sdk_metadata": metadata, "package": package,
        "screenshots": "not applicable: internal protocol fixture, no changed product UI",
    }
    (artifacts / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
