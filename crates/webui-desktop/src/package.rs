// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::bundle::{
    copy_file_to, lexical_absolute_path, normalized_absolute_path, reject_path_overlap,
    DesktopBundleManifest, DesktopPackageTarget,
};
use crate::error::{DesktopError, Result};

/// Options for packaging a desktop bundle.
pub struct DesktopPackageOptions {
    /// Desktop bundle directory created by `webui desktop build`.
    pub bundle_dir: PathBuf,
    /// Output directory for package artifacts.
    pub out_dir: PathBuf,
    /// Package target.
    pub target: DesktopPackageTarget,
    /// Desktop runner executable to include in portable layouts.
    pub runner_exe: PathBuf,
}

/// Result of a package operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopPackageResult {
    /// Package output path.
    pub output_path: PathBuf,
}

/// Package a desktop bundle for a native target.
///
/// # Errors
///
/// Returns [`DesktopError`] if the manifest cannot be read, package files
/// cannot be written, or the target requires external platform tooling.
pub fn package_desktop_bundle(options: DesktopPackageOptions) -> Result<DesktopPackageResult> {
    let manifest_path = options.bundle_dir.join("manifest.webui-desktop.json");
    let manifest = DesktopBundleManifest::load(&manifest_path)?;

    match options.target {
        DesktopPackageTarget::MacosApp => package_macos_app(&options, &manifest),
        DesktopPackageTarget::WindowsPortable => {
            package_portable(&options, &manifest, "windows-portable")
        }
        DesktopPackageTarget::LinuxPortable => package_portable(&options, &manifest, "linux-portable"),
        DesktopPackageTarget::WindowsMsi => requires_tooling(
            DesktopPackageTarget::WindowsMsi,
            "WiX 3.11 and signtool.exe",
            "run this target on a Windows runner with WiX 3.11 and a configured code-signing certificate",
        ),
        DesktopPackageTarget::WindowsMsix => requires_tooling(
            DesktopPackageTarget::WindowsMsix,
            "Windows SDK makeappx.exe and signtool.exe",
            "run this target on a Windows runner with Windows SDK packaging tools and a certificate matching the MSIX publisher",
        ),
        DesktopPackageTarget::LinuxAppImage => requires_tooling(
            DesktopPackageTarget::LinuxAppImage,
            "appimagetool",
            "run this target on a Linux runner with appimagetool and the target architecture runtime available",
        ),
        DesktopPackageTarget::LinuxDeb => requires_tooling(
            DesktopPackageTarget::LinuxDeb,
            "Debian package writer",
            "enable the cargo-packager-backed deb implementation on a Linux runner",
        ),
        DesktopPackageTarget::LinuxRpm => requires_tooling(
            DesktopPackageTarget::LinuxRpm,
            "RPM package writer",
            "enable the rpm crate-backed implementation on a Linux runner with package metadata configured",
        ),
    }
}

fn package_portable(
    options: &DesktopPackageOptions,
    manifest: &DesktopBundleManifest,
    suffix: &str,
) -> Result<DesktopPackageResult> {
    let safe_name = safe_package_name(&manifest.app_name);
    let output_path = options.out_dir.join(format!("{safe_name}-{suffix}"));
    validate_package_output(&output_path, options)?;
    prepare_output_dir(&output_path)?;

    copy_runner(&options.runner_exe, &output_path)?;
    copy_bundle_dir(
        &options.bundle_dir,
        &output_path.join("resources").join("webui"),
    )?;
    copy_portable_icon(options, manifest, &output_path.join("resources"))?;

    Ok(DesktopPackageResult { output_path })
}

fn package_macos_app(
    options: &DesktopPackageOptions,
    manifest: &DesktopBundleManifest,
) -> Result<DesktopPackageResult> {
    let safe_name = safe_package_name(&manifest.app_name);
    let output_path = options.out_dir.join(format!("{safe_name}.app"));
    validate_package_output(&output_path, options)?;
    prepare_output_dir(&output_path)?;

    let contents = output_path.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos).map_err(|source| DesktopError::Io {
        context: format!("creating macOS app MacOS directory {}", macos.display()),
        source,
    })?;
    fs::create_dir_all(&resources).map_err(|source| DesktopError::Io {
        context: format!(
            "creating macOS app Resources directory {}",
            resources.display()
        ),
        source,
    })?;

    let executable_name = copy_runner(&options.runner_exe, &macos)?;
    copy_bundle_dir(&options.bundle_dir, &resources.join("webui"))?;
    let icon_file = copy_macos_icon(options, manifest, &resources)?;
    write_info_plist(
        &contents.join("Info.plist"),
        manifest,
        &executable_name,
        icon_file.as_deref(),
    )?;

    Ok(DesktopPackageResult { output_path })
}

