# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import unittest

from analyze import summarize


def trial(label, value, fcp="default"):
    browser_fcp = value if fcp == "default" else fcp
    return {
        "label": label,
        "document_ready_ms": value,
        "wall_started_ns": 1_000_000_000_000,
        "sampled_host_peak_rss_bytes": value * 1024 * 1024,
        "process_tree_cpu_user_ms": value / 2,
        "process_tree_cpu_system_ms": value / 2,
        "close_to_exit_ms": value,
        "records": [{
            "stage": "document-ready",
            "browser": {
                "timeOriginMs": 1_000_000,
                "fcpMs": browser_fcp,
            },
        }],
    }


class SummarizeTests(unittest.TestCase):
    def test_excludes_pair_when_either_host_is_an_outlier(self):
        trials = []
        for pair_number in range(1, 8):
            electron_value = 100 + pair_number
            native_value = 200 + pair_number
            if pair_number == 7:
                native_value = 2_000
            trials.extend([
                trial(f"pair-{pair_number}-electron", electron_value),
                trial(f"pair-{pair_number}-native", native_value),
            ])

        summary = summarize({
            "pairs_requested": 7,
            "trials": trials,
        })

        for metric_name, metric in summary["metrics"].items():
            if metric_name == "process_start_to_window_visible_ms":
                self.assertFalse(metric["available"])
                continue
            self.assertEqual(metric["excluded_pairs"], [7])
            self.assertEqual(metric["retained_pairs"], 6)

    def test_reports_partial_fcp_as_unavailable(self):
        raw = {
            "pairs_requested": 1,
            "trials": [
                trial("pair-1-electron", 100, fcp=10),
                trial("pair-1-native", 200, fcp=None),
            ],
        }
        raw["trials"][1]["records"][0]["browser"]["fcpMs"] = None

        metric = summarize(raw)["metrics"]["process_start_to_fcp_ms"]

        self.assertFalse(metric["available"])
        self.assertEqual(metric["availability"], {"electron": 1, "native": 0})


if __name__ == "__main__":
    unittest.main()
