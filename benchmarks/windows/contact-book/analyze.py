# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import argparse
import json
import statistics
from pathlib import Path

METRICS = (
    "dashboard_tti_ms",
    "process_start_to_window_visible_ms",
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


def summarize(raw):
    pairs = raw["pairs"]
    metrics = {}
    for metric in METRICS:
        electron = [pair["electron"][metric] for pair in pairs]
        webui = [pair["webui"][metric] for pair in pairs]
        electron_scores = modified_z_scores(electron)
        webui_scores = modified_z_scores(webui)
        excluded = [
            pair["pair"]
            for pair, electron_score, webui_score in zip(
                pairs, electron_scores, webui_scores
            )
            if abs(electron_score) > 3.5 or abs(webui_score) > 3.5
        ]
        retained = [
            pair
            for pair in pairs
            if pair["pair"] not in set(excluded)
        ]
        metrics[metric] = {
            "electron": {
                "median": statistics.median(
                    [pair["electron"][metric] for pair in retained]
                ),
                "unit": "MiB" if metric == "host_process_peak_rss_bytes" else "ms",
            },
            "webui": {
                "median": statistics.median(
                    [pair["webui"][metric] for pair in retained]
                ),
                "unit": "MiB" if metric == "host_process_peak_rss_bytes" else "ms",
            },
            "retained_pairs": len(retained),
            "excluded_pairs": excluded,
            "threshold": 3.5,
        }
        if metric == "host_process_peak_rss_bytes":
            metrics[metric]["electron"]["median"] /= 1024 * 1024
            metrics[metric]["webui"]["median"] /= 1024 * 1024

    return {
        "schema": raw["schema"],
        "source": raw["source"],
        "host": raw["host"],
        "pair_count": len(pairs),
        "metrics": metrics,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("raw", type=Path)
    parser.add_argument("summary", type=Path)
    args = parser.parse_args()
    raw = json.loads(args.raw.read_text(encoding="utf-8"))
    args.summary.write_text(
        json.dumps(summarize(raw), indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
