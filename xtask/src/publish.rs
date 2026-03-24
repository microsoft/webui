// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `cargo xtask publish-stage` — stage all release artifacts into `publish/`.
//!
//! Copies native binaries into npm and NuGet package directories (existing behavior),
//! then assembles a consolidated `publish/` folder with:
//! - `publish/native/`  — CLI binaries per platform
//! - `publish/npm/`     — `.tgz` tarballs from `pnpm pack`
//! - `publish/nuget/`   — `.nupkg` files from `dotnet pack`
//! - `publish/crates/`  — `.crate` files from `cargo package`
//! - `publish/wasm/`    — WASM module + JS glue

use crate::util::{build_command, run_command_quiet};
use crate::version;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ── Platform mapping ────────────────────────────────────────────────────

/// Mapping from Rust target triple to platform identifiers and binary filenames.
struct PlatformEntry {
    triple: &'static str,
    npm_package: &'static str,
    nuget_rid: &'static str,
    ffi_lib: &'static str,
    node_addon: &'static str,
    cli_binary: &'static str,
    /// Suffix appended to CLI binary in `publish/native/` (e.g. `"darwin-arm64"`).
    platform_suffix: &'static str,
}

const PLATFORMS: &[PlatformEntry] = &[
    PlatformEntry {
        triple: "x86_64-unknown-linux-gnu",
        npm_package: "webui-linux-x64",
        nuget_rid: "linux-x64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
        platform_suffix: "linux-x64",
    },
    PlatformEntry {
        triple: "aarch64-unknown-linux-gnu",
        npm_package: "webui-linux-arm64",
        nuget_rid: "linux-arm64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
        platform_suffix: "linux-arm64",
    },
    PlatformEntry {
        triple: "x86_64-pc-windows-msvc",
        npm_package: "webui-win32-x64",
        nuget_rid: "win-x64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-x64",
    },
    PlatformEntry {
        triple: "aarch64-pc-windows-msvc",
        npm_package: "webui-win32-arm64",
        nuget_rid: "win-arm64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-arm64",
    },
    PlatformEntry {
        triple: "x86_64-apple-darwin",
        npm_package: "webui-darwin-x64",
        nuget_rid: "osx-x64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-x64",
    },
    PlatformEntry {
        triple: "aarch64-apple-darwin",
        npm_package: "webui-darwin-arm64",
        nuget_rid: "osx-arm64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-arm64",
    },
];

/// Subdirectories created inside `publish/`.
const PUBLISH_SUBDIRS: &[&str] = &["native", "npm", "nuget", "crates", "wasm"];

// ── Public entry point ──────────────────────────────────────────────────