fn validate_package_output(output_path: &Path, options: &DesktopPackageOptions) -> Result<()> {
    let output = normalized_absolute_path(output_path)?;
    let lexical_output = lexical_absolute_path(output_path)?;
    let bundle = normalized_absolute_path(&options.bundle_dir)?;
    let lexical_bundle = lexical_absolute_path(&options.bundle_dir)?;
    let runner = normalized_absolute_path(&options.runner_exe)?;
    let lexical_runner = lexical_absolute_path(&options.runner_exe)?;
    reject_path_overlap(&output, &bundle, "bundle")?;
    reject_path_overlap(&lexical_output, &bundle, "bundle")?;
    reject_path_overlap(&output, &lexical_bundle, "bundle")?;
    reject_path_overlap(&lexical_output, &lexical_bundle, "bundle")?;
    reject_path_overlap(&output, &runner, "runner")?;
    reject_path_overlap(&lexical_output, &runner, "runner")?;
    reject_path_overlap(&output, &lexical_runner, "runner")?;
    reject_path_overlap(&lexical_output, &lexical_runner, "runner")?;
    Ok(())
}

fn requires_tooling(
    target: DesktopPackageTarget,
    tooling: &str,
    help: &str,
) -> Result<DesktopPackageResult> {
    Err(DesktopError::PackageTargetRequiresTooling {
        target: target_name(target).to_string(),
        tooling: tooling.to_string(),
        help: help.to_string(),
    })
}

fn prepare_output_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| DesktopError::Io {
            context: format!("cleaning desktop package output {}", path.display()),
            source,
        })?;
    }
    fs::create_dir_all(path).map_err(|source| DesktopError::Io {
        context: format!("creating desktop package output {}", path.display()),
        source,
    })
}

fn copy_runner(runner_exe: &Path, dest_dir: &Path) -> Result<String> {
    let name = runner_exe
        .file_name()
        .ok_or_else(|| DesktopError::InvalidAssetPath {
            path: runner_exe.display().to_string(),
        })?;
    copy_file_to(runner_exe, &dest_dir.join(name))?;
    Ok(name.to_string_lossy().into_owned())
}

fn copy_bundle_dir(source: &Path, dest: &Path) -> Result<()> {
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let relative = dir
            .strip_prefix(source)
            .map_err(|_| DesktopError::InvalidAssetPath {
                path: dir.display().to_string(),
            })?;
        let dest_dir = dest.join(relative);
        fs::create_dir_all(&dest_dir).map_err(|source| DesktopError::Io {
            context: format!("creating desktop package resource {}", dest_dir.display()),
            source,
        })?;

        for entry in fs::read_dir(&dir).map_err(|source| DesktopError::Io {
            context: format!("reading desktop bundle directory {}", dir.display()),
            source,
        })? {
            let entry = entry.map_err(|source| DesktopError::Io {
                context: format!("reading desktop bundle entry {}", dir.display()),
                source,
            })?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|source| DesktopError::Io {
                context: format!("reading desktop bundle entry type {}", path.display()),
                source,
            })?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let relative_file =
                    path.strip_prefix(source)
                        .map_err(|_| DesktopError::InvalidAssetPath {
                            path: path.display().to_string(),
                        })?;
                copy_file_to(&path, &dest.join(relative_file))?;
            }
        }
    }
    Ok(())
}

fn copy_portable_icon(
    options: &DesktopPackageOptions,
    manifest: &DesktopBundleManifest,
    resources: &Path,
) -> Result<()> {
    let Some(icon_path) = manifest.shell.icon_path.as_ref() else {
        return Ok(());
    };
    let source = resolve_bundle_relative_file(&options.bundle_dir, icon_path, "icon")?;
    let Some(file_name) = source.file_name() else {
        return Ok(());
    };
    copy_file_to(&source, &resources.join(file_name))
}

fn copy_macos_icon(
    options: &DesktopPackageOptions,
    manifest: &DesktopBundleManifest,
    resources: &Path,
) -> Result<Option<String>> {
    let Some(icon_path) = manifest.shell.icon_path.as_ref() else {
        return Ok(None);
    };
    let source = resolve_bundle_relative_file(&options.bundle_dir, icon_path, "icon")?;
    let Some(extension) = source.extension().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    if extension != "icns" {
        return Ok(None);
    }
    let icon_name = "AppIcon.icns";
    copy_file_to(&source, &resources.join(icon_name))?;
    Ok(Some(icon_name.to_string()))
}

