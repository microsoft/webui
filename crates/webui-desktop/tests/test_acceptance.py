# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Guard the shared acceptance entry point without starting native applications."""
import subprocess
import unittest
from unittest.mock import patch

import acceptance


class AcceptanceTests(unittest.TestCase):
    def test_generation_checks_precede_consumers_and_never_update_fixtures(self):
        with patch.dict(acceptance.os.environ, {}, clear=True), \
                patch.object(acceptance.subprocess, "run") as run:
            acceptance.main()
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(commands[0], [
            "cargo", "test", "--locked", "-p", "microsoft-webui-desktop-build",
            "--test", "generate",
        ])
        sync = next(index for index, command in enumerate(commands)
                    if "packages/webui-desktop/scripts/sync-bootstrap.mjs" in command)
        rust = next(index for index, command in enumerate(commands)
                    if "crates/webui-desktop-build/tests/run-rust.mjs" in command)
        self.assertIn("--check", commands[sync])
        self.assertLess(sync, rust)
        self.assertIn(["cargo", "xtask", "test-desktop"], commands)
        self.assertTrue(all(call.kwargs["check"] for call in run.call_args_list))
        self.assertTrue(all("--write" not in command for command in commands))

    def test_update_environment_is_rejected_even_when_empty(self):
        with patch.dict(acceptance.os.environ, {"WEBUI_UPDATE_IPC_FIXTURE": ""}), \
                patch.object(acceptance.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "not regenerate"):
                acceptance.main()
        run.assert_not_called()

    def test_failed_gate_does_not_run_later_consumers(self):
        with patch.dict(acceptance.os.environ, {}, clear=True), \
                patch.object(acceptance.subprocess, "run",
                             side_effect=subprocess.CalledProcessError(1, "generator")) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                acceptance.main()
        self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
