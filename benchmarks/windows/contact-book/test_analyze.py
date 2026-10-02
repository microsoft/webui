# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import unittest
import json
import tempfile
from pathlib import Path

from analyze import summarize
from run import write_themed_state


class SummarizeTests(unittest.TestCase):
    def test_excludes_pair_when_either_host_is_an_outlier(self):
        pairs = []
        for pair_number in range(1, 8):
            electron_value = 100 + pair_number
            webui_value = 200 + pair_number
            if pair_number == 7:
                webui_value = 2_000
            pairs.append(
                {
                    "pair": pair_number,
                    "electron": {
                        metric: electron_value for metric in (
                            "dashboard_tti_ms",
                            "launcher_to_host_main_ms",
                            "host_process_peak_rss_bytes",
                            "process_tree_cpu_ms",
                            "close_to_exit_ms",
                        )
                    },
                    "webui": {
                        metric: webui_value for metric in (
                            "dashboard_tti_ms",
                            "launcher_to_host_main_ms",
                            "host_process_peak_rss_bytes",
                            "process_tree_cpu_ms",
                            "close_to_exit_ms",
                        )
                    },
                }
            )

        summary = summarize(
            {
                "source": "test",
                "host": "Windows",
                "pairs": pairs,
            }
        )

        for metric in summary["metrics"].values():
            self.assertEqual(metric["excluded_pairs"], [7])
            self.assertEqual(metric["retained_pairs"], 6)

    def test_themed_state_preserves_input_and_injects_sorted_css(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state_path = root / "state.json"
            theme_path = root / "tokens.json"
            output_path = root / "themed.json"
            state_path.write_text('{"contacts":[{"id":"1"}]}', encoding="utf-8")
            theme_path.write_text(
                '{"themes":{"light":{"z":"2","a":"1"}}}',
                encoding="utf-8",
            )

            write_themed_state(state_path, theme_path, output_path)

            self.assertEqual(
                json.loads(state_path.read_text(encoding="utf-8")),
                {"contacts": [{"id": "1"}]},
            )
            self.assertEqual(
                json.loads(output_path.read_text(encoding="utf-8")),
                {
                    "contacts": [{"id": "1"}],
                    "tokens": {"light": "--a: 1;\n--z: 2;"},
                },
            )


if __name__ == "__main__":
    unittest.main()