/// Stage release artifacts into `publish/` and package directories.
///
/// Usage: `cargo xtask publish-stage [--target <triple|all>] [--profile release]`
///
/// Pass `--target all` to stage every platform whose build artifacts exist.
/// If `--target` is omitted, detects the current host platform.
///
/// Steps:
///   1. Stage native binaries into npm/NuGet package directories (existing behavior).
///   2. Copy CLI binaries into `publish/native/` with platform suffixes.
///   3. Pack npm tarballs into `publish/npm/`.
///   4. Pack NuGet packages into `publish/nuget/`.
///   5. Pack publishable Rust crates into `publish/crates/`.
///   6. Build and stage WASM artifacts into `publish/wasm/`.
pub fn run_stage(args: &[String]) -> ExitCode {
    let root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "  {} Failed to read current directory: {e}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    let mut target_triple: Option<&str> = None;
    let mut profile = "release";
    let mut native_only = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                if i < args.len() {
                    target_triple = Some(args[i].as_str());
                }
            }
            "--profile" => {
                i += 1;
                if i < args.len() {
                    profile = args[i].as_str();
                }
            }
            "--native-only" => {
                native_only = true;
            }
            _ => {}
        }
        i += 1;
    }

    // Read workspace version
    let ver = match version::read_version() {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "  {} Failed to read version: {e}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    eprintln!(
        "\n{} publish-stage v{}\n",
        console::style("▸").cyan().bold(),
        console::style(&ver).bold(),
    );

    // Create publish/ directory tree
    if let Err(e) = create_publish_dirs(&root) {
        eprintln!(
            "  {} Failed to create publish/ directories: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 1: Stage native binaries (existing behavior + publish/native/)
    let stage_result = match target_triple {
        Some("all") => stage_all_platforms(&root, profile),
        Some(triple) => stage_one_platform(&root, triple, profile),
        None => {
            let host = detect_host_triple();
            eprintln!(
                "  {} No --target specified, using host: {}",
                console::style("▸").cyan().bold(),
                console::style(&host).bold(),
            );
            stage_one_platform(&root, &host, profile)
        }
    };

    if stage_result != ExitCode::SUCCESS {
        return stage_result;
    }

    // When --native-only is set, skip packaging phases (used by CI build runners
    // that don't have pnpm/dotnet/wasm toolchains installed).
    if native_only {
        eprintln!(
            "\n{} Native binaries staged (--native-only)\n",
            console::style("✔").green(),
        );
        return ExitCode::SUCCESS;
    }

    // Phase 2: Pack npm tarballs
    eprintln!(
        "\n{} Packing npm tarballs",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_npm_tarballs(&root) {
        eprintln!(
            "  {} npm pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 3: Pack NuGet packages
    eprintln!(
        "\n{} Packing NuGet packages",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_nuget_packages(&root) {
        eprintln!(
            "  {} NuGet pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 4: Pack Rust crates
    eprintln!(
        "\n{} Packing Rust crates",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_rust_crates(&root) {
        eprintln!(
            "  {} Rust crate pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 5: Stage WASM artifacts
    eprintln!(
        "\n{} Staging WASM artifacts",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = stage_wasm_artifacts(&root) {
        eprintln!(
            "  {} WASM staging failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Summary
    eprintln!(
        "\n{} All artifacts staged in {}\n",
        console::style("✨").green(),
        console::style("publish/").bold(),
    );

    ExitCode::SUCCESS
}

// ── Publish directory setup ─────────────────────────────────────────────

/// Create the `publish/` directory tree, cleaning it first if it exists.
fn create_publish_dirs(root: &Path) -> Result<(), String> {
    let publish_dir = root.join("publish");

    if publish_dir.exists() {
        fs::remove_dir_all(&publish_dir).map_err(|e| format!("failed to clean publish/: {e}"))?;
    }

    for subdir in PUBLISH_SUBDIRS {
        fs::create_dir_all(publish_dir.join(subdir))
            .map_err(|e| format!("failed to create publish/{subdir}: {e}"))?;
    }

    Ok(())
}

// ── Phase 1: Native binary staging ──────────────────────────────────────

/// Stage all platforms whose build artifacts exist under target/.
fn stage_all_platforms(root: &Path, profile: &str) -> ExitCode {
    eprintln!(
        "{} Staging all available platforms ({})",
        console::style("▸").cyan().bold(),
        console::style(profile).dim(),
    );

    let host = detect_host_triple();
    let mut staged = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for platform in PLATFORMS {
        let build_dir = if platform.triple == host {
            resolve_build_dir(root, platform.triple, profile)
        } else {
            root.join("target").join(platform.triple).join(profile)
        };

        let has_ffi = build_dir.join(platform.ffi_lib).exists();
        let has_cli = build_dir.join(platform.cli_binary).exists();
        let has_addon = build_dir.join(platform.node_addon).exists();

        if !has_ffi && !has_cli && !has_addon {
            skipped += 1;
            continue;
        }

        eprintln!(
            "\n  {} {}",
            console::style("▸").cyan(),
            console::style(platform.triple).bold(),
        );

        if stage_platform(root, platform, &build_dir) {
            staged += 1;
        } else {
            failed += 1;
        }
    }

    eprintln!();
    if staged > 0 {
        eprintln!(
            "  {} Staged {} platform(s)",
            console::style("✔").green(),
            console::style(staged).bold(),
        );
    }
    if skipped > 0 {
        eprintln!(
            "  {} Skipped {} platform(s) (no build artifacts found)",
            console::style("·").dim(),
            skipped,
        );
    }
    if failed > 0 {
        eprintln!(
            "  {} Failed {} platform(s)",
            console::style("✘").red().bold(),
            failed,
        );
        return ExitCode::FAILURE;
    }
    if staged == 0 {
        eprintln!(
            "  {} No build artifacts found. Build first:\n    {}",
            console::style("⚠").yellow(),
            console::style("cargo build --release -p microsoft-webui-ffi -p microsoft-webui-node -p microsoft-webui-cli").dim(),
        );
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Stage a single platform by triple name.
fn stage_one_platform(root: &Path, triple: &str, profile: &str) -> ExitCode {
    let Some(platform) = PLATFORMS.iter().find(|p| p.triple == triple) else {
        eprintln!(
            "  {} Unknown target triple: {}",
            console::style("✘").red().bold(),
            triple,
        );
        eprintln!("  Supported targets (or use 'all'):");
        for p in PLATFORMS {
            eprintln!(
                "    {} → npm: {}, nuget: {}",
                p.triple, p.npm_package, p.nuget_rid
            );
        }
        return ExitCode::FAILURE;
    };

    let build_dir = resolve_build_dir(root, triple, profile);

    eprintln!(
        "{} Staging native binaries for {} ({})",
        console::style("▸").cyan().bold(),
        console::style(triple).bold(),
        console::style(profile).dim(),
    );

    if stage_platform(root, platform, &build_dir) {
        eprintln!(
            "\n  {} All binaries staged for {}",
            console::style("✔").green(),
            console::style(platform.triple).bold(),
        );
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n  {} Some binaries could not be staged (see errors above)",
            console::style("⚠").yellow(),
        );
        ExitCode::FAILURE
    }
}

/// Copy all artifacts for a single platform. Returns true if all found files staged.
fn stage_platform(root: &Path, platform: &PlatformEntry, build_dir: &Path) -> bool {
    let mut ok = true;

    // NuGet: FFI library → dotnet/runtimes/{rid}/native/
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.ffi_lib),
        dest_dir: &root
            .join("dotnet/runtimes")
            .join(platform.nuget_rid)
            .join("native"),
        dest_name: platform.ffi_lib,
        label: "nuget",
    });

    // npm: CLI binary → packages/webui-{platform}/bin/
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.cli_binary),
        dest_dir: &root.join("packages").join(platform.npm_package).join("bin"),
        dest_name: platform.cli_binary,
        label: "npm cli",
    });

    // npm: Node addon (renamed to webui.node)
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.node_addon),
        dest_dir: &root.join("packages").join(platform.npm_package),
        dest_name: "webui.node",
        label: "npm addon",
    });

    // publish/native/: CLI binary with platform suffix for direct download
    let native_name = native_binary_name(platform);
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.cli_binary),
        dest_dir: &root.join("publish").join("native"),
        dest_name: &native_name,
        label: "native",
    });

    ok
}

/// Build a platform-suffixed CLI binary name (e.g. `webui-darwin-arm64`, `webui-win32-x64.exe`).
fn native_binary_name(platform: &PlatformEntry) -> String {
    if platform.cli_binary.ends_with(".exe") {
        format!("webui-{}.exe", platform.platform_suffix)
    } else {
        format!("webui-{}", platform.platform_suffix)
    }
}

// ── Phase 2: npm packaging ──────────────────────────────────────────────

/// Run `pnpm pack` in each `packages/*` directory and move tarballs to `publish/npm/`.
fn pack_npm_tarballs(root: &Path) -> Result<(), String> {
    let packages_dir = root.join("packages");
    let npm_out = root.join("publish").join("npm");

    // Build packages that have build scripts first
    for pkg_name in &["webui", "webui-router"] {
        let pkg_dir = packages_dir.join(pkg_name);
        if !pkg_dir.join("package.json").exists() {
            continue;
        }
        eprintln!(
            "  {} Building {}",
            console::style("·").dim(),
            console::style(pkg_name).bold(),
        );
        run_command_quiet(
            "pnpm",
            &["--filter", &format!("@microsoft/{pkg_name}"), "build"],
            None,
        )
        .map_err(|e| format!("pnpm build @microsoft/{pkg_name} failed: {e}"))?;
    }

    // Pack each package
    let entries =
        fs::read_dir(&packages_dir).map_err(|e| format!("failed to read packages/: {e}"))?;

    let mut count = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("package.json").exists() {
            continue;
        }

        let pkg_name = entry.file_name().to_string_lossy().to_string();

        // Run pnpm pack in the package directory
        let mut cmd = build_command(
            "pnpm",
            &["pack", "--pack-destination", &npm_out.to_string_lossy()],
        );
        cmd.current_dir(&path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = cmd
            .output()
            .map_err(|e| format!("pnpm pack failed for {pkg_name}: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!(
                "pnpm pack failed for {pkg_name}:\n{stdout}{stderr}"
            ));
        }

        eprintln!(
            "  {} [npm] @microsoft/{}",
            console::style("✔").green(),
            console::style(&pkg_name).bold(),
        );
        count += 1;
    }

    eprintln!(
        "  {} Packed {} npm package(s)",
        console::style("✔").green(),
        console::style(count).bold(),
    );
    Ok(())
}

// ── Phase 3: NuGet packaging ────────────────────────────────────────────

/// Run `dotnet pack` and move `.nupkg` files to `publish/nuget/`.
fn pack_nuget_packages(root: &Path) -> Result<(), String> {
    let dotnet_dir = root.join("dotnet");
    let nuget_out = root.join("publish").join("nuget");

    if !dotnet_dir.exists() {
        eprintln!(
            "  {} dotnet/ directory not found, skipping NuGet packaging",
            console::style("·").dim(),
        );
        return Ok(());
    }

    // Pack all packable projects (Directory.Build.props controls versioning)
    run_command_quiet(
        "dotnet",
        &[
            "pack",
            &dotnet_dir.to_string_lossy(),
            "--configuration",
            "Release",
            "--output",
            &nuget_out.to_string_lossy(),
        ],
        None,
    )
    .map_err(|e| format!("dotnet pack failed: {e}"))?;

    // Count produced packages
    let count = count_files_with_extension(&nuget_out, "nupkg");
    eprintln!(
        "  {} Packed {} NuGet package(s)",
        console::style("✔").green(),
        console::style(count).bold(),
    );
    Ok(())
}

// ── Phase 4: Rust crate packaging ───────────────────────────────────────

/// Discover publishable crates by scanning `crates/*/Cargo.toml`.
///
/// A crate is publishable if it has a `[package]` section with a `name` field
/// and does not contain `publish = false`.
fn discover_publishable_crates(root: &Path) -> Result<Vec<String>, String> {
    let crates_dir = root.join("crates");
    if !crates_dir.exists() {
        return Err("crates/ directory not found".to_string());
    }

    let mut entries: Vec<_> = fs::read_dir(&crates_dir)
        .map_err(|e| format!("failed to read crates/: {e}"))?
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut crates = Vec::new();
    for entry in entries {
        let toml_path = entry.path().join("Cargo.toml");
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| format!("failed to read {}: {e}", toml_path.display()))?;

        // Skip crates with publish = false
        if content.lines().any(|l| {
            let t = l.trim();
            t == "publish = false"
        }) {
            continue;
        }

        // Extract name from [package] section
        let mut in_package = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[package]" {
                in_package = true;
            } else if trimmed.starts_with('[') {
                in_package = false;
            }
            if in_package && trimmed.starts_with("name") && trimmed.contains('=') {
                if let Some(start) = trimmed.find('"') {
                    if let Some(end) = trimmed[start + 1..].find('"') {
                        crates.push(trimmed[start + 1..start + 1 + end].to_string());
                        break;
                    }
                }
            }
        }
    }

    if crates.is_empty() {
        return Err("no publishable crates found in crates/".to_string());
    }

    Ok(crates)
}

