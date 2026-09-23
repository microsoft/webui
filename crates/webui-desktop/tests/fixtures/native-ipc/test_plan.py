# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Pure planning tests only: these do not exercise any native adapter."""
import unittest
from pathlib import PureWindowsPath, PurePosixPath
from run import NO_IPC_MODES, assert_clean_native_log, assert_runtime_dependencies, platform_plan


class PlatformPlanTests(unittest.TestCase):
    def test_no_ipc_modes_cover_native_and_frameless_source_and_bundle(self):
        self.assertEqual(NO_IPC_MODES, ["source", "bundle", "frameless-source", "frameless-bundle"])

    def test_runtime_consumer_rejects_source_and_tooling_dependencies(self):
        for package in ["microsoft-webui", "microsoft-webui-parser",
                        "microsoft-webui-discovery", "microsoft-webui-desktop-build",
                        "tokio", "rayon", "clap"]:
            with self.subTest(package=package), self.assertRaisesRegex(RuntimeError, package):
                assert_runtime_dependencies(["microsoft-webui-desktop", package])

    def test_runtime_consumer_accepts_native_and_ipc_dependencies(self):
        assert_runtime_dependencies(["microsoft-webui-desktop", "microsoft-webui-handler",
                                     "prost", "futures-channel", "objc2", "webkit6", "webview2-com"])

    def test_windows_portable(self):
        plan = platform_plan("win32")
        self.assertEqual(plan["cli"], "webui-desktop.exe")
        self.assertEqual(plan["native_backend"], "WebView2")
        self.assertEqual(plan["package_target"], "windows-portable")
        self.assertEqual(PureWindowsPath("C:/package") / plan["resources"],
                         PureWindowsPath("C:/package/resources/webui"))
        self.assertEqual(PureWindowsPath("C:/package") / plan["executable_dir"] / plan["executable"],
                         PureWindowsPath("C:/package/webui-native-ipc-fixture.exe"))

    def test_linux_portable(self):
        plan = platform_plan("linux")
        self.assertEqual(plan["package_target"], "linux-portable")
        self.assertEqual(plan["native_backend"], "WebKitGTK")
        self.assertEqual(PurePosixPath("/package") / plan["resources"],
                         PurePosixPath("/package/resources/webui"))
        self.assertEqual(plan["executable"], "webui-native-ipc-fixture")

    def test_macos_bundle(self):
        plan = platform_plan("darwin")
        self.assertEqual(plan["package_target"], "macos-app")
        self.assertEqual(plan["executable_dir"], "Contents/MacOS")
        self.assertEqual(plan["resources"], "Contents/Resources/webui")

    def test_unsupported_platform_fails(self):
        with self.assertRaisesRegex(RuntimeError, "unsupported native platform"):
            platform_plan("unsupported")

    def test_unhandled_rejection_fails_even_alongside_pass_shaped_output(self):
        log = ('NATIVE_IPC_DIAGNOSTIC failure rejection Error: code=transport\n'
               'NATIVE_IPC_RESULT {"status":"pass"}\n')
        with self.assertRaisesRegex(RuntimeError, "code=transport"):
            assert_clean_native_log("source", log)

    def test_native_assertion_failure_fails(self):
        with self.assertRaisesRegex(RuntimeError, "observer dropped"):
            assert_clean_native_log("source", "NATIVE_IPC_FAILURE observer dropped\n")

    def test_nonfailure_diagnostics_are_not_errors(self):
        assert_clean_native_log("source",
                                "NATIVE_IPC_DIAGNOSTIC pageshow persisted: explicitly reconnecting\n")


if __name__ == "__main__":
    unittest.main()
