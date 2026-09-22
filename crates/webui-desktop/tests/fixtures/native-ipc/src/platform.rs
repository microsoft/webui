// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};
use webui_desktop::{
    package_desktop_bundle, DesktopFrameBackend, DesktopPackageOptions, DesktopPackageTarget,
    DesktopPlatform, PlatformFrameBackend,
};

pub fn metadata() -> (&'static str, &'static str, DesktopPackageTarget) {
    match DesktopPlatform::current() {
        DesktopPlatform::Macos => ("darwin", "WKWebView", DesktopPackageTarget::MacosApp),
        DesktopPlatform::Windows => ("win32", "WebView2", DesktopPackageTarget::WindowsPortable),
        DesktopPlatform::Linux => ("linux", "WebKitGTK", DesktopPackageTarget::LinuxPortable),
    }
}

pub fn print_metadata() {
    let (platform, backend, target) = metadata();
    let capabilities = PlatformFrameBackend.capabilities();
    println!(
        "NATIVE_METADATA {}",
        webui_test_utils::test_json!({
            "platform": platform, "native_backend": backend, "package_target": target,
            "application_ipc": capabilities.application_ipc,
            "window_controls": capabilities.window_controls,
            "events": capabilities.events,
        })
    );
}

pub fn package(bundle: &Path, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let runner = std::env::current_exe()?;
    let (_, _, target) = metadata();
    let package = package_desktop_bundle(DesktopPackageOptions {
        bundle_dir: bundle.to_path_buf(),
        out_dir: output,
        target,
        runner_exe: runner.clone(),
    })?;
    let (executable_dir, resources) = match DesktopPlatform::current() {
        DesktopPlatform::Macos => ("Contents/MacOS", "Contents/Resources/webui"),
        DesktopPlatform::Windows | DesktopPlatform::Linux => ("", "resources/webui"),
    };
    let name = runner.file_name().ok_or("runner name missing")?;
    println!(
        "NATIVE_PACKAGE {}",
        webui_test_utils::test_json!({
            "root": package.output_path,
            "binary": package.output_path.join(executable_dir).join(name),
            "resources": package.output_path.join(resources),
            "target": target,
        })
    );
    Ok(())
}