/// Package all workspace crates together and copy `.crate` files to `publish/crates/`.
///
/// Discovers publishable crates dynamically and uses a single `cargo package`
/// invocation so that inter-crate path dependencies resolve against each other
/// without requiring crates.io.
fn pack_rust_crates(root: &Path) -> Result<(), String> {
    let crates_out = root.join("publish").join("crates");

    let publishable = discover_publishable_crates(root)?;

    // Build args: cargo package -p A -p B -p C ... --no-verify --allow-dirty
    let mut args: Vec<&str> = vec!["package"];
    for crate_name in &publishable {
        args.push("-p");
        args.push(crate_name);
    }
    args.push("--no-verify");
    args.push("--allow-dirty");

    run_command_quiet("cargo", &args, None).map_err(|e| format!("cargo package failed: {e}"))?;

    for crate_name in &publishable {
        eprintln!(
            "  {} [crate] {}",
            console::style("✔").green(),
            console::style(crate_name).bold(),
        );
    }

    // Copy .crate files from target/package/ to publish/crates/
    let package_dir = root.join("target").join("package");
    if package_dir.exists() {
        copy_files_with_extension(&package_dir, &crates_out, "crate")?;
    }

    let count = count_files_with_extension(&crates_out, "crate");
    eprintln!(
        "  {} Packed {} Rust crate(s)",
        console::style("✔").green(),
        console::style(count).bold(),
    );
    Ok(())
}

