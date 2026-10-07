# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Sample total test-process + WebKit child RSS for one native fixture mode.

Run under the same Xvfb/D-Bus/sandbox-enabled container for both modes:
  python3 linux-rss.py /target/debug/deps/linux_local_server-... direct
  python3 linux-rss.py /target/debug/deps/linux_local_server-... ipc
This is an operational footprint sample, not a same-workload speed claim.
"""

import os
import subprocess
import sys
import time


def rss_tree(root):
    result = subprocess.run(
        ["ps", "-eo", "pid=,ppid=,rss="],
        check=True,
        capture_output=True,
        text=True,
    )
    children = {}
    sizes = {}
    for line in result.stdout.splitlines():
        fields = line.split()
        if len(fields) != 3:
            continue
        pid, parent, rss = map(int, fields)
        sizes[pid] = rss
        children.setdefault(parent, []).append(pid)
    stack = [root]
    total = 0
    while stack:
        pid = stack.pop()
        total += sizes.get(pid, 0)
        stack.extend(children.get(pid, ()))
    return total


def main():
    if len(sys.argv) != 3 or sys.argv[2] not in ("direct", "ipc"):
        raise SystemExit("usage: linux-rss.py FIXTURE_BINARY direct|ipc")
    mode = sys.argv[2]
    environment = os.environ.copy()
    environment["WEBUI_LINUX_MEASURE_MODE"] = mode
    process = subprocess.Popen(
        [sys.argv[1], "--exact", "native_local_server_journeys", "--nocapture"],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    samples = []
    while process.poll() is None:
        samples.append(rss_tree(process.pid))
        time.sleep(0.02)
    output, errors = process.communicate()
    if process.returncode:
        sys.stderr.buffer.write(output + errors)
        raise SystemExit(process.returncode)
    if not samples:
        raise SystemExit("process ended before an RSS sample was captured")
    print(
        f"LINUX_RSS mode={mode} samples={len(samples)} "
        f"peak_kib={max(samples)} median_kib={sorted(samples)[len(samples) // 2]}"
    )


if __name__ == "__main__":
    main()