fn resolve_bundle_relative_file(bundle_dir: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    validate_bundle_relative_path(path, label)?;
    let bundle_root = bundle_dir
        .canonicalize()
        .map_err(|source| DesktopError::Io {
            context: format!("resolving desktop bundle root {}", bundle_dir.display()),
            source,
        })?;
    let joined = bundle_root.join(path);
    let canonical = joined.canonicalize().map_err(|source| DesktopError::Io {
        context: format!("resolving desktop bundle {label} {}", joined.display()),
        source,
    })?;
    if !canonical.starts_with(&bundle_root) {
        return Err(DesktopError::InvalidAssetPath {
            path: format!("{label}: {}", path.display()),
        });
    }
    Ok(canonical)
}

fn validate_bundle_relative_path(path: &Path, label: &str) -> Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(segment)
                if segment
                    .to_str()
                    .is_some_and(|value| !value.contains('\\') && !value.contains('\0')) => {}
            _ => {
                return Err(DesktopError::InvalidAssetPath {
                    path: format!("{label}: {}", path.display()),
                });
            }
        }
    }
    Ok(())
}

fn write_info_plist(
    path: &Path,
    manifest: &DesktopBundleManifest,
    executable_name: &str,
    icon_file: Option<&str>,
) -> Result<()> {
    let icon_block = icon_file.map_or_else(String::new, |icon| {
        let mut block = String::with_capacity(64 + icon.len());
        block.push_str("  <key>CFBundleIconFile</key>\n  <string>");
        block.push_str(&xml_escape(icon));
        block.push_str("</string>\n");
        block
    });
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>{}</string>
  <key>CFBundleIdentifier</key>
  <string>{}</string>
  <key>CFBundleName</key>
  <string>{}</string>
{}  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{}</string>
  <key>CFBundleVersion</key>
  <string>{}</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#,
        xml_escape(executable_name),
        xml_escape(&manifest.app_id),
        xml_escape(&manifest.app_name),
        icon_block,
        xml_escape(&manifest.version),
        xml_escape(&manifest.version)
    );
    fs::write(path, plist).map_err(|source| DesktopError::Io {
        context: format!("writing macOS Info.plist {}", path.display()),
        source,
    })
}

fn safe_package_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    if out.is_empty() {
        "webui-app".to_string()
    } else {
        out
    }
}