// ── Phase 5: WASM artifacts ─────────────────────────────────────────────

/// Build WASM and copy artifacts to `publish/wasm/`.
fn stage_wasm_artifacts(root: &Path) -> Result<(), String> {
    let wasm_out = root.join("publish").join("wasm");

    // Build WASM using the existing build_wasm module
    crate::build_wasm::run()?;

    // Copy the built WASM files to publish/wasm/
    let wasm_source = root.join("docs").join("public").join("wasm");
    for filename in &["webui_wasm_bg.wasm", "webui_wasm.js"] {
        let src = wasm_source.join(filename);
        if src.exists() {
            let dest = wasm_out.join(filename);
            fs::copy(&src, &dest)
                .map_err(|e| format!("failed to copy {} to publish/wasm/: {e}", filename))?;
            eprintln!(
                "  {} [wasm] {}",
                console::style("✔").green(),
                console::style(filename).bold(),
            );
        } else {
            eprintln!(
                "  {} [wasm] {} not found at {}",
                console::style("⚠").yellow(),
                filename,
                wasm_source.display(),
            );
        }
    }

    Ok(())
}

// ── Shared helpers ──────────────────────────────────────────────────────

struct CopySpec<'a> {
    src: &'a Path,
    dest_dir: &'a Path,
    dest_name: &'a str,
    label: &'a str,
}

