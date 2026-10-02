# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import argparse
import json
import statistics
from pathlib import Path

HOSTS = ("electron", "native")
METRICS = (
    "dashboard_tti_ms",
    "process_start_to_fcp_ms",
    "host_process_peak_rss_bytes",
    "process_tree_cpu_ms",
    "close_to_exit_ms",
)


def modified_z_scores(values):
    median = statistics.median(values)
    deviations = [abs(value - median) for value in values]
    mad = statistics.median(deviations)
    if mad == 0:
        return [0.0] * len(values)
    return [0.6745 * (value - median) / mad for value in values]


def document_ready(trial):
    return next(
        record for record in trial["records"]
        if record["stage"] == "document-ready"
    )


def metric_value(trial, metric):
    if metric == "dashboard_tti_ms":
        return trial["document_ready_ms"]
    if metric == "process_start_to_fcp_ms":
        browser = document_ready(trial)["browser"]
        fcp_ms = browser["fcpMs"]
        if fcp_ms is None:
            return None
        return (
            browser["timeOriginMs"]
            + fcp_ms
            - trial["wall_started_ns"] / 1_000_000
        )
    if metric == "host_process_peak_rss_bytes":
        return trial["sampled_host_peak_rss_bytes"]
    if metric == "process_tree_cpu_ms":
        return (
            trial["process_tree_cpu_user_ms"]
            + trial["process_tree_cpu_system_ms"]
        )
    if metric == "close_to_exit_ms":
        return trial["close_to_exit_ms"]
    raise KeyError(metric)


def measured_pairs(raw):
    trials = {
        trial["label"]: trial
        for trial in raw["trials"]
        if trial["label"].startswith("pair-")
    }
    pair_count = raw["pairs_requested"]
    return [
        {
            "pair": pair_number,
            **{
                host: trials[f"pair-{pair_number}-{host}"]
                for host in HOSTS
            },
        }
        for pair_number in range(1, pair_count + 1)
    ]


def summarize_metric(pairs, metric):
    values = {
        host: [metric_value(pair[host], metric) for pair in pairs]
        for host in HOSTS
    }
    availability = {
        host: sum(value is not None for value in host_values)
        for host, host_values in values.items()
    }
    if any(count != len(pairs) for count in availability.values()):
        return {
            "available": False,
            "availability": availability,
            "reason": "FCP was not exposed for every measured pair",
        }

    scores = {
        host: modified_z_scores(host_values)
        for host, host_values in values.items()
    }
    excluded = [
        pair["pair"]
        for index, pair in enumerate(pairs)
        if any(abs(scores[host][index]) > 3.5 for host in HOSTS)
    ]
    excluded_set = set(excluded)
    retained = [
        (index, pair)
        for index, pair in enumerate(pairs)
        if pair["pair"] not in excluded_set
    ]
    unit = "MiB" if metric == "host_process_peak_rss_bytes" else "ms"
    scale = 1 / (1024 * 1024) if unit == "MiB" else 1
    return {
        "available": True,
        "electron": {
            "median": statistics.median(
                values["electron"][index] for index, _ in retained
            ) * scale,
            "unit": unit,
        },
        "webui": {
            "median": statistics.median(
                values["native"][index] for index, _ in retained
            ) * scale,
            "unit": unit,
        },
        "retained_pairs": len(retained),
        "excluded_pairs": excluded,
        "threshold": 3.5,
    }


def summarize(raw):
    pairs = measured_pairs(raw)
    metrics = {
        metric: summarize_metric(pairs, metric)
        for metric in METRICS
    }
    metrics["process_start_to_window_visible_ms"] = {
        "available": False,
        "reason": (
            "The macOS harness has no equivalent visible-window receipt. "
            "Electron emits Ready before BrowserWindow construction, and the "
            "native Ready event is not compositing proof."
        ),
    }
    return {
        "schema": 1,
        "source": "benchmarks/macos/contact-book",
        "host": "macOS",
        "pair_count": len(pairs),
        "warmups_per_host": 1,
        "outlier_policy": {
            "method": "per-metric paired exclusion using modified z-score",
            "threshold": 3.5,
            "rule": (
                "Exclude a pair when either host exceeds the absolute "
                "modified z-score threshold; no samples are hand-selected."
            ),
        },
        "metrics": metrics,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("raw", type=Path)
    parser.add_argument("summary", type=Path)
    args = parser.parse_args()
    raw = json.loads(args.raw.read_text(encoding="utf-8"))
    args.summary.write_text(
        json.dumps(summarize(raw), indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