fn target_name(target: DesktopPackageTarget) -> &'static str {
    match target {
        DesktopPackageTarget::MacosApp => "macos-app",
        DesktopPackageTarget::WindowsPortable => "windows-portable",
        DesktopPackageTarget::WindowsMsi => "windows-msi",
        DesktopPackageTarget::WindowsMsix => "windows-msix",
        DesktopPackageTarget::LinuxPortable => "linux-portable",
        DesktopPackageTarget::LinuxAppImage => "linux-appimage",
        DesktopPackageTarget::LinuxDeb => "linux-deb",
        DesktopPackageTarget::LinuxRpm => "linux-rpm",
    }
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::bundle::{build_desktop_bundle, DesktopBundleOptions, WindowOptions};
    use tempfile::TempDir;

    fn write_file(root: &Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn create_bundle(root: &Path) -> PathBuf {
        let app = root.join("app");
        let bundle = root.join("bundle");
        write_file(&app, "index.html", "<main>package</main>");
        write_file(&app, "public/app.js", "console.log('package');");
        build_desktop_bundle(DesktopBundleOptions {
            build_options: webui::BuildOptions {
                app_dir: app.clone(),
                entry: "index.html".to_string(),
                ..webui::BuildOptions::default()
            },
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: Some(app.join("public")),
            token_css: None,
            app_id: "com.microsoft.webui.package".to_string(),
            app_name: "Package Test".to_string(),
            version: "1.2.3".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: crate::bundle::DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();
        bundle
    }

    #[test]
    fn creates_portable_package_layout() {
        let dir = TempDir::new().unwrap();
        let bundle = create_bundle(dir.path());
        let runner = dir.path().join("webui-desktop");
        write_file(dir.path(), "webui-desktop", "runner");

        let result = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: dir.path().join("out"),
            target: DesktopPackageTarget::LinuxPortable,
            runner_exe: runner,
        })
        .unwrap();

        assert!(result.output_path.join("webui-desktop").is_file());
        assert!(result
            .output_path
            .join("resources/webui/manifest.webui-desktop.json")
            .is_file());
        assert!(result
            .output_path
            .join("resources/webui/assets/app.js")
            .is_file());
    }

    #[test]
    fn creates_macos_app_layout() {
        let dir = TempDir::new().unwrap();
        let bundle = create_bundle(dir.path());
        let runner = dir.path().join("contact-book-desktop");
        write_file(dir.path(), "contact-book-desktop", "runner");

        let result = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: dir.path().join("out"),
            target: DesktopPackageTarget::MacosApp,
            runner_exe: runner,
        })
        .unwrap();

        assert!(result
            .output_path
            .join("Contents/MacOS/contact-book-desktop")
            .is_file());
        assert!(result
            .output_path
            .join("Contents/Resources/webui/protocol.bin")
            .is_file());
        let info = fs::read_to_string(result.output_path.join("Contents/Info.plist")).unwrap();
        assert!(info.contains("<string>contact-book-desktop</string>"));
    }

    #[test]
    fn creates_macos_app_icon_layout() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        let bundle = dir.path().join("bundle");
        let runner = dir.path().join("webui-desktop");
        write_file(&app, "index.html", "<main>package</main>");
        write_file(&app, "app.icns", "icon");
        write_file(dir.path(), "webui-desktop", "runner");

        build_desktop_bundle(DesktopBundleOptions {
            build_options: webui::BuildOptions {
                app_dir: app.clone(),
                entry: "index.html".to_string(),
                ..webui::BuildOptions::default()
            },
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.package".to_string(),
            app_name: "Package Test".to_string(),
            version: "1.2.3".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: Some(app.join("app.icns")),
            shell: crate::DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let result = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: dir.path().join("out"),
            target: DesktopPackageTarget::MacosApp,
            runner_exe: runner,
        })
        .unwrap();

        assert!(result
            .output_path
            .join("Contents/Resources/AppIcon.icns")
            .is_file());
        let info = fs::read_to_string(result.output_path.join("Contents/Info.plist")).unwrap();
        assert!(info.contains("<key>CFBundleIconFile</key>"));
        assert!(info.contains("<string>AppIcon.icns</string>"));
    }

    #[test]
    fn rejects_manifest_icon_path_outside_bundle() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        let bundle = dir.path().join("bundle");
        let runner = dir.path().join("webui-desktop");
        write_file(&app, "index.html", "<main>package</main>");
        write_file(dir.path(), "webui-desktop", "runner");

        build_desktop_bundle(DesktopBundleOptions {
            build_options: webui::BuildOptions {
                app_dir: app,
                entry: "index.html".to_string(),
                ..webui::BuildOptions::default()
            },
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.package".to_string(),
            app_name: "Package Test".to_string(),
            version: "1.2.3".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: crate::DesktopShellConfig {
                icon_path: Some(PathBuf::from("../outside.icns")),
                ..crate::DesktopShellConfig::default()
            },
            package_targets: Vec::new(),
        })
        .unwrap();

        let err = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: dir.path().join("out"),
            target: DesktopPackageTarget::MacosApp,
            runner_exe: runner,
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::InvalidAssetPath { .. }));
    }

    #[test]
    fn installer_targets_return_actionable_tooling_error() {
        let dir = TempDir::new().unwrap();
        let bundle = create_bundle(dir.path());
        let runner = dir.path().join("webui-desktop");
        write_file(dir.path(), "webui-desktop", "runner");

        let err = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: dir.path().join("out"),
            target: DesktopPackageTarget::WindowsMsix,
            runner_exe: runner,
        })
        .unwrap_err();

        match err {
            DesktopError::PackageTargetRequiresTooling { help, .. } => {
                assert!(help.contains("Windows SDK"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn rejects_package_output_inside_bundle_directory() {
        let dir = TempDir::new().unwrap();
        let bundle = create_bundle(dir.path());
        let runner = dir.path().join("webui-desktop");
        write_file(dir.path(), "webui-desktop", "runner");

        let err = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle.clone(),
            out_dir: bundle,
            target: DesktopPackageTarget::LinuxPortable,
            runner_exe: runner,
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
    }

    #[test]
    fn rejected_package_output_does_not_create_directory_inside_bundle() {
        let dir = TempDir::new().unwrap();
        let bundle = create_bundle(dir.path());
        let runner = dir.path().join("webui-desktop");
        let rejected_out = bundle.join("packages");
        write_file(dir.path(), "webui-desktop", "runner");

        let err = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle,
            out_dir: rejected_out.clone(),
            target: DesktopPackageTarget::LinuxPortable,
            runner_exe: runner,
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
        assert!(!rejected_out.exists());
    }
}