fn stage_file(spec: &CopySpec<'_>) -> bool {
    if !spec.src.exists() {
        eprintln!(
            "  {} [{}] not found: {}",
            console::style("⚠").yellow(),
            spec.label,
            console::style(spec.src.display()).dim(),
        );
        return false;
    }

    if let Err(e) = fs::create_dir_all(spec.dest_dir) {
        eprintln!(
            "  {} [{}] failed to create {}: {}",
            console::style("✘").red().bold(),
            spec.label,
            spec.dest_dir.display(),
            e,
        );
        return false;
    }

    let dest = spec.dest_dir.join(spec.dest_name);
    if let Err(e) = fs::copy(spec.src, &dest) {
        eprintln!(
            "  {} [{}] copy failed: {} → {}: {}",
            console::style("✘").red().bold(),
            spec.label,
            spec.src.display(),
            dest.display(),
            e,
        );
        return false;
    }

    let rel = dest
        .strip_prefix(std::env::current_dir().as_deref().unwrap_or(Path::new("")))
        .unwrap_or(&dest);
    eprintln!(
        "  {} [{}] {}",
        console::style("✔").green(),
        spec.label,
        console::style(rel.display()).bold(),
    );
    true
}

fn resolve_build_dir(root: &Path, triple: &str, profile: &str) -> PathBuf {
    let cross = root.join("target").join(triple).join(profile);
    if cross.exists() {
        return cross;
    }
    root.join("target").join(profile)
}

