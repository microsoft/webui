# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Shared desktop contract gate; native source/package runs follow on each OS."""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]


def main():
    if "WEBUI_UPDATE_IPC_FIXTURE" in os.environ:
        raise RuntimeError("acceptance must check committed bindings, not regenerate them")
    commands = [
        ["cargo", "test", "--locked", "-p", "microsoft-webui-desktop-build", "--test", "generate"],
        ["pnpm", "--filter", "@microsoft/webui-desktop", "test"],
        ["node", "packages/webui-desktop/scripts/sync-bootstrap.mjs", "--check",
         "crates/webui-desktop/src/generated/ipc"],
        ["node", "crates/webui-desktop-build/tests/run-rust.mjs"],
        ["node", "crates/webui-desktop-build/tests/run-typescript.mjs"],
        ["node", "packages/webui-desktop/scripts/test-lazy-browser.mjs"],
        ["cargo", "xtask", "test-desktop"],
        [sys.executable, "-m", "unittest", "discover", "-s",
         "crates/webui-desktop/tests", "-p", "test_acceptance.py", "-v"],
        [sys.executable, "-m", "unittest", "discover", "-s",
         "crates/webui-desktop/tests/fixtures/native-ipc", "-p", "test_plan.py", "-v"],
        ["cargo", "build", "--locked", "-p", "microsoft-webui-desktop",
         "--no-default-features", "--features", "cli", "--bin", "webui-desktop"],
    ]
    for command in commands:
        print("+", " ".join(command), flush=True)
        # pnpm's Windows entry point is a batch shim; shell=True is restricted
        # to this fixed, repository-owned command, never caller-provided input.
        subprocess.run(command, cwd=ROOT, check=True,
                       shell=sys.platform == "win32" and command[0] == "pnpm")


if __name__ == "__main__":
    main()
