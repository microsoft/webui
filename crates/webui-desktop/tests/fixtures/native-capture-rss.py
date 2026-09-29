# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Sample optimized controlled WK capture host RSS; no process is killed by PID name.

Build --release --features native-services --example native-capture first.
WEBUI_CAPTURE_ARTIFACT_DIR=<scratch> python3 \
    crates/webui-desktop/tests/fixtures/native-capture-rss.py
"""

import json
import os
import pathlib
import statistics
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[4]
FIXTURE = pathlib.Path(
    os.environ.get(
        "WEBUI_CAPTURE_BINARY_PATH",
        str(ROOT / "target/release/examples/native-capture"),
    )
)
ARTIFACTS = pathlib.Path(
    os.environ.get(
        "WEBUI_CAPTURE_ARTIFACT_DIR", "/tmp/webui-native-capture-rss"
    )
)
ARTIFACTS.mkdir(parents=True, exist_ok=True)


def processes():
    rows = {}
    output = subprocess.check_output(
        ["/bin/ps", "-axo", "pid,ppid,rss,command"], text=True
    )
    for line in output.splitlines()[1:]:
        parts = line.strip().split(None, 3)
        if len(parts) != 4:
            continue
        try:
            pid, parent, kib = map(int, parts[:3])
        except ValueError:
            continue
        rows[pid] = (parent, kib, parts[3])
    return rows


before = set(processes())
start = time.monotonic()
env = os.environ.copy()
env["WEBUI_CAPTURE_ARTIFACT_DIR"] = str(ARTIFACTS)
process = subprocess.Popen(
    [str(FIXTURE)],
    env=env,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
)
samples = []
new_helpers = set()
while process.poll() is None and time.monotonic() - start < 90:
    rows = processes()
    tree = {process.pid}
    for _ in range(6):
        tree.update(pid for pid, (parent, _, _) in rows.items() if parent in tree)
    new_helpers.update(
        pid
        for pid, (_, _, command) in rows.items()
        if pid not in before and "/com.apple.WebKit." in command
    )
    if process.pid in rows:
        samples.append(
            {
                "at_ms": round((time.monotonic() - start) * 1000, 1),
                "host_kib": rows[process.pid][1],
                "descendant_tree_kib": sum(rows[pid][1] for pid in tree if pid in rows),
                "new_webkit_xpc_cohort_kib": sum(
                    rows[pid][1] for pid in new_helpers if pid in rows
                ),
            }
        )
    time.sleep(0.04)
if process.poll() is None:
    process.kill()
stdout, stderr = process.communicate(timeout=10)
print(stdout)
print(stderr, file=sys.stderr)
valid = [row for row in samples if row["host_kib"] > 0]
if not valid:
    raise SystemExit("WK host RSS could not be sampled")
early = [row for row in valid if row["at_ms"] < 1200] or valid[:5]
steady = valid[-min(12, len(valid)) :]
report = {
    "exit_code": process.returncode,
    "sample_period_ms": 40,
    "sample_count": len(valid),
    "host_early_median_kib": round(statistics.median(s["host_kib"] for s in early)),
    "host_peak_kib": max(s["host_kib"] for s in valid),
    "host_steady_median_kib": round(
        statistics.median(s["host_kib"] for s in steady)
    ),
    "descendant_tree_peak_kib": max(s["descendant_tree_kib"] for s in valid),
    "descendant_tree_steady_median_kib": round(
        statistics.median(s["descendant_tree_kib"] for s in steady)
    ),
    "new_webkit_xpc_cohort_peak_kib": max(
        s["new_webkit_xpc_cohort_kib"] for s in valid
    ),
    "caveat": (
        "WebKit XPC helpers have PPID 1; the new-PID cohort can include other "
        "applications and is not an attributable process tree. Sampled peaks "
        "are lower bounds."
    ),
    "samples": valid,
}
(ARTIFACTS / "rss.json").write_text(json.dumps(report, indent=2) + "\n")
print(
    "RSS_KIB host_early={host_early_median_kib} host_peak={host_peak_kib} "
    "host_steady={host_steady_median_kib} tree_peak={descendant_tree_peak_kib} "
    "tree_steady={descendant_tree_steady_median_kib} "
    "new_xpc_cohort_peak={new_webkit_xpc_cohort_peak_kib}".format(**report)
)
raise SystemExit(process.returncode)
