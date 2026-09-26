# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Pure planning tests only: these do not exercise any native adapter."""
import unittest
from pathlib import Path, PureWindowsPath, PurePosixPath
from tempfile import TemporaryDirectory
from run import (NO_IPC_MODES, WINDOWS_APP_SDK_FILES, assert_clean_native_log,
                 assert_runtime_dependencies, copy_native_runner, platform_plan,
                 runner_build_command)


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

    def test_runners_use_root_workspace_lock_and_distinct_features(self):
        command = runner_build_command(source=False)
        self.assertEqual(
            command,
            ["cargo", "build", "--offline", "--locked", "--release",
             "-p", "microsoft-webui-desktop", "--example", "webui-native-ipc-fixture",
             "--no-default-features", "--features", "native,application-ipc"],
        )
        self.assertEqual(runner_build_command(source=True)[:-1], command[:-1])
        self.assertEqual(runner_build_command(source=True)[-1], "native,application-ipc,source")

    def test_windows_portable(self):
        plan = platform_plan("win32")
        self.assertEqual(plan["cli"], "webui-desktop.exe")
        self.assertEqual(plan["native_backend"], "WebView2")
        self.assertEqual(plan["package_target"], "windows-portable")
        self.assertEqual(PureWindowsPath("C:/package") / plan["resources"],
                         PureWindowsPath("C:/package/resources/webui"))
        self.assertEqual(PureWindowsPath("C:/package") / plan["executable_dir"] / plan["executable"],
                         PureWindowsPath("C:/package/webui-native-ipc-fixture.exe"))

    def test_windows_runner_copies_bootstrap_and_notices_without_mutating_inputs(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "release" / "fixture.exe"
            destination = root / "run" / "fixture.exe"
            source.parent.mkdir()
            destination.parent.mkdir()
            source.write_bytes(b"runner")
            for name in WINDOWS_APP_SDK_FILES:
                (source.parent / name).write_bytes(name.encode())
            copy_native_runner(source, destination, "win32")
            self.assertEqual(destination.read_bytes(), b"runner")
            for name in WINDOWS_APP_SDK_FILES:
                self.assertEqual((destination.parent / name).read_bytes(),
                                 (source.parent / name).read_bytes())

    def test_windows_runner_missing_companion_fails_before_copying_executable(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "release" / "fixture.exe"
            destination = root / "run" / "fixture.exe"
            source.parent.mkdir()
            destination.parent.mkdir()
            source.write_bytes(b"runner")
            for name in WINDOWS_APP_SDK_FILES[:-1]:
                (source.parent / name).write_bytes(name.encode())
            with self.assertRaisesRegex(RuntimeError, WINDOWS_APP_SDK_FILES[-1]):
                copy_native_runner(source, destination, "win32")
            self.assertFalse(destination.exists())

    def test_windows_example_uses_profile_companions_not_examples_directory(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            profile = root / "release"
            examples = profile / "examples"
            examples.mkdir(parents=True)
            source = examples / "no-ipc-native.exe"
            source.write_bytes(b"example")
            for name in WINDOWS_APP_SDK_FILES:
                (profile / name).write_bytes(name.encode())
            destination = root / "run" / "no-ipc-native.exe"
            destination.parent.mkdir()
            copy_native_runner(source, destination, "win32", companion_directory=profile)
            self.assertEqual(destination.read_bytes(), b"example")
            for name in WINDOWS_APP_SDK_FILES:
                self.assertEqual((destination.parent / name).read_bytes(), name.encode())

    def test_other_platforms_copy_only_the_runner(self):
        with TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "fixture"
            source.write_bytes(b"runner")
            for platform in ("darwin", "linux"):
                destination = root / platform / "fixture"
                destination.parent.mkdir()
                copy_native_runner(source, destination, platform)
                self.assertEqual(destination.read_bytes(), b"runner")
                self.assertEqual([path.name for path in destination.parent.iterdir()],
                                 ["fixture"])

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