fn detect_host_triple() -> String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };

    let os = if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else {
        "unknown"
    };

    format!("{arch}-{os}")
}

/// Count files in a directory matching a given extension.
fn count_files_with_extension(dir: &Path, ext: &str) -> u32 {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == ext) {
                count += 1;
            }
        }
    }
    count
}

/// Copy all files with a given extension from `src_dir` to `dest_dir`.
fn copy_files_with_extension(src_dir: &Path, dest_dir: &Path, ext: &str) -> Result<(), String> {
    let entries =
        fs::read_dir(src_dir).map_err(|e| format!("failed to read {}: {e}", src_dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == ext) {
            if let Some(name) = path.file_name() {
                let dest = dest_dir.join(name);
                fs::copy(&path, &dest).map_err(|e| {
                    format!(
                        "failed to copy {} → {}: {e}",
                        path.display(),
                        dest.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_binary_name_unix() {
        let p = PlatformEntry {
            triple: "aarch64-apple-darwin",
            npm_package: "webui-darwin-arm64",
            nuget_rid: "osx-arm64",
            ffi_lib: "libwebui_ffi.dylib",
            node_addon: "libwebui_node.dylib",
            cli_binary: "webui",
            platform_suffix: "darwin-arm64",
        };
        assert_eq!(native_binary_name(&p), "webui-darwin-arm64");
    }

    #[test]
    fn test_native_binary_name_windows() {
        let p = PlatformEntry {
            triple: "x86_64-pc-windows-msvc",
            npm_package: "webui-win32-x64",
            nuget_rid: "win-x64",
            ffi_lib: "webui_ffi.dll",
            node_addon: "webui_node.dll",
            cli_binary: "webui.exe",
            platform_suffix: "win32-x64",
        };
        assert_eq!(native_binary_name(&p), "webui-win32-x64.exe");
    }

    #[test]
    fn test_create_publish_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        create_publish_dirs(dir.path()).unwrap();

        for subdir in PUBLISH_SUBDIRS {
            assert!(
                dir.path().join("publish").join(subdir).is_dir(),
                "publish/{subdir} should exist"
            );
        }
    }

    #[test]
    fn test_create_publish_dirs_cleans_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let publish = dir.path().join("publish");
        fs::create_dir_all(publish.join("stale")).unwrap();
        fs::write(publish.join("stale").join("old.txt"), "old").unwrap();

        create_publish_dirs(dir.path()).unwrap();

        assert!(!publish.join("stale").exists(), "stale/ should be removed");
        for subdir in PUBLISH_SUBDIRS {
            assert!(publish.join(subdir).is_dir());
        }
    }

    #[test]
    fn test_count_files_with_extension() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.crate"), "").unwrap();
        fs::write(dir.path().join("b.crate"), "").unwrap();
        fs::write(dir.path().join("c.txt"), "").unwrap();
        assert_eq!(count_files_with_extension(dir.path(), "crate"), 2);
        assert_eq!(count_files_with_extension(dir.path(), "txt"), 1);
        assert_eq!(count_files_with_extension(dir.path(), "nupkg"), 0);
    }

    #[test]
    fn test_copy_files_with_extension() {
        let src = tempfile::TempDir::new().unwrap();
        let dest = tempfile::TempDir::new().unwrap();
        fs::write(src.path().join("pkg.crate"), "data").unwrap();
        fs::write(src.path().join("other.txt"), "nope").unwrap();

        copy_files_with_extension(src.path(), dest.path(), "crate").unwrap();

        assert!(dest.path().join("pkg.crate").exists());
        assert!(!dest.path().join("other.txt").exists());
    }

    #[test]
    fn test_detect_host_triple_format() {
        let triple = detect_host_triple();
        assert!(
            triple.contains('-'),
            "host triple should contain a dash: {triple}"
        );
    }
}
